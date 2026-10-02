# Tau
Tau is a client-server agent harness. The server (daemon) runs on a remote host ie. tailnet. The client (frontend) runs on android or windows. Originally it was based on bridging into pi, but now the frontend, daemon and agent execution are all Rust.

This is a beta branch, it is deplyoed to a beta identity which is separate from stable Tau.

## Build

Rust workspace members live under `crates/`. Run from the repository root:

```sh
cargo build --locked --workspace
cargo nextest run --locked --workspace
```

Windows launcher and installer crates use a separate workspace at
`crates/windows/Cargo.toml`. Linux packaging entry points are
`scripts/build-windows-sfx.sh` and `crates/frontend/android/build.sh`.
