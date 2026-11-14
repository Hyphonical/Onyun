# syntax=docker/dockerfile:1

FROM oven/bun:1-alpine AS web-builder
WORKDIR /app/web
COPY web/package.json web/bun.lock ./
RUN bun install --frozen-lockfile
COPY web/ ./
RUN bun run build

FROM rust:1-bookworm AS rust-builder
WORKDIR /app
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY --from=web-builder /app/web/dist ./web/dist
RUN cargo build --locked --release --no-default-features --features web,gpu

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates libvulkan1 libx11-6 libxext6 \
        libglvnd0 libglx0 libegl1 libxcb1 libxau6 libxdmcp6 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 onyun \
    && mkdir -p /data/mints \
    && chown -R onyun:onyun /data

COPY --from=rust-builder /app/target/release/onyun /usr/local/bin/onyun

WORKDIR /data
VOLUME ["/data/mints"]
EXPOSE 8080

ENTRYPOINT ["onyun"]
CMD ["ui", "--web", "--bind", "0.0.0.0", "--port", "8080", "--out", "/data/mints"]