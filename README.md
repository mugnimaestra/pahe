# pahe

Tauri desktop client for animepahe — search, download, and watch.

## Development

Prerequisites: Node 24, pnpm 11, Rust stable, and the
[Tauri system dependencies](https://tauri.app/start/prerequisites/) for your OS.

```sh
pnpm install
pnpm tauri dev
```

Checks:

```sh
pnpm typecheck
pnpm test
cd src-tauri && cargo test && cargo clippy --all-targets -- -D warnings
```

CI builds unsigned bundles on macOS, Windows, and Ubuntu for every push.
Conventions live in [CODING_STANDARDS.md](CODING_STANDARDS.md).
