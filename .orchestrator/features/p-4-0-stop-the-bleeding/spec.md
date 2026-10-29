# P4.0 — Stop the bleeding · Feature spec

> Feature `p-4-0-stop-the-bleeding` · Integration branch `orch/p-4-0-stop-the-bleeding/integration` · Source: [docs/SPEC.md](../../../docs/SPEC.md) §11 P4.0 (v0.12), FR-203, FR-206, FR-415, FR-705, FR-709, FR-1311 · Why: [brief 2026-10-28](../../../docs/brief/2026-10-28-overwatch.md), [ADR-011](../../../docs/adr/011-overwatch-and-attention-model.md) §6
> Companions: [api-contract.md](api-contract.md) (core ↔ UI) · [ux-brief.md](ux-brief.md) (UI touch points, no Penpot pass) · [decisions.md](decisions.md)

## 1. Goal and exit criterion

SPEC §11: *"Read allow on the agent's `messages/` folder and `delivered_at` at paste (FR-206); expired holds park the agent (FR-705); never-started sessions recovered (FR-415); memory-cap hysteresis and no pressure-suspend of a pasted turn (FR-203); who decided each permission (FR-709); Notification events stored with their type; `pane_intervention` events (FR-1311); the restart gate gets a payload."*

**Exit criterion: a real team run in auto mode**, with permission prompts counted by tool and pane interventions counted by cause, against the 2026-10-27 baseline (29 prompts, 6 interventions, ~5 h of agent time waiting). So P4.0 also ships the **run report** (§3.9) that produces those numbers from the store, and makes the permission mode of every agent visible in it.

This phase fixes defects and adds measurement. It adds **no new screen**: the UI changes are the Inbox cards and the agent pane strip that the backend changes touch (§3.10). Attention records, Decision context blocks and the card lifecycle are P4.1/P4.3, not here.

## 2. What the code does today (HEAD a9ccc15)

| Item | Today |
|---|---|
| Permission holds | **Two implementations.** The app turns the runtime's store-backed hold off (`orch-app/src/core.rs:1176` `permission_hold = ZERO`) and holds every non-rule prompt in `orch-app/src/permissions.rs` `PermissionBroker`: in memory only, keeps a 200-char summary (no `tool_input`, no `cwd`), no store row, **no audit, no event**. Hold = hook deadline − 2 s = **538 s**. On expiry the hook returns no decision, the pane's dialog takes over, the `perm-N` card disappears and a `wait-<agent>` card ("A prompt or question in its pane") replaces it: the tool and its input are lost. No park path, no one-shot allow. |
| MCP ask rules in the app | `agent-runtime/src/mcp.rs:103` budget = `min(deadline−2 s, permission_hold.max(1 s))` = **~1 s** in the app: the item expires before anyone can answer. |
| Audit (FR-709) | Rule allow/deny on PermissionRequest is audited (`rule:<id>`). Broker answers, pane answers, expiries, EOF and reconcile closes are not. MCP allows on non-commit tools are not. |
| Messages folder (FR-206) | Turns over 900 chars are written to `<app_root>/agents/<id>/messages/<turn>.md` (outside the worktree). `compile()` emits no Read allow for it, so every long assignment prompts (8 of 29 prompts). |
| `delivered_at` (FR-206) | Turns: already set at paste (`runtime.rs:1923-1930`). **Team messages: set when the turn is queued** (`team.rs:894-896`), so a message stuck behind the Governor or a slot counts as delivered. |
| Memory cap (FR-203) | Pressure sampled every 5 s and applied at once; **no hysteresis** (`runtime.rs:1570-1576`). An agent with a pasted, not-yet-started turn is `Idle` with 0 queued turns, so it is the first pressure victim; its `Delivering` turn is never re-queued after the exit (only the boot reconciler does). |
| Start / resume (FR-415) | No startup timeout: an agent stuck before SessionStart (a dialog other than the trust one) stays `Starting` forever. A resume that exits at once becomes `Error`; nothing checks for a missing transcript and nothing retries fresh. `rec.session_id` is never changed. |
| Notification events | Every hook writes one event `{tool, prompt_id, source, error}`; **`notification_type` and `message` are dropped**. |
| Pane interventions (FR-1311) | Keystrokes reach `Live::user_input` (stores `last_user_input_ms`); nothing records an event. Terminal replies (focus, CPR, DA, mouse) travel the same path. |
| Restart gate | A gate hold still held at a core restart becomes `add_inbox(.., "gate", text)`: **no payload, priority 0**; the UI shows it as "Spec gate", `gate_detail` fails, deciding it has no effect. Gate holds with an `inbox_id` are closed as `Interrupted` although the reconciler logs "back in the Inbox". |
| Run measurement | No count query over events; no weekly report in the code yet. The app serves the ctl socket, so `orchd` commands work against the running app. |

