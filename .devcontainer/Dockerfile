FROM rust:1-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev libsqlite3-dev sqlite3 git curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# WASM target
RUN rustup target add wasm32-unknown-unknown

# Dev tools
RUN cargo install trunk cargo-watch cargo-deny sqlx-cli --locked

# Tailwind CSS v4 standalone CLI (no Node.js required)
RUN curl --proto '=https' --tlsv1.2 -fsSL \
    https://github.com/tailwindlabs/tailwindcss/releases/latest/download/tailwindcss-linux-x64 \
    -o /usr/local/bin/tailwindcss && chmod +x /usr/local/bin/tailwindcss

# just task runner
RUN curl --proto '=https' --tlsv1.2 -sSf \
    https://just.systems/install.sh | bash -s -- --to /usr/local/bin

RUN useradd -m container
USER container
