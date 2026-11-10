# Botrix developer task runner — requires https://just.systems

# Default: list available recipes
default:
    @just --list

# ── Development ──────────────────────────────────────────────────────────────

# Create .env from the example if it is missing
env:
    @[ -f .env ] || cp .env.example .env

# Start the devcontainer
up: env
    devcontainer up --workspace-folder .

# Open a shell in the devcontainer
shell: up
    devcontainer exec --workspace-folder . bash

# Run the server in the devcontainer (loads .env, uses dev-config/ and dev-data/)
dev: up
    devcontainer exec --workspace-folder . sh -c 'set -a; . ./.env; set +a; \
        mkdir -p dev-config dev-data; \
        [ -f dev-config/config.yaml ] || cp config.example.yaml dev-config/config.yaml; \
        export CARGO_TARGET_DIR=/home/vscode/target \
            BOTRIX_CONFIG=$PWD/dev-config/config.yaml \
            BOTRIX_DATA_DIR=$PWD/dev-data \
            BOTRIX_FRONTEND_DIR=$PWD/crates/botrix-frontend/dist; \
        scripts/install-trunk.sh; \
        (cd crates/botrix-frontend && trunk build); \
        exec cargo run -p botrix-server -- serve'

# Run the server with auto-reload (requires cargo-watch)
watch:
    cargo watch -x 'run --bin botrix -- serve'

# Build and watch the WASM frontend with live reload (requires trunk)
frontend-dev:
    cd crates/botrix-frontend && trunk serve

# ── Building ─────────────────────────────────────────────────────────────────

# Build the WASM frontend for production
frontend:
    cd crates/botrix-frontend && trunk build --release

# Build the server binary
build:
    cargo build --release --locked -p botrix-server

# ── Testing ───────────────────────────────────────────────────────────────────

# Run all tests
test:
    cargo test --locked

# ── Code quality ─────────────────────────────────────────────────────────────

# Format all Rust source files
fmt:
    cargo fmt

# Check formatting without modifying files
fmt-check:
    cargo fmt --check

# Run Clippy linter (deny warnings)
lint:
    cargo clippy --all-targets --all-features -- -D warnings

# Run license and security audit
deny:
    cargo deny check

# ── Database ──────────────────────────────────────────────────────────────────

# Run pending database migrations against the dev database
db-migrate:
    sqlx migrate run --database-url sqlite://dev-data/botrix.db \
        --source crates/botrix-server/migrations

# ── Configuration ─────────────────────────────────────────────────────────────

# Validate the dev configuration file
validate-config:
    cargo run --bin botrix -- config validate \
        --config dev-config/config.yaml

# ── Containers ────────────────────────────────────────────────────────────────

# Build the production container image
build-container:
    podman build -t botrix:dev -f Containerfile .

# Run the container locally (requires dev-data/ and dev-config/)
run-container: env
    podman run --rm -p 8080:8080 \
        -v ./dev-data:/data \
        -v ./dev-config:/config:ro \
        --env-file .env \
        botrix:dev
