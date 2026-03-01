use std::collections::HashMap;

use alloy::primitives::{Address, U256};
use alloy::providers::ProviderBuilder;
use alloy::sol;
use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::model::chain::Chain;
use crate::model::node::{Node, PendleAction};
use crate::run::config::RuntimeConfig;
use crate::venues::evm;
use crate::venues::{ExecutionResult, Venue};

// ── Pendle Router interface ────────────────────────────────────────

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IPendleRouter {
        function mintPyFromSy(
            address receiver,
            address YT,
            uint256 netSyIn,
            uint256 minPyOut
        ) external returns (uint256 netPyOut);

        function redeemPyToSy(
            address receiver,
            address YT,
            uint256 netPyIn,
            uint256 minSyOut
        ) external returns (uint256 netSyOut);

        function redeemDueInterestAndRewards(
            address user,
            address[] calldata sys,
            address[] calldata yts,
            address[] calldata markets
        ) external;
    }
}

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IERC20 {
        function approve(address spender, uint256 amount) external returns (bool);
        function balanceOf(address account) external view returns (uint256);
    }
}

// ── SY contract interface (deposit input token → SY shares) ───────

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IPendleSY {
        function deposit(
            address receiver,
            address tokenIn,
            uint256 amountTokenToDeposit,
            uint256 minSharesOut
        ) external payable returns (uint256 amountSharesOut);
    }
}

// ── Contract key derivation ──────────────────────────────────────

/// Derive a contracts manifest key from a Pendle market name.
/// E.g. `pendle_contract_key("PT-kHYPE", "market")` → `"pendle_pt_khype_market"`.
pub fn pendle_contract_key(market: &str, suffix: &str) -> String {
    let normalized = market.to_lowercase().replace('-', "_");
    format!("pendle_{}_{}", normalized, suffix)
}

/// Determine the chain for a Pendle market by inspecting the contracts manifest.
fn pendle_chain(contracts: &evm::ContractManifest, market: &str) -> Option<Chain> {
    let market_key = pendle_contract_key(market, "market");
    let chains = contracts.get(&market_key)?;
    let (chain_name, _) = chains.iter().next()?;
    Some(Chain::from_name(chain_name))
}

// ── Pendle Yield ──────────────────────────────────────────────────

pub struct PendleYield {
    wallet_address: Address,
    private_key: String,
    dry_run: bool,
    tokens: evm::TokenManifest,
    contracts: evm::ContractManifest,
    pt_holdings: HashMap<String, f64>,
    yt_holdings: HashMap<String, f64>,
    /// Pre-populated from node for on-chain queries and token resolution.
    market_name: Option<String>,
    /// Input token symbol from node (e.g. "HYPE", "kHYPE").
    input_token: Option<String>,
    /// Cached alpha stats from on-chain value tracking.
    cached_alpha: Option<(f64, f64)>,
    /// Previous value sample for APY computation — single point, not a Vec.
    prev_value: Option<(u64, f64)>,
}

impl PendleYield {
    pub fn new(
        config: &RuntimeConfig,
        tokens: &evm::TokenManifest,
        contracts: &evm::ContractManifest,
        node: &Node,
    ) -> Result<Self> {
        let (market_name, input_token) = if let Node::Pendle {
            market,
            input_token,
            ..
        } = node
        {
            (Some(market.clone()), input_token.clone())
        } else {
            (None, None)
        };

        Ok(PendleYield {
            wallet_address: config.wallet_address,
            private_key: config.private_key.clone(),
            dry_run: config.dry_run,
            tokens: tokens.clone(),
            contracts: contracts.clone(),
            pt_holdings: HashMap::new(),
            yt_holdings: HashMap::new(),
            market_name,
            input_token,
            cached_alpha: None,
            prev_value: None,
        })
    }

    /// Resolve the output token symbol for redeem/claim operations.
    fn output_token(&self) -> String {
        self.input_token.clone().unwrap_or_else(|| "USDC".to_string())
    }

