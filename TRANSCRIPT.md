# Native transcript and recovery contract

Protocol 12 shares owned serde types between `daemon` and `frontend`.

## Durable versus live

- SQLite entries and their saved UI projections are authoritative.
- Provider deltas are transient, ordered live suffixes. A saved commit replaces the
  corresponding live suffix rather than duplicating the turn.
- First-observed timestamps belong to individual reasoning/text/tool sections. They
  do not all inherit the enclosing assistant message's timestamp.
- Commit precedes publication. Snapshots, history cuts, sequence numbers and live
  suffixes are ordered under the per-chat content boundary.
- History is indexed and paginated. Bounded hot transcripts do not cap saved history.

## Acceptance and uncertainty

A prompt acknowledgement follows an atomic queue/metadata/receipt commit. Moving a
queue prefix into user history atomically removes it from the queue. Queue edits,
deletes, pause/resume and abort have durable request receipts as well. A duplicate
request with identical payload reconciles its prior result; a reused ID with a new
payload or command kind is rejected. Receipt lookup precedes stale generation checks
so a successful pre-crash control remains reconcilable after restart.

Cancellation is signaled promptly, but a successful abort acknowledgement follows
its durable paused-state commit. A replayed old abort cannot cancel a newer run.
Interrupted provider/tool work is paused and marked uncertain; restart never repeats
unknown external side effects. An unfinished built-in also reports uncertainty
instead of replaying its effect.

The client keeps uncertain work visible and asks for deliberate resolution when no
receipt proves acceptance. A URL/token identity change cannot replay one account's
pending work against another daemon.

## Replay, forks, files

Provider replay reads only the applicable compaction checkpoint and retained suffix.
Fork/clone selects the logical prefix, including earlier applicable checkpoint
records even when their storage position follows a retained boundary. Children own
their copied entries and survive parent deletion. Unrelated/pending receipts and
queues are not inherited into a branch.

Attachments remain separate private files. Provider-only replay fields never enter
display events. Generated image bytes are decoded, validated and staged; transcripts
contain references, not duplicated base64 image payloads.

`taud --export-session ID PATH` takes a consistent read snapshot and writes a private
`tau-history` version-1 JSON document. It includes private provider replay material,
not credentials, live deltas, pending queue work or receipts. Treat it as private
conversation data, not a complete recovery backup.

The explicit `--import-state PATH` command reads Tau 1 metadata/Pi histories without
modifying their source and requires an empty destination database. There is no
parallel writable JSONL store and no automatic migration of unreleased native logs.
Use SQLite backup tooling for a running database; settings/auth/files are separate.

## Evidence

The release's nextest suite exercises real native provider fixtures, saved/live cuts,
queue controls, receipt reconciliation, tool/image delivery, file upload, restart,
compaction, forks and a child-process SIGKILL/WAL recovery scenario. Native frontend
integration connects through the actual daemon router, not a hand-written substitute.
See `INTEGRATION.md` and `frontend/QA.md` for release-specific acceptance scope.
