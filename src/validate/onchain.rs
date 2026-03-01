use std::collections::{HashMap, HashSet};
use std::time::Duration;

use alloy::primitives::{Address, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::sol;
use ferrofluid::InfoProvider;

use crate::model::Workflow;
use crate::model::chain::Chain;
use crate::model::node::{MovementType, Node, PerpVenue, SpotVenue};

use super::ValidationError;

// ── Minimal ABI interfaces for validation probes ─────────────────────

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IERC20Probe {
        function decimals() external view returns (uint8);
    }

    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IVaultProbe {
        function asset() external view returns (address);
        function totalAssets() external view returns (uint256);
    }

    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IAavePoolProbe {
        function getReserveData(address asset) external view returns (
            uint256, uint128, uint128, uint128, uint128, uint128,
            uint40, uint16, address, address, address, address,
            uint128, uint128, uint128
        );
    }

    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IERC20Balance {
        function balanceOf(address account) external view returns (uint256);
        function decimals() external view returns (uint8);
    }

    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IPendleSYProbe {
        function previewDeposit(address tokenIn, uint256 amountTokenToDeposit) external view returns (uint256);
    }
}

// ── Types ────────────────────────────────────────────────────────────

/// What kind of contract we expect at this address.
#[derive(Debug, Clone)]
enum ContractRole {
    /// ERC20 token — should respond to decimals()
    Token,
    /// ERC4626 vault — should respond to asset()
    Vault,
    /// Aave V3 pool — should respond to getReserveData(token)
    LendingPool {
        /// Token address to probe getReserveData with (resolved from token manifest)
        token_address: Option<Address>,
    },
    /// Rewards controller — just check code exists (no standard probe)
    RewardsController,
    /// Unknown contract — just check code exists
    Unknown,
}

#[derive(Debug)]
struct AddressCheck {
    label: String,
    role: ContractRole,
    address_str: String,
}

// ── Public entry point ───────────────────────────────────────────────

/// Run on-chain validation: check RPC connectivity, chain IDs,
/// deployed code, and correct interfaces at all manifest addresses.
pub async fn validate_onchain(workflow: &Workflow) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    // Hyperliquid universe check (namespace chains — perps + spot)
    errors.extend(check_hyperliquid_universe(workflow).await);

    let chain_map = collect_chains(workflow);
    let contract_roles = infer_contract_roles(workflow);
    let address_groups = group_addresses(workflow, &contract_roles);

    for (chain_name, checks) in &address_groups {
        // Resolve chain with RPC URL
        let chain = chain_map
            .get(&chain_name.to_lowercase())
            .filter(|c| c.rpc_url().is_some())
            .cloned()
            .unwrap_or_else(|| Chain::from_name(chain_name));

        let rpc_url = match chain.rpc_url() {
            Some(url) => url.to_string(),
            None => continue,
        };

        // Check RPC connectivity
        let provider = match check_rpc(&chain, &rpc_url).await {
            Ok(p) => p,
            Err(e) => {
                errors.push(e);
                continue;
            }
        };

        // Check chain ID
        if let Some(expected) = chain.chain_id() {
            match tokio::time::timeout(Duration::from_secs(10), provider.get_chain_id()).await {
                Ok(Ok(actual)) if actual != expected => {
                    errors.push(ValidationError::ChainIdMismatch {
                        chain: chain_name.clone(),
                        expected,
                        actual,
                    });
                    continue;
                }
                Ok(Err(_)) | Err(_) => {}
                _ => {}
            }
        }

        // Check code + interface for all addresses on this chain
        errors.extend(check_addresses(&provider, chain_name, checks).await);
    }

    // Movement node quote checks (LiFi)
    errors.extend(check_movement_quotes(workflow).await);

    // Pendle balance / minimum warnings (informational)
    check_pendle_balance_warnings(workflow, &chain_map).await;

    errors
}

