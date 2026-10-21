# orch-status

The `statusLine` command of every agent (SPEC §6.3, FR-801). The per-agent `--settings` statusLine
**replaces** the user's own (P0), so orch-status:

1. forwards the statusline JSON to the core (`{"type":"status"}` on the bus, ≤ 1 s connect, no wait
   for an answer, best effort);
2. runs the user's own statusline command (`ORCH_USER_STATUSLINE`, read by the core from
   `~/.claude/settings.json`) through `sh -c` with the same stdin, and prints its output;
3. without one, prints `ORCH 5h 7% · 7d 83% · ctx 12%` (`?` for an unknown window).

Claude Code runs it every 5 s while idle (`refreshInterval: 5`); the core's usage collector ignores
re-renders that carry no new API response.

Tests: `cargo test -p orch-status` (forward + chain, core down, default line).