    /// Query on-chain PT and YT token balances for a market.
    async fn query_onchain_value(&self, market_name: &str) -> Result<f64> {
        let chain = pendle_chain(&self.contracts, market_name)
            .context("Pendle market chain not found")?;
        let rpc_url = chain
            .rpc_url()
            .context("Pendle chain requires RPC URL")?;
        let rp = evm::read_provider(rpc_url)?;

        let mut total = 0.0;

        // Query PT token balance
        let pt_key = pendle_contract_key(market_name, "pt");
        if let Some(pt_addr) = evm::resolve_contract(&self.contracts, &pt_key, &chain) {
            let pt_token = IERC20::new(pt_addr, &rp);
            if let Ok(balance) = pt_token.balanceOf(self.wallet_address).call().await {
                // PT tokens are 18 decimals, approximately 1:1 with underlying at maturity
                total += evm::from_token_units(balance, 18);
            }
        }

        // Query YT token balance
        let yt_key = pendle_contract_key(market_name, "yt");
        if let Some(yt_addr) = evm::resolve_contract(&self.contracts, &yt_key, &chain) {
            let yt_token = IERC20::new(yt_addr, &rp);
            if let Ok(balance) = yt_token.balanceOf(self.wallet_address).call().await {
                total += evm::from_token_units(balance, 18);
            }
        }

        Ok(total)
    }

    /// Deposit input token into the SY contract, returning the SY balance.
    ///
    /// For native-token inputs (HYPE):
    ///   LiFi outputs native HYPE → deposit into SY via SY.deposit{value}(wallet, address(0), amount, 0)
    ///
    /// For ERC20 inputs (kHYPE, stETH, WHYPE, etc.):
    ///   Approve the SY contract → SY.deposit(wallet, tokenIn, amount, 0)
    async fn deposit_into_sy<P: alloy::providers::Provider + Clone>(
        &self,
        provider: &P,
        sy_addr: Address,
        chain: &Chain,
        amount_units: U256,
    ) -> Result<U256> {
        let input_tok = self.input_token.as_deref().unwrap_or("USDC");

        // Check if input token resolves to native (address(0)) or an ERC20
        let token_addr = evm::resolve_token(&self.tokens, chain, input_tok);
        let is_native = token_addr.map_or(false, |a| a == Address::ZERO);

        if is_native {
            // Native token (HYPE) — LiFi swap outputs native to wallet, deposit with msg.value
            let sy = IPendleSY::new(sy_addr, provider);
            println!("  PENDLE: depositing {:.4} native {} into SY",
                evm::from_token_units(amount_units, 18), input_tok);
            let receipt = sy.deposit(self.wallet_address, Address::ZERO, amount_units, U256::ZERO)
                .value(amount_units)
                .send().await.context("SY deposit (native)")?
                .get_receipt().await?;
            println!("  PENDLE: SY deposit tx: {:?}", receipt.transaction_hash);
        } else {
            // ERC20 input (kHYPE, WHYPE, stETH, etc.)
            let addr = token_addr
                .with_context(|| format!("Token '{}' not in manifest for chain {}", input_tok, chain))?;

            let erc20 = IERC20::new(addr, provider);
            let bal = erc20.balanceOf(self.wallet_address).call().await
                .context("input token balanceOf")?;
            let actual = amount_units.min(bal);
            if actual.is_zero() {
                anyhow::bail!("No {} balance for SY deposit", input_tok);
            }

            println!("  PENDLE: approving {:.4} {} for SY",
                evm::from_token_units(actual, 18), input_tok);
            erc20.approve(sy_addr, actual)
                .send().await.context("approve input for SY")?
                .get_receipt().await?;

            let sy = IPendleSY::new(sy_addr, provider);
            let receipt = sy.deposit(self.wallet_address, addr, actual, U256::ZERO)
                .send().await.context("SY deposit (ERC20)")?
                .get_receipt().await?;
            println!("  PENDLE: SY deposit tx: {:?}", receipt.transaction_hash);
        }

        // Return SY balance after deposit
        let sy_erc20 = IERC20::new(sy_addr, provider);
        let sy_bal = sy_erc20.balanceOf(self.wallet_address).call().await
            .context("SY balanceOf after deposit")?;
        Ok(sy_bal)
    }