// ── Movement quote validation (LiFi) ────────────────────────────────

/// For each Movement node, hit the LiFi /quote endpoint to verify a route
/// actually exists for the requested token pair + chains.
async fn check_movement_quotes(workflow: &Workflow) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    let token_manifest = workflow.tokens.clone().unwrap_or_default();

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("defi-flow/0.1")
        .build()
    {
        Ok(c) => c,
        Err(_) => return errors, // can't build client — skip silently
    };

    for node in &workflow.nodes {
        let (id, movement_type, provider, from_token, to_token, from_chain, to_chain) = match node {
            Node::Movement {
                id,
                movement_type,
                provider,
                from_token,
                to_token,
                from_chain,
                to_chain,
                ..
            } => (
                id,
                movement_type,
                provider,
                from_token,
                to_token,
                from_chain,
                to_chain,
            ),
            _ => continue,
        };

        // Only quote-check LiFi nodes — HyperliquidNative uses its own native bridge
        if !matches!(provider, crate::model::node::MovementProvider::LiFi) {
            continue;
        }

        // Determine source and destination chains based on movement type
        let (src_chain, dst_chain) = match movement_type {
            MovementType::Swap => {
                let c = from_chain
                    .as_ref()
                    .or(to_chain.as_ref())
                    .cloned()
                    .unwrap_or_else(Chain::hyperevm);
                (c.clone(), c)
            }
            MovementType::Bridge | MovementType::SwapBridge => {
                let fc = match from_chain.as_ref() {
                    Some(c) => c.clone(),
                    None => continue, // can't validate without from_chain
                };
                let tc = match to_chain.as_ref() {
                    Some(c) => c.clone(),
                    None => continue, // can't validate without to_chain
                };
                (fc, tc)
            }
        };

        // Resolve chain_id from registry if missing (e.g. JSON has just {"name": "hyperliquid"})
        let src_chain = if src_chain.chain_id().is_none() {
            Chain::from_name(&src_chain.name)
        } else {
            src_chain
        };
        let dst_chain = if dst_chain.chain_id().is_none() {
            Chain::from_name(&dst_chain.name)
        } else {
            dst_chain
        };

        // LiFi requires chain_id on both sides
        let from_chain_id = match src_chain.chain_id() {
            Some(id) => id,
            None => continue, // unknown chain with no chain_id — skip
        };
        let to_chain_id = match dst_chain.chain_id() {
            Some(id) => id,
            None => continue,
        };

        // Resolve token addresses from manifest, fall back to symbol
        let from_addr = token_manifest
            .get(from_token.as_str())
            .and_then(|chains| {
                chains
                    .iter()
                    .find(|(c, _)| c.eq_ignore_ascii_case(&src_chain.name))
                    .map(|(_, addr)| addr.clone())
            })
            .unwrap_or_else(|| from_token.clone());

        let to_addr = token_manifest
            .get(to_token.as_str())
            .and_then(|chains| {
                chains
                    .iter()
                    .find(|(c, _)| c.eq_ignore_ascii_case(&dst_chain.name))
                    .map(|(_, addr)| addr.clone())
            })
            .unwrap_or_else(|| to_token.clone());

        // Use a small test amount (1 USDC = 1_000_000 for 6-decimal tokens,
        // or 1e18 for 18-decimal tokens)
        let test_amount = match from_token.to_uppercase().as_str() {
            "USDC" | "USDT" | "USDT0" | "USDE" => "1000000",
            "WBTC" | "CBBTC" => "100000000",
            _ => "1000000000000000000", // 1e18
        };

        // Use a dummy wallet address for the quote
        let dummy_wallet = "0x0000000000000000000000000000000000000001";

        let url = format!(
            "https://li.quest/v1/quote?\
            fromChain={from_chain_id}&\
            toChain={to_chain_id}&\
            fromToken={from_addr}&\
            toToken={to_addr}&\
            fromAmount={test_amount}&\
            fromAddress={dummy_wallet}&\
            slippage=0.03"
        );

        println!(
            "  LiFi quote check: {id} ({from_token}@{from_chain_id} → {to_token}@{to_chain_id})"
        );

        match tokio::time::timeout(Duration::from_secs(15), client.get(&url).send()).await {
            Ok(Ok(resp)) => {
                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    // Try to extract a useful message from LiFi error JSON
                    let reason = serde_json::from_str::<serde_json::Value>(&body)
                        .ok()
                        .and_then(|v| v.get("message").and_then(|m| m.as_str().map(String::from)))
                        .unwrap_or_else(|| format!("HTTP {status}"));
                    errors.push(ValidationError::MovementNoRoute {
                        node_id: id.clone(),
                        provider: "LiFi".to_string(),
                        reason,
                    });
                }
            }
            Ok(Err(e)) => {
                errors.push(ValidationError::MovementNoRoute {
                    node_id: id.clone(),
                    provider: "LiFi".to_string(),
                    reason: format!("request failed: {e}"),
                });
            }
            Err(_) => {
                errors.push(ValidationError::MovementNoRoute {
                    node_id: id.clone(),
                    provider: "LiFi".to_string(),
                    reason: "timeout (15s)".to_string(),
                });
            }
        }
    }

    errors
}

