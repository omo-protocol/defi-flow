# syntax=docker/dockerfile:1

# Stage 1: Build WASM bindings
FROM rust:1.91-bookworm AS wasm-builder
RUN cargo install wasm-pack

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY ferrofluid/Cargo.toml ferrofluid/Cargo.toml

# Cache deps
RUN mkdir -p src ferrofluid/src && \
    echo "fn main() {}" > src/main.rs && \
    touch ferrofluid/src/lib.rs && \
    cargo build --release --no-default-features --features wasm 2>/dev/null || true && \
    rm -rf src ferrofluid/src

COPY src/ src/
COPY ferrofluid/ ferrofluid/
RUN wasm-pack build --target web --out-dir /build/pkg --no-default-features --features wasm

# Stage 2: Build Next.js
FROM node:22-bookworm-slim AS builder

RUN apt-get update && \
    apt-get install -y --no-install-recommends python3 make g++ && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Install deps first (cache layer)
COPY ui/package.json ui/package-lock.json ./
RUN npm ci

# Copy UI source
COPY ui/ .

# Copy WASM pkg from builder
COPY --from=wasm-builder /build/pkg ./pkg/

ENV NEXT_TELEMETRY_DISABLED=1

RUN npm run build

# Stage 3: Slim production runtime
FROM node:22-bookworm-slim
WORKDIR /app

ENV NODE_ENV=production
ENV NEXT_TELEMETRY_DISABLED=1

# Copy standalone output
COPY --from=builder /app/.next/standalone ./
COPY --from=builder /app/.next/static .next/static
COPY --from=builder /app/public public

# Data directory for SQLite (mounted as volume)
RUN mkdir -p /app/data

EXPOSE 3000

CMD ["node", "server.js"]