    async fn execute_mint_pt(
        &mut self,
        market_name: &str,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        let market_key = pendle_contract_key(market_name, "market");
        let sy_key = pendle_contract_key(market_name, "sy");
        let yt_key = pendle_contract_key(market_name, "yt");

        let chain = pendle_chain(&self.contracts, market_name);

        println!(
            "  PENDLE MINT_PT: {} with {:.4} {} (input_token={:?})",
            market_name, input_amount,
            self.input_token.as_deref().unwrap_or("?"),
            self.input_token,
        );

        if let Some(ref ch) = chain {
            let market_addr = evm::resolve_contract(&self.contracts, &market_key, ch);
            let router = evm::resolve_contract(&self.contracts, "pendle_router", ch);
            println!(
                "  PENDLE: chain={}, market={}, router={}",
                ch,
                market_addr
                    .map(|a| evm::short_addr(&a))
                    .unwrap_or("none".to_string()),
                router
                    .map(|r| evm::short_addr(&r))
                    .unwrap_or("none".to_string()),
            );
        }

        if self.dry_run {
            println!("  PENDLE: [DRY RUN] would deposit → SY → mintPyFromSy()");
            let pt_amount = input_amount * 0.98;
            *self
                .pt_holdings
                .entry(market_name.to_string())
                .or_insert(0.0) += pt_amount;
            return Ok(ExecutionResult::PositionUpdate {
                consumed: input_amount,
                output: None,
            });
        }

        let Some(ch) = chain else {
            anyhow::bail!(
                "Pendle market '{}' not in contracts manifest (key: {})",
                market_name,
                market_key,
            );
        };

        let sy_addr = evm::resolve_contract(&self.contracts, &sy_key, &ch)
            .with_context(|| format!("'{}' not in contracts manifest for chain {}", sy_key, ch))?;
        let yt_addr = evm::resolve_contract(&self.contracts, &yt_key, &ch)
            .with_context(|| format!("'{}' not in contracts manifest for chain {}", yt_key, ch))?;
        let router_addr = evm::resolve_contract(&self.contracts, "pendle_router", &ch)
            .with_context(|| {
                format!("'pendle_router' not in contracts manifest for chain {}", ch)
            })?;

        let signer: alloy::signers::local::PrivateKeySigner = self
            .private_key
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid key: {e}"))?;
        let wallet = alloy::network::EthereumWallet::from(signer);
        let provider = ProviderBuilder::new()
            .wallet(wallet)
            .connect_http(ch.rpc_url().expect("Pendle chain requires RPC").parse()?);

        let amount_units = evm::to_token_units(input_amount, 1.0, 18);

        // Step 1: Deposit input token into SY contract
        let sy_balance = self.deposit_into_sy(&provider, sy_addr, &ch, amount_units).await?;
        println!("  PENDLE: SY balance after deposit: {}", evm::from_token_units(sy_balance, 18));

        // Step 2: Approve SY tokens for router
        let erc20 = IERC20::new(sy_addr, &provider);
        erc20
            .approve(router_addr, sy_balance)
            .send()
            .await
            .context("approve SY for router")?
            .get_receipt()
            .await?;

        // Step 3: Mint PT+YT from SY
        let router = IPendleRouter::new(router_addr, &provider);
        let result = router
            .mintPyFromSy(self.wallet_address, yt_addr, sy_balance, U256::ZERO)
            .send()
            .await
            .context("mintPyFromSy")?;
        let receipt = result.get_receipt().await.context("mint receipt")?;
        println!("  PENDLE: mint tx: {:?}", receipt.transaction_hash);

        *self
            .pt_holdings
            .entry(market_name.to_string())
            .or_insert(0.0) += input_amount;
        Ok(ExecutionResult::PositionUpdate {
            consumed: input_amount,
            output: None,
        })
    }

