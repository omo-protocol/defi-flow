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
echo "  2. Create a GitHub PAT with 'read:packages' scope for GHCR pull:"
echo "     https://github.com/settings/tokens/new?scopes=read:packages"
echo ""
echo "  3. Add these GitHub repo secrets:"
echo "     VPS_HOST              = $(hostname -I | awk '{print $1}')"
echo "     VPS_USER              = $(whoami)"
echo "     VPS_SSH_KEY           = (contents of ~/.ssh/deploy_key)"
echo "     GH_APP_ID             = (GitHub App ID for omo-protocol org)"
echo "     GH_APP_PRIVATE_KEY    = (GitHub App private key)"
echo "     GHCR_TOKEN            = (PAT with read:packages for pulling images)"
echo "     ANTHROPIC_API_KEY     = sk-ant-..."
echo "     GATEWAY_AUTH_TOKEN    = (secure random token)"
echo "     MONGODB_URI           = mongodb+srv://..."
echo ""
echo "     # Each service has its own wallet:"
echo "     PK_QUANT_AGENT        = 0x... (quant agent wallet)"
echo "     PK_HEDGEFUND_AGENT    = 0x... (hedgefund agent wallet)"
echo "     PK_STRATEGY_LENDING   = 0x... (lending strategy vault wallet)"
echo "     PK_STRATEGY_DN        = 0x... (delta-neutral strategy vault wallet)"
echo "     PK_STRATEGY_PT        = 0x... (PT yield strategy vault wallet)"
echo ""
echo "  4. Trigger first deploy:"
echo "     gh workflow run deploy.yml --field force_all=true"
echo ""
echo "  No source code is needed on this server."
echo "  Images are pulled from ghcr.io/omo-protocol/defi-flow/*"
