# Botrix developer task runner — requires https://just.systems

# Default: list available recipes
default:
    @just --list

# ── Development ──────────────────────────────────────────────────────────────

# Run the server in dev mode with auto-reload (requires cargo-watch)
dev:
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
run-container:
    podman run --rm -p 8080:8080 \
        -v ./dev-data:/data \
        -v ./dev-config:/config:ro \
        -e BOTRIX_SESSION_SECRET=dev-secret-change-me \
        -e BOTRIX_CRYPTO_PASSPHRASE=dev-passphrase-change-me \
        -e RUST_LOG=info \
        botrix:dev
