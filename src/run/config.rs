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
    pub fn from_cli(cli: &crate::run::RunConfig) -> Result<Self> {
        let private_key = std::env::var("DEFI_FLOW_PRIVATE_KEY").map_err(|_| {
            anyhow::anyhow!(
                "DEFI_FLOW_PRIVATE_KEY env var not set. \
                 Set it to your hex private key (without 0x prefix)."
            )
        })?;

        let network = match cli.network.to_lowercase().as_str() {
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
            state_file: cli.state_file.clone(),
            dry_run: cli.dry_run,
            once: cli.once,
            slippage_bps: cli.slippage_bps,
        })
    }
}
