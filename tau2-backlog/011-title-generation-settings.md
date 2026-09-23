# 011 — Title generation model and prompt in daemon settings

Priority: with 001. Status: in progress.

Tau 2 exposes the title prompt and generation toggle but always uses the chat's
model. Add an optional explicit title provider/model. Unset follows the chat model;
set sends that exact ID without a metadata membership check. Keep the existing
native title path, bounded prompt/time, no tools, prompt-ack independence, safe
fallback and manual-name protection. Do not resurrect the Python/Pi title helper.

Acceptance: independent title model and exact template through real local provider
HTTP, settings save/reload/restart, unset fallback, failed generation fallback and
manual rename while title generation is in flight. Document optional extra billing.
