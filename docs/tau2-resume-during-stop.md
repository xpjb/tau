# Resume while stopping — isolated bug fix

Branch: `fix/tau2-resume-during-stop`, based on integration `415aeff`.
This fix is independent of the retained-UI rewrite. No live daemon or client was
changed, no database writes were made during diagnosis, and no restart is part
of this change.

Abort durably pauses the queue and cancels the running task. Previously, Resume
could durably unpause that queue while the cancelled task was still cleaning up;
`start_run` correctly refused a concurrent run, but cleanup then paused it again.
The accepted Resume was lost. The regression reproduces this with the actual
provider/agent path and a test-only cleanup gate; it failed before the fix.

An accepted Resume now records an in-memory continuation intent until cleanup
finishes. Only then can a new run start. A subsequent stop, close, deletion or
shutdown clears that intent through `AgentSession::stop`. Failed settlement does
not restart work. Duplicate immutable Resume IDs remain no-ops. No protocol or
persisted schema changes; crash recovery still requires explicit review/resume.
This does not automatically retry provider failures or remove safety pauses.

Read-only diagnosis also found historical incomplete/provider stream failures;
those are a separate possible reason for repeated pauses. The reported incident
cannot be conclusively attributed to this race from the available metadata.

Validation: regression fails before the fix (run
`86cd12c0-3cb7-4b21-9283-a895cfd86c07`); all 32 agent tests pass after the fix
(`672e78cb-b23d-4573-a36e-9254e7ca577b`), including later-stop precedence,
idempotent retries, interrupted tools, recovery safety and real socket controls.
Full daemon nextest: 58/58 passed, zero skipped
(`77d8c66d-0404-4769-a8e9-fe0d3f358a1f`).

Integration follow-up: merged independently as `b52f80e` alongside the retained UI
merge `7ab80f3`, after the user's integration request. The combined workspace
passes 308/308 nextest cases (`ac0efec3-a0dc-4d7b-b238-3e371e7d693d`). No deployment,
restart, package build, production data changes or automatic live Resume occurred.
