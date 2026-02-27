# syntax=docker/dockerfile:1

# Stage 1: Build defi-flow binary (with cargo cache)
FROM rust:1.91-bookworm AS builder
WORKDIR /build

# Cache deps — only re-download when Cargo.toml/lock change
COPY Cargo.toml Cargo.lock ./
COPY ferrofluid/Cargo.toml ferrofluid/Cargo.toml
RUN mkdir -p src ferrofluid/src && \
    echo "fn main() {}" > src/main.rs && \
    touch ferrofluid/src/lib.rs && \
    cargo build --release --bin defi-flow 2>/dev/null || true && \
    rm -rf src ferrofluid/src

# Now copy real source and build (only recompiles changed crates)
COPY src/ src/
COPY ferrofluid/ ferrofluid/
RUN cargo build --release --bin defi-flow

# Stage 2: Minimal runtime with strategy
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates curl && \
    rm -rf /var/lib/apt/lists/*

# Copy defi-flow binary
COPY --from=builder /build/target/release/defi-flow /usr/local/bin/defi-flow
RUN chmod +x /usr/local/bin/defi-flow

WORKDIR /app
RUN mkdir -p /app/data /app/logs

# Copy the strategy JSON (set via build arg)
ARG STRATEGY=lending.json
COPY experiment/vault-strategies/${STRATEGY} /app/strategy.json

# Persistent volumes for state and data
VOLUME ["/app/data", "/app/logs"]

# Default: dry-run daemon mode
# Override CMD in docker-compose for production
ENTRYPOINT ["defi-flow", "run", "/app/strategy.json"]
CMD ["--dry-run", "--state-file", "/app/data/state.json", "--log-file", "/app/logs/strategy.log", "--network", "testnet"]
