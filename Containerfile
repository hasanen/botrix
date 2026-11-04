# syntax=docker/dockerfile:1

# The builder and runtime images must share a Debian release: a binary linked
# against a newer glibc than the runtime provides fails to start. `rust:1-slim`
# (builder) is pinned to the same release as the runtime. Trixie is required:
# trunk's prebuilt wasm-bindgen needs glibc >= 2.38, which bookworm lacks.
ARG DEBIAN_RELEASE=trixie

# ── Stage 1: Build the WASM frontend ─────────────────────────────────────────
FROM rust:1-slim-${DEBIAN_RELEASE} AS frontend-builder
WORKDIR /build
RUN rustup target add wasm32-unknown-unknown \
    && cargo install trunk --locked
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
# Only the crates the frontend depends on are copied, and the server is dropped
# from this stage's workspace, so server-only changes do not invalidate this
# stage (it downloads large wasm tooling, so rebuilding it is slow).
RUN sed -i '/"crates\/botrix-server"/d' Cargo.toml
COPY crates/botrix-common crates/botrix-common
COPY crates/botrix-frontend crates/botrix-frontend
WORKDIR /build/crates/botrix-frontend
# Build, then keep only the output: the multi-GB target/ directory and cargo
# caches must not be committed into the layer.
RUN CARGO_INCREMENTAL=0 trunk build --release \
    && cp -r dist /frontend-dist \
    && rm -rf /build/target "${CARGO_HOME}/registry" dist

# ── Stage 2: Build the server binary ─────────────────────────────────────────
FROM rust:1-slim-${DEBIAN_RELEASE} AS server-builder
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ crates/
# The built frontend is embedded into the binary at compile time (rust-embed),
# so the runtime image needs no asset files.
COPY --from=frontend-builder /frontend-dist crates/botrix-frontend/dist
RUN CARGO_INCREMENTAL=0 cargo build --release --locked -p botrix-server \
    && cp target/release/botrix /botrix \
    && rm -rf target "${CARGO_HOME}/registry"

# ── Stage 3: Runtime image ────────────────────────────────────────────────────
FROM debian:${DEBIAN_RELEASE}-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN useradd -u 1000 -m -s /bin/sh botrix \
    && mkdir -p /data /config \
    && chown botrix:botrix /data /config
COPY --from=server-builder /botrix /usr/local/bin/botrix
USER botrix
ENV BOTRIX_DATA_DIR=/data \
    BOTRIX_CONFIG=/config/config.yaml
EXPOSE 8080
VOLUME ["/data", "/config"]
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["botrix", "healthcheck"]
ENTRYPOINT ["botrix"]
CMD ["serve"]
