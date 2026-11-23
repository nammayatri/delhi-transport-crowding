# Crowding pipeline, service and web app in one image.
FROM rust:1.95-bookworm AS rust
WORKDIR /src
COPY crowding ./crowding
RUN cd crowding && cargo build --release -p crowd-pipeline -p crowd-service -p crowd-replay

FROM node:22-bookworm-slim AS web
WORKDIR /web
COPY web/package.json web/pnpm-lock.yaml* web/package-lock.json* ./
RUN npm install --no-audit --no-fund
COPY web ./
RUN npx vite build

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/*
COPY --from=rust /src/crowding/target/release/crowd-pipeline /src/crowding/target/release/crowd-service /src/crowding/target/release/crowd-replay /usr/local/bin/
COPY crowding/config /app/config
COPY --from=web /web/dist /app/web
WORKDIR /app
ENV DHALL_CONFIG=/app/config/service.dhall \
    CROWD_SNAPSHOT=/data/out/full/snapshot.msgpack.zst \
    CROWD_WEB_DIR=/app/web \
    RUST_LOG=info
EXPOSE 8090
HEALTHCHECK CMD curl -sf localhost:8090/health || exit 1
CMD ["crowd-service"]