## 3. Scope

### 3.1 One permission hold path — FR-705 (M)
- **The app uses the runtime's store-backed hold** (`agent-runtime/src/inbox.rs` `hold_for_decision`); `PermissionBroker` is removed. Every prompt that no rule answers becomes a store Inbox item of kind `permission` with its payload: `tool`, full `tool_input`, `cwd`, `tool_use_id`, `summary`, `description`, `ask_rule?` (the rule id when an *ask* rule sent it), `always?` (the pattern "Always allow" would add).
- **Hold length: 300 s by default** (`permission_hold`, setting `permissions.hold_secs`), always ≤ hook deadline − 2 s. MCP ask rules use the same budget (fixes the 1 s hold).
- Pane answers, EOF (Esc in the pane) and the end of the turn close the item as today's runtime path does (`answered_in_pane`, `conn_closed`).
- "Answer in the pane" stays offered while the hook is still held; "Always allow" is offered only when no *ask* rule sent the prompt.
- The agent pane's permission strip and the Inbox read the same item (no second list).

### 3.2 Expired holds park the agent — FR-705 (M), ADR-011 §6
- When the hold reaches its deadline with no answer: the hook **denies** with `ORCH: awaiting the user's approval. Stop here; you will be resumed when it is answered.`, the item moves to status **`parked`** (still open, payload intact), and the turn ends.
- A parked agent with no other running work is **suspended at once** to free its slot, except an agent with a reserved slot (Lead), which just idles. It is not resumed by the idle/queue logic while its decision is parked.
- **On Allow**: the core adds a **one-shot allow** for that exact call (agent + tool + canonical JSON of `tool_input`, consumed on its first match, expires after 1 h), then resumes the agent (or delivers to it if live) the user turn `approved: retry the call <tool> <summary>`. The one-shot allow is checked before the rule engine and is audited when consumed (`decided_by = user`, `one_shot: true`).
- **On Deny**: delivers `denied: <tool> <summary>. <message or "The user said no.">` as a user turn.
- **On "Always allow"** (when offered): adds the rule as today, then resumes as for Allow.
- A parked item is never replaced by a `wait-<agent>` card; "Answer in the pane" is not offered for it (the pane dialog is gone).
- A parked item survives a core restart (it is a store row); the reconciler leaves it `parked`.

