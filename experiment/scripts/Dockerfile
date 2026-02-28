FROM node:22-bookworm-slim
WORKDIR /app
COPY experiment/scripts/baseline-tracker.mjs .
RUN npm init -y > /dev/null 2>&1 && npm install mongodb@6
ENTRYPOINT ["node", "baseline-tracker.mjs"]
