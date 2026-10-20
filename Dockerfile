# Multi-stage build for Shinrai gateways (Rust 1.90).
#   docker build -t shinrai-trade .
#   docker build --target md-gateway -t shinrai-md-gateway .
#   docker build --target order-gateway -t shinrai-order-gateway .

FROM rust:1.90.0-bookworm AS builder
WORKDIR /app

# Cache dependency rebuilds when only sources change.
COPY Cargo.toml Cargo.lock rust-toolchain.toml rustfmt.toml ./
COPY crates ./crates

RUN cargo build --release -p shinrai-md-gateway -p shinrai-order-gateway \
    && strip target/release/shinrai-md-gateway target/release/shinrai-order-gateway

FROM debian:bookworm-slim AS runtime-base
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --uid 10001 shinrai
USER shinrai
WORKDIR /home/shinrai

FROM runtime-base AS md-gateway
COPY --from=builder /app/target/release/shinrai-md-gateway /usr/local/bin/shinrai-md-gateway
ENV SHINRAI_MD_BIND=0.0.0.0:8080
EXPOSE 8080
HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/health || exit 1
ENTRYPOINT ["shinrai-md-gateway"]

FROM runtime-base AS order-gateway
USER root
COPY deploy/docker/entrypoint-order-gateway.sh /usr/local/bin/entrypoint-order-gateway.sh
RUN chmod 755 /usr/local/bin/entrypoint-order-gateway.sh
COPY --from=builder /app/target/release/shinrai-order-gateway /usr/local/bin/shinrai-order-gateway
USER shinrai
ENV SHINRAI_OG_BIND=0.0.0.0:8081
EXPOSE 8081
HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8081/health || exit 1
ENTRYPOINT ["/usr/local/bin/entrypoint-order-gateway.sh"]

# Default image = order gateway (most common paper entrypoint).
FROM order-gateway
