#!/bin/bash
set -e

echo "═══════════════════════════════════════════════"
echo " defi-flow VPS Setup"
echo "═══════════════════════════════════════════════"

# Install Docker
if ! command -v docker &> /dev/null; then
  echo ">>> Installing Docker..."
  apt-get update
  apt-get install -y docker.io docker-compose-plugin curl
  systemctl enable docker
  systemctl start docker
  echo "Docker installed."
else
  echo "Docker already installed."
fi

# Create deploy directory
mkdir -p /opt/defi-flow/experiment
echo "Created /opt/defi-flow/experiment/"

echo ""
echo "═══════════════════════════════════════════════"
echo " Setup complete!"
echo "═══════════════════════════════════════════════"
echo ""
echo "Next steps:"
echo ""
echo "  1. Generate an SSH keypair for CI deploy:"
echo "     ssh-keygen -t ed25519 -f ~/.ssh/deploy_key -N ''"
echo "     cat ~/.ssh/deploy_key.pub >> ~/.ssh/authorized_keys"
echo ""
echo "  2. Create a GitHub PAT with 'read:packages,write:packages' scope:"
echo "     https://github.com/settings/tokens/new?scopes=read:packages,write:packages"
echo ""
echo "  3. Add these GitHub repo secrets (Settings > Secrets > Actions):"
echo ""
echo "     # ── VPS Access ──"
echo "     VPS_HOST              = $(hostname -I | awk '{print $1}')"
echo "     VPS_USER              = $(whoami)"
echo "     VPS_SSH_KEY           = (contents of ~/.ssh/deploy_key)"
echo ""
echo "     # ── Registry ──"
echo "     GHCR_TOKEN            = (PAT with read:packages + write:packages)"
echo ""
echo "     # ── Agent LLM Config ──"
echo "     AGENT_API_KEY         = sk-ant-...  (LLM API key)"
echo "     AGENT_BASE_URL        = https://api.anthropic.com"
echo "     AGENT_MODEL           = custom/claude-sonnet-4-5-20250929"
echo "     AGENT_MODEL_ID        = claude-sonnet-4-5-20250929"
echo "     AGENT_API_TYPE        = anthropic-messages  (or openai-completions)"
echo "     ANTHROPIC_API_KEY     = sk-ant-...  (openclaw internal)"
echo ""
echo "     # ── Services ──"
echo "     GATEWAY_AUTH_TOKEN    = (secure random token)"
echo "     MONGODB_URI           = mongodb+srv://..."
echo ""
echo "     # ── Wallet Private Keys (one per service) ──"
echo "     PK_QUANT_AGENT        = 0x..."
echo "     PK_HEDGEFUND_AGENT    = 0x..."
echo "     PK_STRATEGY_LENDING   = 0x..."
echo "     PK_STRATEGY_DN        = 0x..."
echo "     PK_STRATEGY_PT        = 0x..."
echo ""
echo "  4. Trigger first deploy:"
echo "     gh workflow run deploy.yml --field force_all=true"
echo ""
echo "  No source code is needed on this server."
echo "  Images are pulled from ghcr.io/omo-protocol/defi-flow/*"
