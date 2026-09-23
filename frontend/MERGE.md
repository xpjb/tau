# Integration status

The former merge contract is implemented in the `tau2` branch. The maintained
frontend branch is `tau2-rust-frontend` (local `tau2/rust-frontend`); keep it for
subsequent frontend work. Native backend and completed SQLite agent work were
merged in `4171a86`, followed by settings/control/native-tool release reconciliation.
No Kotlin/FFI/Pi runtime fallback is shipped on this branch.

The settings handoff `446ac29` and shared-editor/Sanscale work through `04fd60c`
were folded into the frontend branch first, then into `tau2` (local
`tau2-integration`). Both integrations are fast-forwards. All completed backend,
settings and editor histories are retained; no cherry-picks or source conflict
resolutions were necessary. Feature branches/worktrees are preserved and clean.

This integration changes documentation only beyond `04fd60c`; its recorded
64-test run remains the implementation evidence. No build/test rerun, packaging,
deployment, database migration or daemon restart was performed for the merge.
The merge itself did not ship a build; the subsequent authorized beta daemon and
Windows 0.7.1/protocol-13 delivery is recorded in ../INTEGRATION.md. SDK ligature limitations and physical
Windows/Android input acceptance remain open; see QA and backlog 004.

Current contracts and evidence live in:

- [Integration/release log](../INTEGRATION.md)
- [Architecture](../ARCHITECTURE.md)
- [Transcript contract](../TRANSCRIPT.md)
- [QA checklist](QA.md)

Beta deployment and Windows/Android builds were explicitly authorized. Stable remains
separate; there is no authorization here to cut stable over to the beta database.