// ── Chain collection ─────────────────────────────────────────────────

fn collect_chains(workflow: &Workflow) -> HashMap<String, Chain> {
    let mut chains: HashMap<String, Chain> = HashMap::new();
    for node in &workflow.nodes {
        if let Some(chain) = node.chain() {
            chains.entry(chain.name.to_lowercase()).or_insert(chain);
        }
    }
    // Include reserve vault chain
    if let Some(ref rc) = workflow.reserve {
        chains
            .entry(rc.vault_chain.name.to_lowercase())
            .or_insert(rc.vault_chain.clone());
    }
    chains
}

// ── Contract role inference ──────────────────────────────────────────

/// Walk workflow nodes to determine what role each contract manifest key plays.
/// Returns a map of (contract_key, chain_name) → ContractRole.
fn infer_contract_roles(workflow: &Workflow) -> HashMap<(String, String), ContractRole> {
    let mut roles: HashMap<(String, String), ContractRole> = HashMap::new();

    // Pre-resolve token addresses from manifest for lending pool probes
    let token_manifest = workflow.tokens.clone().unwrap_or_default();

    for node in &workflow.nodes {
        match node {
            Node::Lending {
                chain,
                pool_address,
                rewards_controller,
                asset,
                ..
            } => {
                // Resolve the token address for this asset on this chain
                let token_addr = token_manifest.get(asset.as_str()).and_then(|chains| {
                    chains
                        .iter()
                        .find(|(c, _)| c.eq_ignore_ascii_case(&chain.name))
                        .and_then(|(_, addr)| addr.parse::<Address>().ok())
                });

                roles.insert(
                    (pool_address.clone(), chain.name.to_lowercase()),
                    ContractRole::LendingPool {
                        token_address: token_addr,
                    },
                );
                if let Some(rc) = rewards_controller {
                    roles.insert(
                        (rc.clone(), chain.name.to_lowercase()),
                        ContractRole::RewardsController,
                    );
                }
            }
            Node::Vault {
                chain,
                vault_address,
                ..
            } => {
                roles.insert(
                    (vault_address.clone(), chain.name.to_lowercase()),
                    ContractRole::Vault,
                );
            }
            Node::Lp { chain, pool, .. } => {
                // Position manager — just check code exists (no standard probe interface)
                let chain_name = chain
                    .as_ref()
                    .map(|c| c.name.to_lowercase())
                    .unwrap_or_else(|| "base".to_string());
                roles.insert(
                    ("aerodrome_position_manager".to_string(), chain_name.clone()),
                    ContractRole::Unknown,
                );

                // Pool tokens — validate as ERC20 on the LP chain
                for token_sym in pool.split('/') {
                    if let Some(addr_str) =
                        token_manifest.get(token_sym.trim()).and_then(|chains| {
                            chains
                                .iter()
                                .find(|(c, _)| c.eq_ignore_ascii_case(&chain_name))
                                .map(|(_, addr)| addr.clone())
                        })
                    {
                        roles.insert((addr_str, chain_name.clone()), ContractRole::Token);
                    }
                }
            }
            _ => {}
        }
    }

    // Reserve vault: treat as ERC4626 Vault role
    if let Some(ref rc) = workflow.reserve {
        roles.insert(
            (rc.vault_address.clone(), rc.vault_chain.name.to_lowercase()),
            ContractRole::Vault,
        );
    }

    roles
}

