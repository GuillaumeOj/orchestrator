# Improve Agent Usage · Feature spec

> Feature `improve-agent-usage` · Integration branch `orch/improve-agent-usage/integration` · Core (agent-runtime) + one timeline label in the UI · No design pass

## 1. Problem

When a node gets several tasks in a feature (e.g. `back` on T-1, then T-3, then T-5), `dispatch` (`crates/agent-runtime/src/team.rs:587`) hands each new task to the node's **free agent** (`back-1`) and delivers the task turn into **the same Claude session** (`session_id` set once in `add_team_agent`, then `--resume` on every spawn). The second task starts with the whole transcript of the first, the third with both, and so on until the session hits the context limit: auto-compaction mid-task, degraded focus, higher cost per turn (every turn re-reads the old tasks).

**Goal:** each task a team agent starts runs in a **fresh session**: no context from previous tasks, same agent, same worktree, same branch.

## 2. Options: fresh session vs `/compact` (and `/clear`)

| | A. Fresh session per task (recommended) | B. `/compact` before the next task | C. `/clear` in the live process |
|---|---|---|---|
| Context at task start | System prompt + role + task turn only. Deterministic. | A model-written summary of every earlier task, plus the new task. | Same as A. |
| Noise | None. | The summary stays, and grows: task 4 carries a summary of a summary of tasks 1–3. Irrelevant details survive. | None. |
| Cost | One process restart (a few seconds) and a cold prompt cache for the system prompt/tools (~tens of k tokens, cache write). | A full pass over the whole old context to summarize it (the most expensive turn of the session, up to the context size), then the summary is re-read by every later turn. Compaction near the limit can fail. | Same as A, no restart. |
| Latency | Spawn time (seconds). | Compaction time (often 30 s – minutes on a large context). | Instant. |
| Useful carry-over | Lost: the agent re-reads the code it needs. Mitigated: the task turn lists the agent's earlier tasks on this branch, and `git log` holds their work. | Kept, in lossy form. | Lost (same mitigation as A). |
| Orchestrator fit | Reuses the existing FR-415 machinery (`start_fresh`, new `--session-id`, spawn admitted by the Governor like any resume). Session id, transcript and usage tracking stay exact. | Fits (`/compact` is already understood by the state machine), but the session id never changes, so transcript/usage stay one ever-growing session. | Claude Code mints a new session id inside the process; the orchestrator's `session_id` goes stale, so the next resume targets the wrong session. Needs new hook plumbing to follow the id: fragile. |

**Decision: A.** Tasks are scoped by the lead to be self-contained (the task body, the feature docs and the branch carry everything needed), so a summary of earlier tasks is mostly noise; A costs less than B, is deterministic, and uses a path the runtime already has and tests. C would be A without the restart but breaks session tracking.

## 3. Scope

### Core (`back`), `crates/agent-runtime`

1. **When a free team agent that has already run a session is given a new task, its session is rotated before the task turn is delivered**: a new session id, `start_fresh = true`, so the delivery loop spawns it with `--session-id <new>` (not `--resume`) once the Governor admits the turn. Same agent id, worktree, branch, role, allocation.
   - Never-started agents are unchanged (they already start fresh).
   - The lead is never rotated (it gets no tasks; its session is the feature's memory).
   - Messages, nudges and review requests between tasks keep going into the current session as today. Only the **start of a task** rotates.
2. **Only rotate a quiet agent.** The task is given to the free agent only when it is not mid-turn and has no queued or delivering turn (e.g. a nudge from the lead about its previous task, or the end of the turn that called `report_done`). If it is live and idle, it is suspended (session kept on disk) and then rotated. If it is busy or has turns pending, the task waits for a later `team_tick` (or another free agent / a new seat within `max_concurrency` takes it, as today). The tick must not block on a busy agent.
3. **The task turn of a fresh session says where the agent stands**: one extra line listing the agent's earlier tasks on this branch with their status (e.g. `Earlier on this branch: T-1 (done), T-3 (review). Their work is in git log; follow-ups on them may still reach you.`). Nothing added when there are none.
4. **Events**: an agent event `fresh_session` `{old_session, new_session, task}` (not `restarted_fresh`, which counts startup failures in the report), and a run event `agent.fresh_session` `{node, task, title}` for the timeline (see `api-contract.md`).
5. The old session's transcript stays findable (it is on disk; the project-folder sweep in `transcript_files` indexes it). Usage already handles a new session id (`usage.rs` resets the cost delta). Verify both, no change expected.

### UI (`front`), `ui/src/lib/teams.js`

6. `timelineText` renders `agent.fresh_session` as `T-<task> starts in a fresh session`, and `FAMILY` files it under **Run**. Unit test next to the existing timeline tests.

### Out of scope

- A per-node or per-team setting to keep one session across tasks (possible follow-up if some role benefits from carry-over).
- Rotating the lead, or rotating on messages/reviews.
- Compacting anything automatically.
- New worktrees or agents per task (the agent and its branch are kept).

## 4. Acceptance

1. Team test (in `team.rs` tests): a node with `max_concurrency = 1` gets T-1 then T-2. After T-1 is reported done and the agent is idle, T-2 is assigned to the **same agent** with a **different `session_id`**, `start_fresh` set, and the T-2 task turn queued; the agent event `fresh_session` and run event `agent.fresh_session` are recorded.
2. E2E with the fake claude (`tests/e2e_fake_claude.rs`): the spawn for the second task uses `--session-id <new id>`, not `--resume`.
3. An agent with a queued turn (a nudge) or mid-turn is not rotated: the task stays Ready until it is quiet, then is assigned.
4. A first task on a never-started agent behaves exactly as today (no rotation, no event).
5. The T-2 turn lists T-1 with its status; the T-1 turn has no such line.
6. `cargo test -p agent-runtime`, `cargo clippy`, and `bun run test` / `bun run check` / `bun run lint` in `ui/` pass.

## 5. Tasks

| Task | Owner | Notes |
|---|---|---|
| T-a Fresh session per task | back | `team.rs` dispatch + a rotation helper next to `fresh_session` in `startup.rs`; tests 1–5 |
| T-b Timeline label for `agent.fresh_session` | front | `ui/src/lib/teams.js` + test; independent of T-a (contract in `api-contract.md`) |

No design work: `ux` and `ui` are not used.
