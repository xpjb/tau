# Tau

Tau is a private, Tailnet-native client for independent Pi coding-agent sessions.
It runs beside the existing Telegram gateway without sharing processes or state.

The project contains:

- `daemon/`: `taud`, the Linux service that owns Tau's Pi RPC processes and session files.
- `app/`: one Compose Multiplatform client for Android and desktop JVM targets.
- `windows/`: the portable launcher and version-aware self-extracting Windows package.

The first release uses `/root` as every Pi process working directory. Tau sessions are
stored separately from Telegram sessions.
