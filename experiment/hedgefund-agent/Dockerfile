# syntax=docker/dockerfile:1

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

# Install foundry (cast)
RUN curl -L https://foundry.paradigm.xyz | bash && \
    /root/.foundry/bin/foundryup
ENV PATH="/root/.foundry/bin:${PATH}"

# Clone shared skills repo via GitHub App token
RUN --mount=type=secret,id=gh_token \
    git clone --recursive \
    https://x-access-token:$(cat /run/secrets/gh_token)@github.com/omo-protocol/clawd-shared-configs.git \
    /tmp/shared-configs

# Setup workspace
WORKDIR /app
RUN mkdir -p /app/workspace/skills /app/workspace/memory

# Copy shared skills
RUN cp -r /tmp/shared-configs/skills/* /app/workspace/skills/
RUN rm -rf /tmp/shared-configs

# Copy workspace files
COPY experiment/hedgefund-agent/AGENTS.md /app/workspace/
COPY experiment/hedgefund-agent/SOUL.md /app/workspace/
COPY experiment/hedgefund-agent/TOOLS.md /app/workspace/
COPY experiment/hedgefund-agent/HEARTBEAT.md /app/workspace/
COPY experiment/hedgefund-agent/MEMORY.md /app/workspace/
COPY experiment/hedgefund-agent/vaults.json /app/workspace/
COPY experiment/hedgefund-agent/openclaw.json /root/.openclaw/openclaw.json

# Copy custom skills (override shared where names collide)
COPY experiment/hedgefund-agent/skills/ /app/workspace/skills/

# Install mongodb driver for log shipping
RUN npm install -g mongodb@6

# Copy scripts
COPY experiment/hedgefund-agent/scripts/ /app/scripts/
COPY experiment/hedgefund-agent/entrypoint.sh /app/entrypoint.sh
RUN chmod +x /app/entrypoint.sh

VOLUME ["/app/workspace/memory"]

EXPOSE 18790

ENTRYPOINT ["/app/entrypoint.sh"]
