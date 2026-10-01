# Stop/Play must not be blocked by a deferred queue action

October 1, 2026. Feature branch `fix/tau2-control-recovery`, starting at `40ac1b0`.
Implementation: `6422e27`. User authorized commit, push and source merge, with a
separate [open UX/control-state backlog](../tau2-backlog/045-play-pause-control-ux-debt.md).
No deployment, version/protocol/schema change, service restart or live-data edits.

## Failure

A pending `QueueOperation::Prefix` ("run through here") or Pause was stored as
`queue.control.status = "waiting"`. Abort durably held the queue and cancelled the
agent, but did not retire that control. Run settlement cleared the run ID without
settling the waiting control. Resume's guard then refused with
`Cancel the pending queue control first`. This is a logical control lockout,
reproduced with the actual authenticated socket and scripted local provider.

A second reproduction seeds an old idle, paused chat with a waiting action owned
by an ended run. Play alone must recover it; users must not need database surgery
or a special cancellation sequence. A run ID from the frame before Stop settled
must not become another blocker once no live run exists.

## Changed contract

- **Stop** saves the held queue and clears its boundary action in the same durable
  transaction, then cancels active work. It preserves every queued message. Stop
  does not silently start another provider request.
- **Play** explicitly unpauses the queue and clears any old action. It accepts an
  ended run ID when no current run exists, but still rejects a different live run
  or stale lineage. The remaining queue is consumed normally.
- **New valid Pause/Prefix/Resume intents supersede older boundary intents.**
  Prefix membership/revisions, supported boundaries and generation fences still
  validate before any state or receipt changes. Editing a selected pending prefix
  still requires cancelling/replacing that limit.
- **Stopping-task ordering** extends the existing resume-after-stop mechanism to
  Prefix. Accepted continuation waits until old cleanup/tools finish. A later
  Pause or Stop wins; old receipt retries cannot resurrect the previous intent.
- **Run failure** holds the queue and settles an unapplied boundary action as
  failed with a bounded reason. It never starts automatic recovery. If storage
  cannot commit settlement, a later explicit Play can still replace a persisted
  waiting action once storage is usable.
- **Restore review** applies to Prefix as well as Resume; run-through must not
  bypass the guard for starting execution on restored history.

UI text now says "Run through here, then pause", names "Cancel run limit" versus
"Cancel pause", and distinguishes pending/finishing run-through from generic
Working. An active compaction/capacity detail and failure detail keep priority.
The cancellation row and Play are exercised on a phone-sized retained layout.

**Limit:** run-through still applies at the existing provider/tool-batch boundary.
This patch does not make it interrupt generation sooner and does not settle the
user's wider complaint about its usefulness or historical Stop→drain behavior.
That product/ownership review remains open in 045.

## Validation evidence

Managed Cargo and nextest only; isolated temporary databases and loopback providers.
No real provider request, production mutation or physical-device acceptance.

- **Failing before:** both new lockout reproductions fail at the old waiting-control
  guard/Stop state (`1f70f8c4-259d-4d8e-974e-849434fba2bb`,
  `/tmp/tau2-control-recovery-before.log`).
- **Passing after:** all 11 control/restore-review cases
  (`9ccaa085-0a71-4808-9e60-cf5fb904ae90`,
  `/tmp/tau2-control-recovery-controls.log`). Covers actual socket Stop→Play,
  already-stuck state, supersession, stale input/receipt fences, prefix/tail
  boundaries, a later Pause during cleanup, and provider failure without retry.
- **Remaining daemon coverage:** all other 55 cases across daemon library/binary
  and SIGKILL/WAL integration binaries
  (`03e75952-7d6b-4238-a3cc-12848103f8f2`,
  `/tmp/tau2-control-recovery-daemon.log`). Combined with the disjoint selection
  above, **66/66 daemon cases pass**, including disk-full and transactional
  rollback, interrupted effects, compaction, restart and restore safety.
- **Frontend control layout:** the existing retained-control regression, extended
  to named mobile cancellation and visible Play, passes
  (`55f8fb11-4fbb-4bdb-a970-021f6b75a95e`,
  `/tmp/tau2-control-recovery-ui.log`). One unrelated existing dead-code warning
  in `daemon_settings::Draft::identity`; no lint-driven rewrite.

Native all-target compilation and a real-controller queue end-to-end check will
be recorded against the final source integration. Do not rerun the complete
workspace/device/package suites merely for this bounded fix.
