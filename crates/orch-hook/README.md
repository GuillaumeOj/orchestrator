# orch-hook

The command every hook of every agent runs (SPEC §6.3, FR-806, ADR-009 decision 4). It reads the
hook JSON on stdin, sends it to the core over the bus ([orch-protocol](../orch-protocol/README.md))
and prints the core's answer.

Claude Code **fails open** on hook timeouts, crashes (exit 1) and a missing hook binary (P0), so
orch-hook always answers itself:

| Event | Core reachable | Core unreachable (missing/refused socket, EOF, deadline, bad reply) |
|---|---|---|
| PreToolUse | waits up to `ORCH_HOOK_DEADLINE_MS` (default 540 s, under the 600 s hook timeout) and prints the core's decision | JSON **deny** `ORCH: orchestrator unavailable…`, exit 0, spooled |
| PermissionRequest | same | JSON **deny** (`decision.behavior`), exit 0, spooled |
| every other event | waits at most 1 s for the answer (e.g. additional context) | **no output, exit 0**, spooled to `$ORCH_AGENT_DIR/spool.jsonl` |

- Connect timeout ≤ 1 s. A missing socket (`ENOENT`) and a stale one (`ECONNREFUSED`) fail at once.
- Never exits 2 (that blocks and erases the user's prompt) and never 1.
- Missing `ORCH_SOCK`/`ORCH_AGENT_ID` is treated as "core unreachable".
- Environment: `ORCH_AGENT_ID`, `ORCH_SOCK`, `ORCH_AGENT_DIR`, `ORCH_HOOK_DEADLINE_MS` (set by the core).
- The binary lives at a stable app-data path (`<app>/bin/orch-hook`), checked at every spawn.

Tests (`cargo test -p orch-hook`): core absent (every event), stale socket, missing env, EOF while
held, deadline (tool events deny before the deadline; non-tool events return within ~1 s on a hung
core), core answer printed, empty answer, protocol version mismatch, garbage input.

## Readings

- "≤ 1 s connect timeout" for non-tool events is read as ≤ 1 s for connect **and** reply, so a hung
  core never delays a prompt by more than a second (P0 A2 saw 5 s per event with a hung core).
