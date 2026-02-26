use std::path::PathBuf;

use alloy::primitives::Address;
use anyhow::{Result, bail};
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
    /// Build from CLI args. Private key comes from env var or file.
    ///
    /// Resolution order:
    /// 1. `DEFI_FLOW_PRIVATE_KEY` env var (direct value)
    /// 2. `DEFI_FLOW_PRIVATE_KEY_FILE` env var (path to file containing the key)
    ///
    /// Using the _FILE variant is preferred in containers — the key never appears
    /// in `env` or `printenv` output, reducing accidental exposure.
    pub fn from_cli(cli: &crate::run::RunConfig) -> Result<Self> {
        let private_key = if let Ok(pk) = std::env::var("DEFI_FLOW_PRIVATE_KEY") {
            pk
        } else if let Ok(path) = std::env::var("DEFI_FLOW_PRIVATE_KEY_FILE") {
            std::fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!("Failed to read private key from {path}: {e}"))?
                .trim()
                .to_string()
        } else {
            return Err(anyhow::anyhow!(
                "Private key not configured. Set DEFI_FLOW_PRIVATE_KEY env var \
                 or DEFI_FLOW_PRIVATE_KEY_FILE pointing to a file containing the key."
            ));
        };

        Self::build(
            private_key,
            &cli.network,
            cli.state_file.clone(),
            cli.dry_run,
            cli.once,
            cli.slippage_bps,
        )
    }

    /// Build from explicit args. Used by the API server where the PK comes from the request.
    pub fn from_args(
        private_key: String,
        network: &str,
        dry_run: bool,
        slippage_bps: f64,
    ) -> Result<Self> {
        Self::build(
            private_key,
            network,
            PathBuf::from("/dev/null"),
            dry_run,
            dry_run,
            slippage_bps,
        )
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
