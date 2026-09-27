# 010 — Account usage remaining in context hover

Priority: context hover. Status: shipped in Tau2 beta 0.7.6 / protocol 19.
Live provider and device acceptance remains pending.

Tau 1 showed remaining usage percentages in the context hover; Tau 2 does not.
Inspect Tau 1's Codex usage extension and tooltip contract. Keep account quota
separate from context-token capacity and the estimated prompt-cache timer.

Use a native read-only provider usage path with bounded refresh and error/staleness
states, never a billed model request or imported Pi extension runtime. Display
available remaining percentages/reset times and clear unsupported/unavailable
states for other providers. Preserve private credentials. Acceptance includes
missing quotas, changed windows, expiry/errors and actual hover/pinned display.

Implementation: the authenticated `get_codex_usage` control reads the daemon's
Codex OAuth account through a fixed, read-only WHAM endpoint (no Pi extension
or model prompt). A bounded in-memory, credential-scoped cache throttles regular
and manual refreshes. The context indicator still measures context tokens; its
hover/pinned card separately shows account quota windows, plan, reset countdown,
last-known/error/empty/offline states and a manual Refresh button. Other providers
show an explicit unsupported quota state. No credential or raw provider payload
crosses the client wire. Local mock-HTTP, authenticated WebSocket and headless
GPU UI tests cover the path; a live provider/device check is still pending.