### 3.3 Who decided — FR-709 (M)
- Every permission answer, gate decision and permission rule change writes **one audit row** whose `actor` is the decider: `user` (Inbox card or app action), `pane` (answered in Claude Code's own dialog, or Esc in the pane), `rule:<id>`, `expired` (hold deadline reached: parked), `core` (closed by the reconciler after a restart). `overwatch` is reserved for P4.4.
- The audit `detail` carries `item` (inbox id when there is one), `tool`, `tool_input`, `tool_use_id`, and for rules the rule's pattern and effect.
- Paths to cover: rule allow/deny/ask (PermissionRequest **and** MCP, including MCP allows on non-commit tools), user decide, pane answer, EOF, expiry → park, one-shot consumption, reconciler closes, rule added/revoked.
- The Inbox item stores `decided_by` with its final status, so the UI and the run report can read it without joining the audit log.

### 3.4 The agent can read its own messages — FR-206 (M)
- `compile()` adds `Read(//<agent_dir>/messages/**)` to `permissions.allow` (via `rule_path`, the path contains a space). Edit/Write on the app root stay denied; sandbox reads of `agents/` are unchanged.
- Test: the compiled settings for a solo and a team agent contain the rule, and no allow rule covers `state/` or another agent's folder.

### 3.5 `delivered_at` at paste — FR-206 (M)
- Team messages are marked delivered (`delivered_at_ms`, `to_agent`) **when the turn that carries them is pasted** (in `pump_one`, after the PTY write), not when it is queued. The busy-agent `additionalContext` path keeps setting it at hand-over.
- The re-queue guard no longer relies on `delivered_at_ms IS NULL`: a message already carried by a `Queued`/`Delivering` turn of the agent is not queued again.
- A cancelled turn still un-delivers its messages (today's `undeliver_messages`).

### 3.6 Memory cap hysteresis and pasted turns — FR-203 (M)
- **Hysteresis** on the pressure level that sets the cap: a pure `PressureFilter { effective, candidate, since_ms }` in `resources.rs`, fed every housekeeping sample. A **higher** level becomes effective once the raw level has stayed at or above it for **60 s**; a **lower** level once the raw level has stayed at or below it for **120 s** (so critical → warn needs 120 s at warn or below, warn → normal 120 s of normal). The `memory_pressure` event fires on an effective change and carries `raw` and `effective`.
- **A pasted turn is not idle**: an agent with a `Delivering` turn (pasted, no UserPromptSubmit yet) is not suspendable for pressure or idleness, and the paste counts as activity for least-recently-active ordering.
- **No lost turn**: when an agent exits or is suspended with a `Delivering` turn, that turn goes back to `Queued` (same rule as the boot reconciler) and its team messages are un-delivered.
- Test (fake memory probe): warn↔normal flipping every 5 s for 10 min changes the cap 0 times; warn held 60 s lowers it once; normal held 120 s raises it once; an agent with a `Delivering` turn is never suspended, and a suspended agent's `Delivering` turn is back in the queue.

### 3.7 Starts that never happen — FR-415 (M)
- **Startup blocked**: a spawned agent with no SessionStart within **30 s** and no trust dialog gets one Inbox item, kind `startup`, level 2, payload `{waiting_on: "startup", mode: "new" | "resume", since_ms, tail}` (the ANSI-stripped PTY tail). Actions: open the pane, **restart fresh**, dismiss. SessionStart closes it (`decided_by: core`, "started").
- **Resume that dies at once**: a `--resume` spawn that exits within **5 s** while no transcript exists for its session id is restarted **once** as a fresh session: new `rec.session_id`, `SessionMode::New`, same worktree; the agent's task brief is delivered again (rebuilt from its seat's task, or the feature brief for a Lead), followed by its `Delivering`/`Delivered`-but-unstarted turns, re-queued. Event `restarted_fresh {old_session, new_session, why}`.
- A second failure (or a fresh start that also exits within 5 s) becomes an Inbox item kind `startup`, payload `{waiting_on: "startup", failed: "resume" | "fresh", exit_status, tail}`, with **restart fresh** and **delete** actions; the agent is `Error`, not retried in a loop.
- **Restart fresh** is also a core action (`Runtime::restart_fresh(id)`), callable from the item and the agent's menu; the Lead's `resume_agent` tool uses it when the agent's last resume failed this way.
- The spawn mode and `spawned_at_ms` are kept on the live record so the exit callback can apply the rule.

### 3.8 Events that measure — Notification type, FR-1311 (M)
- **Hook events keep their specific fields**: the event detail adds `notification_type` and `message` for Notification, `tool_use_id` for tool events, `permission_mode` for SessionStart when the payload has it. (Kind stays the hook name.)
- **`pane_intervention`** events: when the user types into an agent's pane (app terminal or `orchd attach`) while the agent **waits** (a pending/parked decision of its own, startup, trust dialog, a `Delivering` turn, a `Queued` turn) or is **idle with an open task**, the core records one event per wait episode: `{waited_on: "permission" | "question" | "gate" | "parked" | "startup" | "trust" | "delivering" | "queued" | "idle_with_task", item?, since_ms, waited_ms, team_run_id?}`. Priority order is the order of that list. The episode ends when the agent starts a turn or the decision closes.
- Terminal replies are not typing: focus in/out (`ESC[I`, `ESC[O`), cursor position and device attribute reports, and mouse reports are filtered before the check. Keystrokes into a working agent, or an idle one with no open task, record nothing.
- A permission answered in the pane while a card existed also counts (the `pane` decider in §3.3 is the evidence; the run report counts both).

### 3.9 Run report — exit criterion measurement (M)
- A store query and ctl request `RunReport { team_run_id?, since_ms?, until_ms? }` → `orchd report [--run <id>] [--since <iso>] [--until <iso>]` (JSON with `--json`, else a short text table).
- Content: per agent its permission mode (from its spawn args) and role; **permission prompts** (PermissionRequest hook events) by tool and by decider; **pane interventions** by cause; **waiting time**: sum of the open → decided durations of the agent's permission/question/gate/startup decisions (parked time included); startup-blocked and fresh-restart counts; memory-pressure effective changes and pressure suspensions.
- The baseline row (2026-10-27: 29 prompts, 6 interventions, ~5 h waiting) is printed next to the numbers when `--baseline` is passed (constant in the code, from the brief).
- Events are kept 30 days; the report is run right after the exit run, so retention is unchanged.

### 3.10 Restart gate payload and UI touch points
- **Restart gate**: a gate or permission hold still held when the core restarts comes back as an **open item of its original kind with its original payload** plus `{restarted: true}` (permission holds become `parked`: the hook is gone, so §3.2 applies). Deciding it has its normal effect, delivered as a user turn. A legacy hold with no item gets kind `gate`, payload `{action: "restart_gate", agent, detail}`, level 1, and Approve/Reject deliver `approved: proceed` / the reason. The reconciler's log text and its action agree.
- **Inbox UI** (front, see [ux-brief.md](ux-brief.md)): the permission card shows the tool, the cwd and the full input (collapsed beyond 6 lines), and a **Parked** marker with "the agent stopped and resumes on your answer"; actions follow §3.2. New `startup` kind (label, badge, actions). `restart_gate` and unknown gate actions no longer render as "Spec gate". The decided toast names the decider's effect ("allowed · agent resumed"). The agent pane strip reads pending `permission` items. The sidebar "needs you" count includes `permission` and `startup` items.

## 4. Out of scope (P4.0)
- Attention records, `waiting_on` as a core model, push channel (P4.1). `pane_intervention` computes its cause locally in the core; P4.1 will replace that with the attention record.
- Decision context blocks, the card lifecycle display, threading (P4.1/P4.3). P4.0 stores the payload; it does not redesign the card.
- The policy file, the Overwatch and the `overwatch` decider (P4.4).
- The weekly report (FR-818): only the run report.
- Changing role permission profiles: starter roles already default to `autonomous` (`--permission-mode auto`). The run report shows the mode each agent actually ran with; the exit run uses roles in auto mode.

## 5. Non-functional
- **Security (ADR-009)**: the new Read allow is limited to the agent's own `messages/`; a one-shot allow matches one exact call of one agent; parking never widens a rule; the hard rules (deny over ask over allow) are unchanged.
- **Memory (NFR-1b)**: no new process. Parking frees a slot; hysteresis stops the suspend/resume churn.
- **Data (NFR-8)**: one migration: `inbox_items.decided_by`, status value `parked`, hold status `parked`, `one_shot_allows` table (agent, tool, input hash, expires, consumed). Old rows read as before.
- **Tests**: `make ci` green (fmt, clippy -D warnings, cargo test, biome, svelte-check, vitest, ui build). Every behaviour in §3 has a test; the runtime ones run on the fake claude (`agent-runtime/tests/e2e_fake_claude.rs`) and the fake memory probe. Tests hold at any date, hour and time zone (as on main).

## 6. Acceptance (demo at the PR gate)
1. A long task assignment (> 900 chars) to a team member produces no Read prompt.
2. A permission prompt left unanswered for 300 s parks the agent: the card stays with the tool, cwd and full input; the agent is suspended. Allow → the agent resumes and the call runs without a second prompt; the audit shows `expired` then `user`.
3. Answering a prompt in the pane audits `pane`; a rule answer audits `rule:<id>`; `orchd audit` shows all three.
4. A fake claude that never sends SessionStart produces a `startup` card with its PTY tail after 30 s; a resume that exits at once with no transcript restarts fresh once with its brief; a second failure produces a card.
5. Fake memory probe flipping warn↔normal: the cap does not move; an agent with a pasted turn is never suspended.
6. A team message waiting behind a full slot has no `delivered_at` until its turn is pasted.
7. Typing into a waiting agent's pane records one `pane_intervention` with its cause; focus changes record nothing.
8. Restart the core with a held permission and a held gate: both come back with their payloads and work.
9. **Exit run**: the user launches a real team run on the new build with roles in auto mode; `orchd report --run <id> --baseline` prints prompts by tool, interventions by cause and waiting time next to the baseline. (Run by the user after the merge; the numbers go into the PR follow-up.)