// ── Address grouping ─────────────────────────────────────────────────

/// Group all manifest addresses by chain, annotated with their expected role.
fn group_addresses(
    workflow: &Workflow,
    contract_roles: &HashMap<(String, String), ContractRole>,
) -> HashMap<String, Vec<AddressCheck>> {
    let mut groups: HashMap<String, Vec<AddressCheck>> = HashMap::new();

    // Token manifest (skip native token address — address(0) has no code)
    if let Some(ref tokens) = workflow.tokens {
        for (symbol, chain_map) in tokens {
            for (chain_name, address_str) in chain_map {
                if address_str.parse::<Address>().ok() == Some(Address::ZERO) {
                    continue; // native token, no code to check
                }
                groups
                    .entry(chain_name.clone())
                    .or_default()
                    .push(AddressCheck {
                        label: symbol.clone(),
                        role: ContractRole::Token,
                        address_str: address_str.clone(),
                    });
            }
        }
    }

    // Contract manifest
    if let Some(ref contracts) = workflow.contracts {
        for (name, chain_map) in contracts {
            for (chain_name, address_str) in chain_map {
                let role = contract_roles
                    .get(&(name.clone(), chain_name.to_lowercase()))
                    .cloned()
                    .unwrap_or(ContractRole::Unknown);

                groups
                    .entry(chain_name.clone())
                    .or_default()
                    .push(AddressCheck {
                        label: name.clone(),
                        role,
                        address_str: address_str.clone(),
                    });
            }
        }
    }

    groups
}

// ── RPC check ────────────────────────────────────────────────────────

async fn check_rpc(chain: &Chain, rpc_url: &str) -> Result<impl Provider, ValidationError> {
    let provider = ProviderBuilder::new().connect_http(rpc_url.parse().map_err(|e| {
        ValidationError::RpcUnreachable {
            chain: chain.name.clone(),
            url: rpc_url.to_string(),
            reason: format!("{e}"),
        }
    })?);

    match tokio::time::timeout(Duration::from_secs(10), provider.get_block_number()).await {
        Ok(Ok(_)) => Ok(provider),
        Ok(Err(e)) => Err(ValidationError::RpcUnreachable {
            chain: chain.name.clone(),
            url: rpc_url.to_string(),
            reason: e.to_string(),
        }),
        Err(_) => Err(ValidationError::RpcUnreachable {
            chain: chain.name.clone(),
            url: rpc_url.to_string(),
            reason: "timeout (10s)".to_string(),
        }),
    }
}

// ── Address + interface checks ───────────────────────────────────────

