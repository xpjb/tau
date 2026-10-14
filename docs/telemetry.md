# Desktop telemetry

Tau 2 desktop clients use `living-telemetry` to record slow frames, blocked UI callbacks, Rust panics, native Windows crashes, and unclean exits. Reports use the same Tailnet-only collector as Compendium and remain in a bounded local spool while the collector is unavailable.

The normal report state contains connection state, counts, durations, frame phases, and native transport counters. It does not contain prompts, responses, chat titles, message text, credentials, clipboard data, or full paths. Windows minidumps and panic details can contain process data and stay inside the trusted Tailnet collector.

Set `TAU_TELEMETRY_ENDPOINT=off` to disable uploads. Set it to an `http://` report endpoint to use another collector. The default is `http://vibe.tail8689b0.ts.net:4318/v1/report`.

Android does not start the same-executable watchdog and is outside this change.
