# 010 — Account usage remaining in context hover

Priority: context hover. Status: open.

Tau 1 showed remaining usage percentages in the context hover; Tau 2 does not.
Inspect Tau 1's Codex usage extension and tooltip contract. Keep account quota
separate from context-token capacity and the estimated prompt-cache timer.

Use a native read-only provider usage path with bounded refresh and error/staleness
states, never a billed model request or imported Pi extension runtime. Display
available remaining percentages/reset times and clear unsupported/unavailable
states for other providers. Preserve private credentials. Acceptance includes
missing quotas, changed windows, expiry/errors and actual hover/pinned display.
