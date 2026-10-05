# Tau
Tau is a client-server agent harness. The server (daemon) runs on a remote host ie. tailnet. The client (frontend) runs on android or windows. Originally it was based on bridging into pi, but now the frontend, daemon and agent execution are all Rust.

The `tau2` mainline deploys to a beta identity separate from stable Tau.

## Build

Rust workspace members live under `crates/`. Run from the repository root:

```sh
cargo build --locked --workspace
cargo nextest run --locked --workspace
```

Windows launcher and installer crates use a separate workspace at
`crates/windows/Cargo.toml`. Linux packaging entry points are
`scripts/build-windows-sfx.sh` and `crates/frontend/android/build.sh`.

## Source and tests

Shared messages and native streams live in `crates/net`; SQLite block storage
lives in `crates/block-store`. Endpoint-specific networking belongs in each
application's `net` module, not in another transport crate.

Each crate keeps test code in `tests/`: public integration tests at the top level,
private unit tests in `tests/unit/`. Unit modules use a test-only `#[path]` from
the implementation they exercise, so testing does not widen production APIs.
Examples are diagnostic programs, not a second regression suite. Release-script
tests run with `python tests/release.py`; the Android harness is under
`crates/frontend/tests/android/` and requires a disposable emulator.

## Current beta features

Markdown rendering uses the external Barkdown repository, pinned to
`9bc7163193c5e1764e5c20a7119b5318673aa6b9`, including its delimiter-scanning fix.
The native-network refactor, client-first model selection, Codex device sign-in,
and encrypted reasoning checkpoint recovery are integrated on this mainline.

See [Codex sign-in](docs/codex-signin.md) and
[reasoning recovery](docs/codex-reasoning-recovery.md) for behavior and ownership.
`scripts/release-beta.sh` builds matching daemon, Windows and Android packages.
Client packages are delivered before deployment; restarting the beta service
requires explicit operator approval.
