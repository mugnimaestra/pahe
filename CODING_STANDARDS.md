# Coding standards

## Structure

- The frontend reaches Rust through `src/ipc/client.ts` only. No other runtime
  frontend source file imports `@tauri-apps/*`; `src/ipc/boundary.test.ts`
  enforces this.
- Tauri commands are thin adapters. They translate arguments, call a domain
  module, and map its error to a plain-language string. Domain modules must build
  and test without a Tauri runtime (`src-tauri/src/settings.rs` is the model).
- IPC commands are `snake_case`; JSON crosses the boundary as `camelCase`
  (`#[serde(rename_all = "camelCase")]`).
- Use the vocabulary in `GLOSSARY.md` (Anime, Episode, Release, Session, Source,
  Quality, Download job, Library item, Base URL) in code, tests, and UI copy.
- No telemetry and no update checks. The app makes only the network requests its
  features need.

## Formatting and lints

- TypeScript is strict. No `any`, no non-null assertions to paper over a missing
  check, no `@ts-ignore` without a comment explaining why.
- TypeScript, CSS, JSON, and Markdown use two spaces. Rust uses `rustfmt`.
- These must pass before a commit: `pnpm typecheck`, `pnpm test`,
  `cargo fmt --check`, and `cargo clippy --all-targets -- -D warnings`.

## Tests

- Test behaviour at public seams, never private functions or call order.
  Current seams: the settings `Store` in Rust, the typed IPC client, and React
  views with the IPC client mocked.
- Frontend tests are colocated `*.test.ts(x)` files run by Vitest. DOM tests
  start with `// @vitest-environment jsdom`.
- UI tests mock `src/ipc/client.ts`, never `@tauri-apps/api`.
- Rust tests that touch the filesystem use `tempfile::TempDir`.
- New non-trivial logic (persistence, parsing, branching, money or security
  decisions) lands with a test that fails without it.

## Naming

- React components: `PascalCase` in a matching file (`SettingsView.tsx`).
  Functions and variables are `camelCase`; hooks start with `use`.
- Rust: modules and functions `snake_case`, types `PascalCase`, constants
  `SCREAMING_SNAKE_CASE`.
- Name things after the domain, not the framework (`settings`, not `commands`).

## Adding an IPC command

1. Implement and test the behaviour in a Rust domain module.
2. Add a thin `#[tauri::command]` and register it in `generate_handler!`.
3. Add a typed method to `src/ipc/client.ts`. UI code calls only that method.

## Commits

Conventional Commits, with the ticket reference: `feat: scaffold tauri app shell (#2)`.
