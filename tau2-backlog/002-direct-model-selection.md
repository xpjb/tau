# 002 — Select a model without a catalog gate

Priority: after/alongside 001. Status: in progress.

The user wants exact provider/model IDs sent to the provider. A local list must
never disable a quick-model tile or reject `/model`. Do not silently substitute
GPT-5.6 for GPT-6 or v4 for v4.1. Provider configuration/auth is still required.

Confirmed: favorites in `frontend/src/models.rs` are disabled by `resolve`; daemon
`SettingsExt::model` also rejects IDs outside `settings.models`. Luna/Sol were in
Tau 1's separate `models.json`, which the importer missed. Flash's preset differs
from the imported hints. Those facts are not provider availability checks.

Use optional metadata for names/context/thinking/prompts and optional suggestions.
Unknown capacity stays unknown; skip threshold-based auto-compaction in that case.
Test an ID absent from metadata reaching the HTTP provider unchanged, the actual
provider error reaching the chat, no retries of a 400 rejection, and persistence of
explicit model choice without draft loss. Quick-model search is convenience only.
