# ADR-010 — Review before merge: self-review by slash commands, or a reviewer node

- Status: Proposed · Date: 2026-10-27

## Context
In P2 the Lead "reviews then merges" only because its prompt says so: nothing tracks a review, and `request_review` is a bare message. Running a full team (p3-ecosystem, 2026-10-27) showed the cost of that: the Lead's context fills with diffs, findings never reach anyone, and the user ends up reviewing in the Lead's pane. Claude Code ships two review commands: `/code-review` (correctness, with its own reviewer subagents, `--fix` to apply them, `--max-findings all`) and `/simplify` (reuse, simplification, efficiency, applied).

Two ways to use them:

| | Self-review (the owner runs the commands) | A reviewer node |
|---|---|---|
| Live slots | none more (the 3-slot cap is already tight) | one more agent per team |
| Context | the owner's, warm (prompt cache) | cold: reads the branch from scratch |
| Independence | the commands start their own reviewer subagents, but the author applies the fixes | a second agent, its own judgment |
| Fixes | applied in place by the owner | sent back to the owner |
| Cost | the owner's context and two turns | a whole agent session per review |

## Decision
1. **The team chooses** (`Team.review`, canvas Rules panel): `self` (default), `node`, or `none` (the Lead reviews, as in P2).
2. **Self**: the owner's first `report_done` does not report. It marks the task reviewing and queues, in order, `/code-review high --fix --max-findings all`, `/simplify`, then an `ORCH:` turn asking it to commit the fixes and call `report_done` again with `review: {fixed, unfixed: [{finding, reason}]}`. The task stays in progress (the owner gets no new task meanwhile). The second call reports, with the review, and moves the task to Review.
3. **Node**: the first `report_done` creates a review task for the node a `reviews` edge from the owner points to (else a node with a reviewer role), so the dispatcher starts it like any task. The reviewer reads the branch, runs `/code-review high --max-findings all` (no fixes: it doesn't change another node's branch), and calls `submit_review(task, findings)`; the report then goes to the Lead with the findings, and the owner gets them too.
4. **The Lead triages**: `merge` of a task is refused until it has a review (unless `none`). For each finding left, the Lead accepts it (with a reason), sends it back with `nudge`, or asks the user, and only for a product, scope or security call (one `ask_user` per task).
5. **Command delivery**: FR-206 starts any app message beginning with `/` with a space, so no command runs. Review commands are the exception: a turn flagged `command` is typed **as is**, alone (never batched with other turns), and only from the fixed list above. It carries no turn marker, so the core recognises it at UserPromptSubmit by its prompt and adds the marker before the state machine sees it (the turn is the app's, not the user's). A command whose prompt never reaches a hook ends with the agent's next Stop.
6. `/code-review` runs subagents: a `self` team's nodes don't deny the Agent tool.

## Consequences
- The Lead's turns shrink to triage and merge; the user sees only the findings the Lead escalates.
- Each task costs two more owner turns (self), or one reviewer session (node).
- To confirm on each pinned Claude Code version: UserPromptSubmit and Stop fire for skill slash commands typed in the TUI, and a pasted `/command` runs as a command. If not, the fallback (bound at the next Stop) still keeps the queue moving.
