# syntax=docker/dockerfile:1

# Stage 1: Install deps + build Next.js
FROM node:22-bookworm-slim AS builder

RUN apt-get update && \
    apt-get install -y --no-install-recommends python3 make g++ && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Install deps first (cache layer)
COPY ui/package.json ui/package-lock.json ./
RUN npm ci

# Copy UI source (node_modules excluded via .dockerignore)
COPY ui/ .

ENV NEXT_TELEMETRY_DISABLED=1

RUN npm run build

# Stage 2: Slim production runtime
FROM node:22-bookworm-slim
WORKDIR /app

ENV NODE_ENV=production
ENV NEXT_TELEMETRY_DISABLED=1

# Copy standalone output (includes node_modules with better-sqlite3)
COPY --from=builder /app/.next/standalone ./
COPY --from=builder /app/.next/static .next/static
COPY --from=builder /app/public public

# Data directory for SQLite (mounted as volume)
RUN mkdir -p /app/data

EXPOSE 3000

CMD ["node", "server.js"]
