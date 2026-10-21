# Desktop telemetry

Tau 2 desktop clients use `living-telemetry` to record slow frames, blocked UI callbacks, Rust panics, native Windows crashes, and unclean exits. Reports use the same Tailnet-only collector as Compendium and remain in a bounded local spool while the collector is unavailable. Frame and UI-callback timing is active only while the desktop window is focused and visible. Focus loss, Alt-Tab, minimization, and occlusion invalidate open timing intervals; foreground timing resumes from a clean baseline instead of reporting operating-system background throttling as an app stall.

The normal report state contains connection state, counts, durations, frame phases, native transport counters, and selected-chat size/state counts. Chat navigation records use a hashed content-free chat key so repeated selections can be correlated. Slow navigation is split into controller polling, model selection, transcript binding, layout reconciliation, window measurement, and visible-row mounting phases. Reports do not contain raw chat IDs, prompts, responses, chat titles, message text, credentials, clipboard data, provider names, or full paths. Windows minidumps and panic details can contain process data and stay inside the trusted Tailnet collector.

Set `TAU_TELEMETRY_ENDPOINT=off` to disable uploads. Set it to an `http://` report endpoint to use another collector. The default is `http://vibe.tail8689b0.ts.net:4318/v1/report`.

Android does not start the same-executable watchdog and is outside this change.


## Watchdog backlog hotfix (0.7.22)

The shared client is pinned to `59e5fcf`. It measures UI inactivity inside the
sampled heartbeat, not the time spent queued in IPC/spool work. Routine callback
wake-ups are coalesced to 100 ms, and offline spool budgeting caches the small
immutable envelope index. Old installed clients need replacement/restart; a
collector update alone cannot change their watchdogs.

`TAU_TELEMETRY_ENDPOINT=off` (also empty, `0`, or `none`, case-insensitive) now
returns before client initialization: no local recorder, reporter, watchdog,
spool or upload. Previously it disabled upload only. Restart after changing it.

Automatic hang dumps have two attempts/run with a ten-minute cooldown. Additional
hangs still send lightweight session/sequence deltas with capture status; failures
consume the dump budget too. Explicit repro and fatal-crash reports retain full
bounded context. Checkpoints retain 256 recent records, every 15 seconds busy or
30 seconds idle. These changes do not alter daemon protocol or deploy a daemon.
