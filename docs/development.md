---
layout: default
title: Development
nav_order: 7
---

# Development

Code runs in the devcontainer (Rust toolchain, `wasm32` target). The host needs [`just`](https://just.systems) (`brew install just`), Podman and the `devcontainer` CLI. Run `just` to list all recipes.

## Start the server

```bash
just dev
```

This creates `.env` from `.env.example` if missing, starts the devcontainer, creates `dev-config/config.yaml` and `dev-data/` on first run, installs Trunk, builds the frontend, and runs the server. Open <http://localhost:8080>. The first start takes several minutes. Stop with Ctrl-C and rerun after code changes (frontend included).

- `.env` holds the dev secrets and settings. It, `dev-config/` and `dev-data/` are git-ignored. Edit `dev-config/config.yaml` to change the config.
- The devcontainer publishes container port 8080 on host port `DEV_PORT` (default 8080). If the port is taken, run `DEV_PORT=8081 just dev` and open that port instead.
- The port is fixed when the container is created. A container made before this was added needs a one-time recreate: `devcontainer up --workspace-folder . --remove-existing-container`. This discards container-local state such as the build cache and installed tools (Trunk is reinstalled by `just dev`).

## Shell in the devcontainer

```bash
just shell
```

If builds are killed (out of memory), set `CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0`, or give the Podman VM more RAM with `podman machine set --memory 8192`. Set `CARGO_TARGET_DIR=/home/vscode/target` in the shell to keep Linux build output out of the host `target/`.

## Checks (same as CI)

Run inside `just shell`:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
cargo check -p botrix-frontend --target wasm32-unknown-unknown --locked
```

Scope to one crate (`-p botrix-frontend`, `-p botrix-server`) to go faster.

## Run the container image (full UI)

Builds frontend and server, serves on <http://localhost:8080>:

```bash
just build-container
just run-container
```

Uses `.env`, `dev-config/` and `dev-data/`. Connecting a real Matrix account needs an HTTPS `public_url`; see [Matrix setup](matrix-setup.md).

## Layout

- `crates/botrix-common`: config and API types shared by server and frontend
- `crates/botrix-server`: Axum server, SQLite migrations, Matrix client, delivery worker
- `crates/botrix-frontend`: Leptos WASM UI, built with Trunk (`dist/` is embedded into the server binary)

## Conventions

- Write a failing test first, then the change.
- Add a `CHANGELOG.md` entry for user-visible changes.
- Run `cargo deny check` after changing dependencies.
