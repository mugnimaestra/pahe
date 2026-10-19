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

CI builds unsigned bundles on macOS, Windows, and Ubuntu for every push, and
launches each build once against an empty app store (cold start smoke). Pushing
a `v*` tag runs the release workflow, which builds the same three bundles,
smoke-tests them, and attaches the unsigned artifacts to a GitHub Release.
Cross-platform verification that CI cannot do — Challenge, download, resume,
playback — is the [manual smoke checklist](docs/manual-smoke-checklist.md).
Conventions live in [CODING_STANDARDS.md](CODING_STANDARDS.md).
