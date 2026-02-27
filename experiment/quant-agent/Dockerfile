# syntax=docker/dockerfile:1

# Stage 1: Build defi-flow binary
FROM rust:1.91-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY ferrofluid/ ferrofluid/
RUN cargo build --release --bin defi-flow

# Stage 2: OpenClaw + defi-flow
FROM node:22-bookworm-slim

# Install system deps
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
      ca-certificates \
      curl \
      git \
      jq \
    && rm -rf /var/lib/apt/lists/*

# Install OpenClaw
RUN npm install -g openclaw@latest

# Copy defi-flow binary
COPY --from=builder /build/target/release/defi-flow /usr/local/bin/defi-flow
RUN chmod +x /usr/local/bin/defi-flow

# Clone shared skills repo via GitHub App token (short-lived, never baked into image)
RUN --mount=type=secret,id=gh_token \
    git clone --recursive \
    https://x-access-token:$(cat /run/secrets/gh_token)@github.com/omo-protocol/clawd-shared-configs.git \
    /tmp/shared-configs

# Setup workspace
WORKDIR /app
RUN mkdir -p /app/workspace/skills /app/workspace/memory /app/strategies /app/data

# Copy shared skills + TOOLS.md from cloned repo
RUN cp -r /tmp/shared-configs/skills/* /app/workspace/skills/
RUN cp /tmp/shared-configs/TOOLS.md /app/workspace/TOOLS-shared.md

# Copy our workspace files (overrides shared where names collide)
COPY experiment/quant-agent/AGENTS.md /app/workspace/
COPY experiment/quant-agent/SOUL.md /app/workspace/
COPY experiment/quant-agent/TOOLS.md /app/workspace/
COPY experiment/quant-agent/HEARTBEAT.md /app/workspace/
COPY experiment/quant-agent/MEMORY.md /app/workspace/
COPY experiment/quant-agent/openclaw.json /root/.openclaw/openclaw.json

# Copy our custom skills (overrides shared backtest with our defi-flow-specific one)
COPY experiment/quant-agent/skills/ /app/workspace/skills/

# Cleanup clone
RUN rm -rf /tmp/shared-configs

# Install mongodb driver for log shipping
RUN npm install -g mongodb@6

# Copy scripts
COPY experiment/quant-agent/scripts/ /app/scripts/
COPY experiment/quant-agent/entrypoint.sh /app/entrypoint.sh
RUN chmod +x /app/entrypoint.sh

# Volumes for persistent data
VOLUME ["/app/strategies", "/app/data", "/app/workspace/memory"]

# Expose gateway port
EXPOSE 18789

# Entrypoint: start gateway + register cron jobs
ENTRYPOINT ["/app/entrypoint.sh"]
