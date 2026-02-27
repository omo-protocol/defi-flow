# Onchain Valuer — Deployment Guide

Each vault strategy pushes its NAV (TVL) to a Morpho v2 valuer contract on HyperEVM. The strategy's runtime wallet (derived from `DEFI_FLOW_PRIVATE_KEY`) is the signer.

## Contract Interface

The valuer contract must implement:

```solidity
interface IValuer {
    function updateValue(
        bytes32 strategyId,
        uint256 value,
        uint256 confidence,
        uint256 nonce,
        uint256 expiry,
        bytes[] calldata signatures
    ) external;

    function getReport(bytes32 strategyId)
        external view returns (
            uint256 value,
            uint256 timestamp,
            uint256 confidence,
            uint256 nonce,
            bool isPush,
            address lastUpdater
        );

    function emergencyUpdate(bytes32 strategyId, uint256 value) external;
    function setEmergencyMode(bool enabled) external;
    function maxPriceChangeBps() external view returns (uint256);
}
```

## Strategy ID

Each strategy has a `strategy_id` text field (e.g. `"lending"`, `"delta_neutral_basic"`). The on-chain ID is:

```
bytes32 strategyId = keccak256(abi.encodePacked(strategyIdText))
```

## Signature Format (EIP-191)

The signer computes:

```
messageHash = keccak256(abi.encode(
    strategyId,    // bytes32
    value,         // uint256
    confidence,    // uint256
    nonce,         // uint256
    expiry,        // uint256
    chainId,       // uint256 (999 for HyperEVM)
    valuerAddress  // address
))

signature = sign("\x19Ethereum Signed Message:\n32" + messageHash)
```

This matches the keeper repo's `_verifySignatures()` pattern.

## Emergency Mode

Emergency mode is used for:

1. **Initial push** — first value update (current value = 0)
2. **Large price changes** — when the value change exceeds `maxPriceChangeBps` (default 5000 = 50%)

Flow: `setEmergencyMode(true)` → `emergencyUpdate(strategyId, value)` → `setEmergencyMode(false)`

The disable step always runs, even if the update reverts.

## Signer Authorization

Each strategy wallet must be authorized as a signer on the valuer contract. One wallet per strategy.

| Strategy | strategy_id | Wallet |
|----------|------------|--------|
| USDC Lending | `lending` | Derived from strategy container's `DEFI_FLOW_PRIVATE_KEY` |
| Delta Neutral | `delta_neutral_basic` | Derived from strategy container's `DEFI_FLOW_PRIVATE_KEY` |
| PT Fixed Yield | `pt_yield` | Derived from strategy container's `DEFI_FLOW_PRIVATE_KEY` |

To get the wallet address: `cast wallet address $DEFI_FLOW_PRIVATE_KEY`

## Strategy JSON Configuration

Each strategy JSON has:

```json
{
  "contracts": {
    "valuer_contract": { "hyperevm": "0x_REPLACE_WITH_DEPLOYED_ADDRESS" }
  },
  "valuer": {
    "contract": "valuer_contract",
    "strategy_id": "lending",
    "chain": { "name": "hyperevm", "chain_id": 999, "rpc_url": "https://rpc.hyperliquid.xyz/evm" },
    "confidence": 90,
    "underlying_decimals": 6,
    "push_interval": 3600
  }
}
```

- `contract`: key in `contracts` manifest → resolves to deployed address
- `confidence`: 0-100 (reported alongside value)
- `underlying_decimals`: 6 for USDC (scales TVL f64 → uint256)
- `push_interval`: seconds between pushes (throttled in-memory, resets on restart)

## Wallet Address

Hardcode the wallet address directly in each strategy JSON. Derive from the PK: `cast wallet address $PK`

## Reserve Management

Each strategy also has a `reserve` config for vault reserve management:

```json
{
  "reserve": {
    "vault_address": "morpho_usdc_vault",
    "vault_chain": { "name": "hyperevm", "chain_id": 999, "rpc_url": "https://rpc.hyperliquid.xyz/evm" },
    "vault_token": "USDC"
  }
}
```

The vault address (`morpho_usdc_vault`) must also be in `contracts` manifest.

## Deployment Checklist

1. Deploy the valuer contract on HyperEVM
2. Hardcode `valuer_contract` address in each strategy JSON
3. Deploy the Morpho USDC vault on HyperEVM
4. Hardcode `morpho_usdc_vault` address in each strategy JSON
5. Hardcode each strategy's wallet address in its JSON (`cast wallet address $PK`)
6. Authorize each strategy wallet as a signer on the valuer contract
7. Set `maxPriceChangeBps` as needed (default 5000 = 50%)
8. Test with `defi-flow run --dry-run` — should log `[valuer] [DRY RUN] would push...`

## Troubleshooting

- **"valuer.getReport() failed"**: Contract not deployed or wrong address
- **"EIP-191 signing failed"**: Invalid PK format
- **"valuer.updateValue reverted"**: Wallet not authorized as signer, or nonce mismatch
- **"setEmergencyMode(true) failed"**: Wallet lacks emergency mode permissions
- **Push not happening**: `push_interval` throttle (default 1 hour). Restarting the daemon resets the throttle.
