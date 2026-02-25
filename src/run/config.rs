use std::path::PathBuf;

use alloy::primitives::Address;
use anyhow::{bail, Result};
use ferrofluid::Network;

/// Runtime configuration for the `run` command.
pub struct RuntimeConfig {
    pub network: Network,
    pub wallet_address: Address,
    pub private_key: String,
    pub state_file: PathBuf,
    pub dry_run: bool,
    pub once: bool,
    pub slippage_bps: f64,
}

impl RuntimeConfig {
    /// Build from CLI args. Private key comes from env var.
    pub fn from_cli(cli: &crate::run::RunConfig) -> Result<Self> {
        let private_key = std::env::var("DEFI_FLOW_PRIVATE_KEY").map_err(|_| {
            anyhow::anyhow!(
                "DEFI_FLOW_PRIVATE_KEY env var not set. \
                 Set it to your hex private key (without 0x prefix)."
            )
        })?;

        Self::build(private_key, &cli.network, cli.state_file.clone(), cli.dry_run, cli.once, cli.slippage_bps)
    }

    /// Build from explicit args. Used by the API server where the PK comes from the request.
    pub fn from_args(
        private_key: String,
        network: &str,
        dry_run: bool,
        slippage_bps: f64,
    ) -> Result<Self> {
        Self::build(private_key, network, PathBuf::from("/dev/null"), dry_run, dry_run, slippage_bps)
    }

    fn build(
        private_key: String,
        network: &str,
        state_file: PathBuf,
        dry_run: bool,
        once: bool,
        slippage_bps: f64,
    ) -> Result<Self> {
        let network = match network.to_lowercase().as_str() {
            "mainnet" => Network::Mainnet,
            "testnet" => Network::Testnet,
            other => bail!("Invalid network '{other}'. Use 'mainnet' or 'testnet'."),
        };

        // Derive address from private key
        use alloy::signers::local::PrivateKeySigner;
        let signer: PrivateKeySigner = private_key
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid private key: {e}"))?;
        let wallet_address = signer.address();

        Ok(RuntimeConfig {
            network,
            wallet_address,
            private_key,
            state_file,
            dry_run,
            once,
            slippage_bps,
        })
    }
}
