# Tau release process

The `tau2` branch ships the isolated **Tau Beta** install identity. “Beta” names the
installed channel: its clients, service, ports and state remain separate from stable Tau.

## Commands

- `scripts/build-windows.sh` builds and verifies the Windows x64 installer.
- `scripts/build-android.sh` builds, signs and verifies one Android ABI.
- `scripts/release-clients.sh` bumps the version, commits, calls those two builds in
  Windows-then-Android order, pushes `tau2`, and prepares or performs delivery.
- `scripts/deploy-daemon.sh` builds and deploys the daemon. It is always a separate,
  explicitly approved final operation.

Final packages and SHA-256 files are written under `dist/`. Release logs, fingerprints
and delivery receipts are written under `dist/releases/VERSION/`.

## Compiler checks

Use the managed Cargo wrapper. A plain Windows target check does not configure the
MSVC SDK and is not a valid failure:

```sh
/usr/local/bin/cargo check --locked --workspace --all-targets
/usr/local/bin/cargo check-windows
/usr/local/bin/cargo nextest run --locked --workspace
```

`cargo check-windows` is the repository alias for the required `cargo xwin check` path.
Android release compilation is performed by `scripts/build-android.sh` with the NDK
compiler, archiver and linker configuration.

## Client release

Start from a clean `tau2` branch that already contains the intended work. The release
command does not merge feature branches or run tests implicitly.

```sh
scripts/release-clients.sh --plan --version 0.7.21
scripts/release-clients.sh --version 0.7.21
```

The command performs this fixed order:

1. Validate that local `tau2` contains `origin/tau2`.
2. Bump shared package metadata and Android version code, then commit.
3. Build and verify Windows.
4. Build, sign and verify Android ARM64.
5. Push `tau2`.
6. Produce `delivery.json`, then deliver Windows before Android when a sender is supplied.
7. Stop. No daemon binary or service is touched.

When the protocol is unchanged, the released clients may continue using the current
matching-protocol daemon. Daemon deployment remains a separate decision.

## Daemon deployment

After explicit approval, run:

```sh
scripts/deploy-daemon.sh
```

This builds the daemon last, verifies that the source commit is published, preserves
stable Tau, installs only the isolated Tau Beta service, and checks the running binary,
version, protocol and health response.

The command may be invoked from a Tau session served by the daemon being replaced. In
that case it first moves itself into a transient systemd scope outside
`tau2-beta.service`, so stopping the service cannot kill the build or installer. After
starting this command, **pause and issue no further work**. The current connection can
reset during restart. Reconnect afterward and inspect the reported health instead of
starting another deployment.
