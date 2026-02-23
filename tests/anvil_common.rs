#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;

use alloy::node_bindings::Anvil;
use alloy::primitives::{Address, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::sol;

use defi_flow::run::config::RuntimeConfig;
use defi_flow::venues::evm::{ContractManifest, TokenManifest};

// ── Test-only contract interfaces ────────────────────────────────────

sol! {
    #[sol(rpc)]
    contract IERC20Test {
        function transfer(address to, uint256 amount) external returns (bool);
        function balanceOf(address account) external view returns (uint256);
        function approve(address spender, uint256 amount) external returns (bool);
        function decimals() external view returns (uint8);
    }
}

sol! {
    #[sol(rpc)]
    contract IWETH {
        function deposit() external payable;
        function balanceOf(address account) external view returns (uint256);
    }
}

// ── Fork context ─────────────────────────────────────────────────────

pub struct ForkContext {
    pub _anvil: alloy::node_bindings::AnvilInstance,
    pub rpc_url: String,
    pub wallet_address: Address,
    pub private_key: String,
}

/// Spawn an Anvil fork of the given chain.
pub fn spawn_fork(fork_url: &str, chain_id: u64) -> ForkContext {
    let anvil = Anvil::at("/Users/joeru/.foundry/bin/anvil")
        .fork(fork_url)
        .chain_id(chain_id)
        .spawn();

    let rpc_url = anvil.endpoint();
    let wallet_address = anvil.addresses()[0];
    let private_key = hex::encode(anvil.keys()[0].to_bytes());

    ForkContext {
        _anvil: anvil,
        rpc_url,
        wallet_address,
        private_key,
    }
}

// ── Token funding ────────────────────────────────────────────────────

/// Fund native ETH via anvil_setBalance.
pub async fn fund_eth(rpc_url: &str, addr: Address, amount: U256) {
    let provider = ProviderBuilder::new()
        .connect_http(rpc_url.parse().unwrap());
    let _: () = provider
        .raw_request("anvil_setBalance".into(), (addr, amount))
        .await
        .expect("anvil_setBalance failed");
}

/// Fund ERC20 tokens by impersonating a whale and transferring.
pub async fn fund_erc20(
    rpc_url: &str,
    token: Address,
    whale: Address,
    recipient: Address,
    amount: U256,
) {
    let provider = ProviderBuilder::new()
        .connect_http(rpc_url.parse().unwrap());

    // Impersonate whale
    let _: () = provider
        .raw_request("anvil_impersonateAccount".into(), [whale])
        .await
        .expect("anvil_impersonateAccount failed");

    // Fund whale with ETH for gas
    let _: () = provider
        .raw_request(
            "anvil_setBalance".into(),
            (whale, U256::from(100u128 * 10u128.pow(18))),
        )
        .await
        .expect("anvil_setBalance for whale failed");

    // Transfer tokens from whale to recipient
    let erc20 = IERC20Test::new(token, &provider);
    erc20
        .transfer(recipient, amount)
        .from(whale)
        .send()
        .await
        .expect("ERC20 transfer from whale failed")
        .get_receipt()
        .await
        .expect("ERC20 transfer receipt failed");

    // Stop impersonating
    let _: () = provider
        .raw_request("anvil_stopImpersonatingAccount".into(), [whale])
        .await
        .expect("anvil_stopImpersonatingAccount failed");
}

/// Wrap native ETH into WETH.
pub async fn wrap_eth(rpc_url: &str, private_key: &str, weth_addr: Address, amount: U256) {
    let signer: alloy::signers::local::PrivateKeySigner = private_key.parse().unwrap();
    let wallet = alloy::network::EthereumWallet::from(signer);
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(rpc_url.parse().unwrap());

    let weth = IWETH::new(weth_addr, &provider);
    weth.deposit()
        .value(amount)
        .send()
        .await
        .expect("WETH deposit failed")
        .get_receipt()
        .await
        .expect("WETH deposit receipt failed");
}

/// Query ERC20 balance.
pub async fn balance_of(rpc_url: &str, token: Address, account: Address) -> U256 {
    let provider = ProviderBuilder::new()
        .connect_http(rpc_url.parse().unwrap());
    let erc20 = IERC20Test::new(token, &provider);
    erc20
        .balanceOf(account)
        .call()
        .await
        .expect("balanceOf call failed")
}

// ── Config builders ──────────────────────────────────────────────────

pub fn make_config(ctx: &ForkContext) -> RuntimeConfig {
    RuntimeConfig {
        network: ferrofluid::Network::Mainnet,
        wallet_address: ctx.wallet_address,
        private_key: ctx.private_key.clone(),
        state_file: PathBuf::from("/tmp/defi-flow-test-state.json"),
        dry_run: false,
        once: true,
        slippage_bps: 50.0,
    }
}

// ── Manifest builders ────────────────────────────────────────────────

/// Build token manifest with a single token on a single chain.
pub fn token_manifest(entries: &[(&str, &str, &str)]) -> TokenManifest {
    let mut manifest: TokenManifest = HashMap::new();
    for (symbol, chain, address) in entries {
        manifest
            .entry(symbol.to_string())
            .or_default()
            .insert(chain.to_string(), address.to_string());
    }
    manifest
}

/// Build contract manifest with named contract entries.
pub fn contract_manifest(entries: &[(&str, &str, &str)]) -> ContractManifest {
    let mut manifest: ContractManifest = HashMap::new();
    for (name, chain, address) in entries {
        manifest
            .entry(name.to_string())
            .or_default()
            .insert(chain.to_string(), address.to_string());
    }
    manifest
}
