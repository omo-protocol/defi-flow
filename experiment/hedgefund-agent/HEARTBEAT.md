# HEARTBEAT.md

## Vault Health (every heartbeat)
- [ ] Read `vaults.json` — check each whitelisted vault's reserve ratio
- [ ] Any vault with reserve_ratio < trigger_threshold? Flag immediately.
- [ ] Any vault TVL changed >10% since last check? Investigate.

## Running Daemons
- [ ] Run `defi-flow ps` — are all strategy daemons alive?
- [ ] Any crashed daemons? Check `defi-flow logs <name>`, restart if transient.
- [ ] Any daemon TVL dropped >15%? Investigate and report.

## Reserve Actions
- [ ] Check state files for recent `reserve_actions` entries
- [ ] Log any new reserve unwinds to daily memory

## Memory Maintenance
- [ ] Log vault metrics to `memory/YYYY-MM-DD.md`
- [ ] Update `MEMORY.md` if new persistent learnings found

## State File
Track last check timestamps in `memory/heartbeat-state.json`. Don't repeat work.

When nothing needs attention: `HEARTBEAT_OK`