    async fn execute_redeem_pt(&mut self, market_name: &str) -> Result<ExecutionResult> {
        let holdings = self.pt_holdings.get(market_name).copied().unwrap_or(0.0);
        let out_token = self.output_token();

        println!(
            "  PENDLE REDEEM_PT: {} (holdings: {:.4} {})",
            market_name, holdings, out_token,
        );

        if self.dry_run {
            println!("  PENDLE: [DRY RUN] would redeemPyToSy()");
            let output = holdings;
            self.pt_holdings.remove(market_name);
            return Ok(ExecutionResult::TokenOutput {
                token: out_token,
                amount: output,
            });
        }

        let output = holdings;
        self.pt_holdings.remove(market_name);
        Ok(ExecutionResult::TokenOutput {
            token: out_token,
            amount: output,
        })
    }

    async fn execute_claim_rewards(&mut self, market_name: &str) -> Result<ExecutionResult> {
        println!("  PENDLE CLAIM_REWARDS: {}", market_name);

        if self.dry_run {
            println!("  PENDLE: [DRY RUN] would redeemDueInterestAndRewards()");
            return Ok(ExecutionResult::Noop);
        }

        Ok(ExecutionResult::Noop)
    }
}

#[async_trait]
impl Venue for PendleYield {
    async fn execute(&mut self, node: &Node, input_amount: f64) -> Result<ExecutionResult> {
        match node {
            Node::Pendle { market, action, .. } => match action {
                PendleAction::MintPt => self.execute_mint_pt(market, input_amount).await,
                PendleAction::RedeemPt => self.execute_redeem_pt(market).await,
                PendleAction::MintYt => {
                    println!("  PENDLE MINT_YT: {} {:.4}", market, input_amount);
                    if self.dry_run {
                        println!("  PENDLE: [DRY RUN] would mint YT");
                    }
                    *self.yt_holdings.entry(market.to_string()).or_insert(0.0) += input_amount;
                    Ok(ExecutionResult::PositionUpdate {
                        consumed: input_amount,
                        output: None,
                    })
                }
                PendleAction::RedeemYt => {
                    println!("  PENDLE REDEEM_YT: {}", market);
                    let holdings = self.yt_holdings.get(market).copied().unwrap_or(0.0);
                    if self.dry_run {
                        println!("  PENDLE: [DRY RUN] would redeem YT");
                    }
                    self.yt_holdings.remove(market);
                    Ok(ExecutionResult::TokenOutput {
                        token: self.output_token(),
                        amount: holdings,
                    })
                }
                PendleAction::ClaimRewards => self.execute_claim_rewards(market).await,
            },
            _ => {
                println!("  PENDLE: unsupported node type '{}'", node.type_name());
                Ok(ExecutionResult::Noop)
            }
        }
    }

    async fn total_value(&self) -> Result<f64> {
        // Live mode: query on-chain PT/YT token balances for accurate TVL.
        if !self.dry_run {
            if let Some(ref market) = self.market_name {
                match self.query_onchain_value(market).await {
                    Ok(val) if val > 0.0 => return Ok(val),
                    Ok(_) => {} // 0 balance, fall through to local tracking
                    Err(e) => {
                        eprintln!(
                            "  PENDLE: on-chain query failed, falling back to local: {:#}",
                            e
                        );
                    }
                }
            }
        }
        let pt_total: f64 = self.pt_holdings.values().sum();
        let yt_total: f64 = self.yt_holdings.values().sum();
        Ok(pt_total + yt_total)
    }

