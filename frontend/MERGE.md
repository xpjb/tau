# Integration status

The former merge contract is implemented in the `tau2` branch. The frontend snapshot
is preserved as `tau2-rust-frontend`; native backend and completed SQLite agent work
were merged in `4171a86`, followed by protocol-12 settings/control/native-tool release
reconciliation. No Kotlin/FFI/Pi runtime fallback is shipped on this branch.

Current contracts and evidence live in:

- [Integration/release log](../INTEGRATION.md)
- [Architecture](../ARCHITECTURE.md)
- [Transcript contract](../TRANSCRIPT.md)
- [QA checklist](QA.md)

Beta deployment and Windows/Android builds were explicitly authorized. Stable remains
separate; there is no authorization here to cut stable over to the beta database.