async fn check_addresses(
    provider: &impl Provider,
    chain_name: &str,
    checks: &[AddressCheck],
) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    for check in checks {
        let address: Address = match check.address_str.parse() {
            Ok(a) => a,
            Err(_) => {
                errors.push(match check.role {
                    ContractRole::Token => ValidationError::TokenNoCode {
                        token: check.label.clone(),
                        chain: chain_name.to_string(),
                        address: format!("{} (invalid hex)", check.address_str),
                    },
                    _ => ValidationError::ContractNoCode {
                        contract: check.label.clone(),
                        chain: chain_name.to_string(),
                        address: format!("{} (invalid hex)", check.address_str),
                    },
                });
                continue;
            }
        };

        // 1. Check code exists
        let has_code = match tokio::time::timeout(
            Duration::from_secs(10),
            provider.get_code_at(address),
        )
        .await
        {
            Ok(Ok(code)) => {
                if code.is_empty() {
                    errors.push(match check.role {
                        ContractRole::Token => ValidationError::TokenNoCode {
                            token: check.label.clone(),
                            chain: chain_name.to_string(),
                            address: format!("{address}"),
                        },
                        _ => ValidationError::ContractNoCode {
                            contract: check.label.clone(),
                            chain: chain_name.to_string(),
                            address: format!("{address}"),
                        },
                    });
                    false
                } else {
                    true
                }
            }
            _ => {
                // RPC error / timeout — skip interface check
                continue;
            }
        };

        if !has_code {
            continue;
        }

        // 2. Interface probe — verify the contract is the right kind
        if let Some(err) = probe_interface(provider, chain_name, address, check).await {
            errors.push(err);
        }
    }

    errors
}

/// Call a role-specific function to verify the contract implements the expected interface.
async fn probe_interface(
    provider: &impl Provider,
    chain_name: &str,
    address: Address,
    check: &AddressCheck,
) -> Option<ValidationError> {
    let timeout = Duration::from_secs(10);

    match &check.role {
        ContractRole::Token => {
            // ERC20 must respond to decimals()
            let contract = IERC20Probe::new(address, provider);
            match tokio::time::timeout(timeout, contract.decimals().call()).await {
                Ok(Ok(_)) => None,
                _ => Some(ValidationError::WrongInterface {
                    contract: check.label.clone(),
                    chain: chain_name.to_string(),
                    address: format!("{address}"),
                    expected: "ERC20 — decimals() call failed".to_string(),
                }),
            }
        }
        ContractRole::Vault => {
            // ERC4626 vault must respond to asset(). totalAssets() may revert on
            // unconfigured Morpho vaults (no markets allocated yet) — that's OK.
            let contract = IVaultProbe::new(address, provider);
            let asset_ok = tokio::time::timeout(timeout, contract.asset().call()).await;
            match asset_ok {
                Ok(Ok(_)) => None,
                _ => Some(ValidationError::WrongInterface {
                    contract: check.label.clone(),
                    chain: chain_name.to_string(),
                    address: format!("{address}"),
                    expected: "ERC4626 vault — asset() call failed".to_string(),
                }),
            }
        }
        ContractRole::LendingPool { token_address } => {
            // Aave V3 pool must respond to getReserveData(token)
            if let Some(token) = token_address {
                let contract = IAavePoolProbe::new(address, provider);
                match tokio::time::timeout(timeout, contract.getReserveData(*token).call()).await {
                    Ok(Ok(_)) => None,
                    _ => Some(ValidationError::WrongInterface {
                        contract: check.label.clone(),
                        chain: chain_name.to_string(),
                        address: format!("{address}"),
                        expected: "Aave V3 pool — getReserveData() call failed".to_string(),
                    }),
                }
            } else {
                None // Can't probe without a token address
            }
        }
        ContractRole::RewardsController | ContractRole::Unknown => {
            None // Code check is sufficient
        }
    }
}

// ── Hyperliquid universe validation ─────────────────────────────────

