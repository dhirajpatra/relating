# Relating server image.
#
# Build context: repository root (needs Cargo.toml, Cargo.lock, crates/, web/).
#   docker build -f docker/server/Dockerfile -t relating-server .
#
# Produces two binaries:
#   relating-server  the Axum API + workbench UI + MCP HTTP transport
#   relating         the CLI (built with --features postgres so the db-init
#                    compose job can provision the shared Postgres schema)

# ---- Stage 1: Web frontend ----
FROM node:22-alpine AS web-builder
WORKDIR /app/web
COPY web/package*.json ./
RUN npm ci --no-audit --no-fund
COPY web/ ./
RUN npm run build

# ---- Stage 2: Rust workspace ----
FROM rust:1-bookworm AS rust-builder
WORKDIR /app
# Dependency manifests first so `cargo fetch` is cached until they change.
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/
COPY --from=web-builder /app/web/dist ./web/dist
RUN cargo fetch --locked
RUN cargo build --locked --release -p relating-server \
    && cargo build --locked --release -p relating-cli --features postgres

# ---- Stage 3: Runtime ----
FROM debian:bookworm-slim
WORKDIR /app
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
COPY --from=rust-builder /app/target/release/relating-server /usr/local/bin/relating-server
COPY --from=rust-builder /app/target/release/relating /usr/local/bin/relating
COPY --from=web-builder /app/web/dist ./web/dist
COPY docker/server/entrypoint.sh /usr/local/bin/relating-entrypoint.sh
RUN chmod +x /usr/local/bin/relating-entrypoint.sh

ENV RELATING_PORT=8080
ENV RELATING_HOST=0.0.0.0
ENV RELATING_DATA_DIR=/app/data
ENV RELATING_DATABASE_URL=sqlite:///app/data/relating_metadata.db
ENV RELATING_LOG_LEVEL=info

VOLUME ["/app/data"]
EXPOSE 8080

ENTRYPOINT ["relating-entrypoint.sh"]
CMD ["relating-server"]
