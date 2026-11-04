# syntax=docker/dockerfile:1

# The builder and runtime images must share a Debian release: a binary linked
# against a newer glibc than the runtime provides fails to start. `rust:1-slim`
# (builder) is pinned to the same release as the runtime. Trixie is required:
# trunk's prebuilt wasm-bindgen needs glibc >= 2.38, which bookworm lacks.
ARG DEBIAN_RELEASE=trixie

# ── Stage 1: Build the WASM frontend ─────────────────────────────────────────
FROM rust:1-slim-${DEBIAN_RELEASE} AS frontend-builder
WORKDIR /build
RUN rustup target add wasm32-unknown-unknown

# Trunk is installed from its pinned, checksum-verified release binary instead
# of `cargo install` (which compiled it from source: ~4.5 minutes per cold
# build, and an unpinned version). To upgrade, bump TRUNK_VERSION and both
# checksums (the .sha256 files published with the release, which also match
# the digest GitHub shows for each asset). Static musl builds are used so they
# do not depend on the image's glibc.
ARG TARGETARCH
ARG TRUNK_VERSION=0.22.1
ARG TRUNK_SHA256_AMD64=a1fe5cd6b053f5a76f66b598d296022a7f06f56def6013cc16cd6897d53f5363
ARG TRUNK_SHA256_ARM64=8c84a939aaf48bbdac2d78195c932adb319e610fb2cbf791bae07b2660cf3e46
RUN set -eux; \
    case "${TARGETARCH}" in \
      amd64) arch=x86_64;  sha="${TRUNK_SHA256_AMD64}" ;; \
      arm64) arch=aarch64; sha="${TRUNK_SHA256_ARM64}" ;; \
      *) echo "unsupported architecture: ${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    apt-get update; \
    apt-get install -y --no-install-recommends curl ca-certificates; \
    curl --proto '=https' --tlsv1.2 -fsSL -o /tmp/trunk.tar.gz \
      "https://github.com/trunk-rs/trunk/releases/download/v${TRUNK_VERSION}/trunk-${arch}-unknown-linux-musl.tar.gz"; \
    echo "${sha}  /tmp/trunk.tar.gz" | sha256sum -c -; \
    tar -xzf /tmp/trunk.tar.gz -C /usr/local/bin trunk; \
    rm -f /tmp/trunk.tar.gz; \
    rm -rf /var/lib/apt/lists/*; \
    trunk --version
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
