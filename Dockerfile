# Stage 1: Build Web Frontend
FROM node:22-alpine AS web-builder
WORKDIR /app/web
COPY web/package*.json ./
RUN npm install
COPY web/ ./
RUN npm run build

# Stage 2: Build Rust Application
FROM rust:1-bookworm AS rust-builder
WORKDIR /app
COPY Cargo.toml ./
COPY crates/ ./crates/
COPY --from=web-builder /app/web/dist ./web/dist
RUN cargo build --release -p relating-server

# Stage 3: Runtime
FROM debian:bookworm-slim
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=rust-builder /app/target/release/relating-server /usr/local/bin/relating-server
COPY --from=web-builder /app/web/dist ./web/dist

ENV RELATING_PORT=8080
ENV RELATING_HOST=0.0.0.0
ENV RELATING_DATA_DIR=/app/data
ENV RELATING_DATABASE_URL=sqlite:///app/data/relating_metadata.db
ENV RELATING_LOG_LEVEL=info

VOLUME ["/app/data"]
EXPOSE 8080

CMD ["relating-server"]
