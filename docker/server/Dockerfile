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
# Pinned (minor): bump together after verifying `npm ci && npm run build`.
FROM node:22.12-alpine AS web-builder
WORKDIR /app/web
COPY web/package*.json ./
RUN npm ci --no-audit --no-fund
COPY web/ ./
RUN npm run build

# ---- Stage 2: Rust workspace (needs BuildKit: `docker compose` v2) ----
# Pinned (minor): bump after verifying `cargo build --locked`.
FROM rust:1.90-bookworm AS rust-builder
WORKDIR /app
# Dependency manifests first so `cargo fetch` is cached until they change.
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/
COPY --from=web-builder /app/web/dist ./web/dist
# BuildKit cache mounts keep the cargo registry and the target dir across
# builds, so a code change recompiles incrementally (seconds) instead of
# rebuilding the whole workspace from scratch (many minutes).
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo fetch --locked
# NB: cache mounts do not persist into image layers, so the binaries are
# copied to /app/bin; the runtime stage copies from there.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build --locked --release -p relating-server \
    && cargo build --locked --release -p relating-cli --features postgres \
    && mkdir -p /app/bin \
    && cp /app/target/release/relating-server /app/target/release/relating /app/bin/

# ---- Stage 3: Runtime ----
FROM debian:bookworm-slim
WORKDIR /app
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --home-dir /app --shell /usr/sbin/nologin relating \
    && mkdir -p /app/data \
    && chown -R relating:relating /app
COPY --from=rust-builder /app/bin/relating-server /usr/local/bin/relating-server
COPY --from=rust-builder /app/bin/relating /usr/local/bin/relating
COPY --from=web-builder /app/web/dist ./web/dist
COPY docker/server/entrypoint.sh /usr/local/bin/relating-entrypoint.sh
RUN chmod +x /usr/local/bin/relating-entrypoint.sh \
    && chown -R relating:relating /app/web

ENV RELATING_PORT=8080
ENV RELATING_HOST=0.0.0.0
ENV RELATING_DATA_DIR=/app/data
ENV RELATING_DATABASE_URL=sqlite:///app/data/relating_metadata.db
ENV RELATING_LOG_LEVEL=info

VOLUME ["/app/data"]
EXPOSE 8080

# Never run as root: the API holds Aura credentials and executes migrations.
USER relating

ENTRYPOINT ["relating-entrypoint.sh"]
CMD ["relating-server"]