/// Verify that Hyperliquid perp coins and spot tokens actually exist
/// in the live Hyperliquid universe (API check).
///
/// Note: on Hyperliquid, major assets (ETH, BTC) are traded "spot" via the
/// perp infrastructure, not the HIP-2 spot meta. So we check spot base tokens
/// against BOTH perp universe AND spot token list.
async fn check_hyperliquid_universe(workflow: &Workflow) -> Vec<ValidationError> {
    let mut perp_coins: Vec<(String, String)> = Vec::new(); // (node_id, coin)
    let mut spot_bases: Vec<(String, String)> = Vec::new(); // (node_id, base_token)

    for node in &workflow.nodes {
        match node {
            Node::Perp {
                id, venue, pair, ..
            } if matches!(venue, PerpVenue::Hyperliquid) => {
                let coin = pair.split('/').next().unwrap_or(pair).to_string();
                perp_coins.push((id.clone(), coin));
            }
            Node::Spot {
                id, venue, pair, ..
            } if matches!(venue, SpotVenue::Hyperliquid) => {
                // Only check the base token (e.g. "ETH" from "ETH/USDC")
                // Quote token (USDC) is always valid on HL
                let base = pair.split('/').next().unwrap_or(pair).trim().to_string();
                spot_bases.push((id.clone(), base));
            }
            _ => {}
        }
    }

    if perp_coins.is_empty() && spot_bases.is_empty() {
        return Vec::new();
    }

    let info = InfoProvider::mainnet();
    let mut errors = Vec::new();

    // Fetch perp universe (needed for both perp + spot validation)
    let perp_known: HashSet<String> =
        match tokio::time::timeout(Duration::from_secs(10), info.meta()).await {
            Ok(Ok(meta)) => {
                let known: HashSet<String> = meta
                    .universe
                    .iter()
                    .filter(|a| !a.is_delisted.unwrap_or(false))
                    .map(|a| a.name.to_uppercase())
                    .collect();
                println!("  HL perp universe: {} listed assets", known.len());
                known
            }
            Ok(Err(e)) => {
                eprintln!("  warning: could not fetch HL perp meta: {e}");
                HashSet::new()
            }
            Err(_) => {
                eprintln!("  warning: HL perp meta request timed out");
                HashSet::new()
            }
        };

    // Check perp coins
    for (node_id, coin) in &perp_coins {
        if !perp_known.is_empty() && !perp_known.contains(&coin.to_uppercase()) {
            errors.push(ValidationError::HyperliquidUnknownPerpAsset {
                node_id: node_id.clone(),
                asset: coin.clone(),
            });
        }
    }

    // Check spot base tokens: valid if in perp universe OR spot token list
    if !spot_bases.is_empty() {
        // Also fetch spot tokens for HIP-2 memecoins
        let spot_known: HashSet<String> =
            match tokio::time::timeout(Duration::from_secs(10), info.spot_meta()).await {
                Ok(Ok(spot_meta)) => {
                    let known: HashSet<String> = spot_meta
                        .tokens
                        .iter()
                        .map(|t| t.name.to_uppercase())
                        .collect();
                    println!("  HL spot universe: {} listed tokens", known.len());
                    known
                }
                Ok(Err(e)) => {
                    eprintln!("  warning: could not fetch HL spot meta: {e}");
                    HashSet::new()
                }
                Err(_) => {
                    eprintln!("  warning: HL spot meta request timed out");
                    HashSet::new()
                }
            };

        for (node_id, base) in &spot_bases {
            let upper = base.to_uppercase();
            let found = perp_known.contains(&upper) || spot_known.contains(&upper);
            if !perp_known.is_empty() && !spot_known.is_empty() && !found {
                errors.push(ValidationError::HyperliquidUnknownSpotToken {
                    node_id: node_id.clone(),
                    token: base.clone(),
                });
            }
        }
    }

    errors
}

// ── Pendle balance warnings ──────────────────────────────────────────