    async fn unwind(&mut self, fraction: f64) -> Result<f64> {
        let total = self.total_value().await?;
        if total <= 0.0 || fraction <= 0.0 {
            return Ok(0.0);
        }
        let f = fraction.min(1.0);
        let freed = total * f;

        println!("  PENDLE: UNWIND {:.1}% ({:.4})", f * 100.0, freed);

        if self.dry_run {
            println!(
                "  PENDLE: [DRY RUN] would redeemPyToSy for {:.1}% of holdings",
                f * 100.0
            );
            for val in self.pt_holdings.values_mut() {
                *val *= 1.0 - f;
            }
            for val in self.yt_holdings.values_mut() {
                *val *= 1.0 - f;
            }
            self.pt_holdings.retain(|_, v| *v > 1e-12);
            self.yt_holdings.retain(|_, v| *v > 1e-12);
            return Ok(freed);
        }

        // Live: call redeemPyToSy for each market's PT fraction
        let markets: Vec<String> = self.pt_holdings.keys().cloned().collect();
        for market_name in &markets {
            let pt_amount = self.pt_holdings.get(market_name).copied().unwrap_or(0.0);
            let redeem_amount = pt_amount * f;
            if redeem_amount < 1e-12 {
                continue;
            }

            let yt_key = pendle_contract_key(market_name, "yt");
            let chain = match pendle_chain(&self.contracts, market_name) {
                Some(ch) => ch,
                None => {
                    eprintln!(
                        "  PENDLE: unwind skipping {} — chain not found",
                        market_name
                    );
                    continue;
                }
            };
            let rpc_url = match chain.rpc_url() {
                Some(url) => url,
                None => {
                    eprintln!("  PENDLE: unwind skipping {} — no RPC URL", market_name);
                    continue;
                }
            };

            let yt_addr = match evm::resolve_contract(&self.contracts, &yt_key, &chain) {
                Some(addr) => addr,
                None => {
                    eprintln!(
                        "  PENDLE: unwind skipping {} — YT address not found",
                        market_name
                    );
                    continue;
                }
            };
            let router_addr = match evm::resolve_contract(&self.contracts, "pendle_router", &chain)
            {
                Some(addr) => addr,
                None => {
                    eprintln!(
                        "  PENDLE: unwind skipping {} — router not found",
                        market_name
                    );
                    continue;
                }
            };

            let signer: alloy::signers::local::PrivateKeySigner = self
                .private_key
                .parse()
                .map_err(|e| anyhow::anyhow!("Invalid key: {e}"))?;
            let wallet = alloy::network::EthereumWallet::from(signer);
            let provider = ProviderBuilder::new()
                .wallet(wallet)
                .connect_http(rpc_url.parse()?);

            let amount_units = evm::to_token_units(redeem_amount, 1.0, 18);

            let router = IPendleRouter::new(router_addr, &provider);
            match router
                .redeemPyToSy(self.wallet_address, yt_addr, amount_units, U256::ZERO)
                .send()
                .await
            {
                Ok(pending) => {
                    let receipt = pending.get_receipt().await?;
                    println!("  PENDLE: redeemPyToSy tx: {:?}", receipt.transaction_hash);
                }
                Err(e) => {
                    eprintln!("  PENDLE: redeemPyToSy failed for {}: {:#}", market_name, e);
                }
            }
        }

        // Update internal tracking
        for val in self.pt_holdings.values_mut() {
            *val *= 1.0 - f;
        }
        for val in self.yt_holdings.values_mut() {
            *val *= 1.0 - f;
        }
        self.pt_holdings.retain(|_, v| *v > 1e-12);
        self.yt_holdings.retain(|_, v| *v > 1e-12);

        Ok(freed)
    }

    async fn tick(&mut self, now: u64, _dt_secs: f64) -> Result<()> {
        let pt_total: f64 = self.pt_holdings.values().sum();
        let yt_total: f64 = self.yt_holdings.values().sum();
        if pt_total > 0.0 || yt_total > 0.0 {
            println!("  PENDLE TICK: PT={:.4}, YT={:.4}", pt_total, yt_total,);
        }

        // Query on-chain position value and compute APY from single previous sample.
        let val = self.total_value().await.unwrap_or(0.0);
        if val > 0.0 {
            if let Some((prev_ts, prev_val)) = self.prev_value {
                let dt = now.saturating_sub(prev_ts) as f64;
                if dt > 0.0 && prev_val > 0.0 {
                    let period_return = val / prev_val - 1.0;
                    let apy = period_return * (365.25 * 86400.0 / dt);
                    self.cached_alpha = Some((apy, apy.abs() * 0.3));
                }
            }
            self.prev_value = Some((now, val));
        }
        Ok(())
    }

    fn alpha_stats(&self) -> Option<(f64, f64)> {
        self.cached_alpha
    }
}
