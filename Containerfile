# syntax=docker/dockerfile:1

# ── Stage 1: Build the WASM frontend ─────────────────────────────────────────
FROM rust:1-slim AS frontend-builder
WORKDIR /build
RUN rustup target add wasm32-unknown-unknown \
    && cargo install trunk --locked
# Tailwind CSS v4 standalone CLI (no Node.js required)
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates \
    && curl --proto '=https' --tlsv1.2 -fsSL \
       https://github.com/tailwindlabs/tailwindcss/releases/latest/download/tailwindcss-linux-x64 \
       -o /usr/local/bin/tailwindcss \
    && chmod +x /usr/local/bin/tailwindcss \
    && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ crates/
WORKDIR /build/crates/botrix-frontend
RUN trunk build --release

# ── Stage 2: Build the server binary ─────────────────────────────────────────
FROM rust:1-slim AS server-builder
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev libsqlite3-dev \
    && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ crates/
# Copy the built frontend assets so rust-embed can embed them at compile time.
COPY --from=frontend-builder /build/crates/botrix-frontend/dist \
    crates/botrix-frontend/dist
RUN cargo build --release --locked -p botrix-server

# ── Stage 3: Runtime image ────────────────────────────────────────────────────
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/*
RUN useradd -u 1000 -m -s /bin/sh botrix
COPY --from=server-builder /build/target/release/botrix /usr/local/bin/botrix
USER botrix
EXPOSE 8080
VOLUME ["/data", "/config"]
ENTRYPOINT ["botrix"]
CMD ["serve"]