/// Print informational warnings about wallet balance vs Pendle SY deposit.
/// Not a hard error — just tells the agent how much is available and what to expect.
async fn check_pendle_balance_warnings(
    workflow: &Workflow,
    chain_map: &HashMap<String, Chain>,
) {
    let token_manifest = workflow.tokens.clone().unwrap_or_default();
    let contract_manifest = workflow.contracts.clone().unwrap_or_default();

    // Find wallet address + token
    let wallet_addr = match workflow.nodes.iter().find_map(|n| {
        if let Node::Wallet { address, .. } = n {
            address.parse::<Address>().ok()
        } else {
            None
        }
    }) {
        Some(a) => a,
        None => return,
    };

    // Find Pendle nodes
    for node in &workflow.nodes {
        let (node_id, market, input_token) = match node {
            Node::Pendle { id, market, input_token, .. } => {
                (id, market, input_token.as_deref().unwrap_or("USDC"))
            }
            _ => continue,
        };

        // Resolve SY address from contracts manifest
        let sy_key = format!(
            "pendle_{}_sy",
            market.to_lowercase().replace('-', "_")
        );
        let sy_chain = chain_map
            .values()
            .find(|c| c.rpc_url().is_some())
            .cloned()
            .unwrap_or_else(Chain::hyperevm);
        let sy_addr = contract_manifest
            .get(&sy_key)
            .and_then(|m| m.values().next())
            .and_then(|a| a.parse::<Address>().ok());

        let sy_addr = match sy_addr {
            Some(a) => a,
            None => continue,
        };

        let rpc_url = match sy_chain.rpc_url() {
            Some(u) => u.to_string(),
            None => continue,
        };

        let provider = ProviderBuilder::new().connect_http(match rpc_url.parse() {
            Ok(u) => u,
            Err(_) => continue,
        });

        // Resolve input token address
        let token_addr = token_manifest
            .get(input_token)
            .and_then(|m| m.values().next())
            .and_then(|a| a.parse::<Address>().ok());

        let token_addr = match token_addr {
            Some(a) => a,
            None => continue,
        };

        // Check wallet balance of the input token
        let is_native = token_addr == Address::ZERO;
        let balance = if is_native {
            match tokio::time::timeout(
                Duration::from_secs(10),
                provider.get_balance(wallet_addr),
            ).await {
                Ok(Ok(b)) => b,
                _ => continue,
            }
        } else {
            let erc20 = IERC20Balance::new(token_addr, &provider);
            match tokio::time::timeout(
                Duration::from_secs(10),
                erc20.balanceOf(wallet_addr).call(),
            ).await {
                Ok(Ok(b)) => b,
                _ => continue,
            }
        };

        // Get decimals
        let decimals: u8 = if is_native {
            18
        } else {
            let erc20 = IERC20Balance::new(token_addr, &provider);
            match tokio::time::timeout(
                Duration::from_secs(10),
                erc20.decimals().call(),
            ).await {
                Ok(Ok(d)) => d,
                _ => 18,
            }
        };

        let human_balance = balance.to::<u128>() as f64 / 10f64.powi(decimals as i32);

        // Preview deposit to check if it works
        let sy = IPendleSYProbe::new(sy_addr, &provider);
        let preview = tokio::time::timeout(
            Duration::from_secs(10),
            sy.previewDeposit(token_addr, balance).call(),
        ).await;

        match preview {
            Ok(Ok(shares)) if shares > U256::ZERO => {
                println!(
                    "  Pendle {node_id}: wallet has {human_balance:.2} {input_token} → \
                     previewDeposit OK ({shares} SY shares)"
                );
            }
            Ok(Ok(_)) => {
                eprintln!(
                    "  warning: Pendle {node_id}: wallet has {human_balance:.2} {input_token} \
                     but previewDeposit returns 0 shares — amount may be below SY minimum"
                );
            }
            Ok(Err(e)) => {
                eprintln!(
                    "  warning: Pendle {node_id}: wallet has {human_balance:.2} {input_token} \
                     but previewDeposit reverted — deposit may be below SY minimum ({e})"
                );
            }
            Err(_) => {
                eprintln!("  warning: Pendle {node_id}: previewDeposit timed out");
            }
        }
    }
}
