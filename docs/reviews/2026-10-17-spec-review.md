# Spec review — 2026-10-17 (SPEC v0.1 + ADR-001…008)

Five specialist reviewers read the spec independently: **product**, **architecture**, **Claude SDK / Claude Code feasibility**, **UX/UI** and **pacing/scheduling**. This report merges their findings, removes duplicates and adds a verdict on each one. Two critical claims were checked directly against the official docs (marked ✔ verified). One reviewer claim was corrected (marked ✎).

**Overall verdict:** the vision is coherent and the spec is broad, but it is **not buildable as written**:
1. Its core premise (driving a Max subscription through the Agent SDK) conflicts with Anthropic's stated policy.
2. The pacing feature, the main differentiator, has no control law.
3. Several mechanisms (Governor admission, sandbox, branch names) don't work the way the spec assumes.
4. The scope is very large for one developer.

None of this is fatal, but a few **structural decisions** have to be revisited before anything else is changed (§1).

---

## 1. Decisions needed from Guillaume (these change the shape of the spec)

### D1 — How do we drive Claude? ✔ verified · *blocker*
- Agent SDK overview: *"Unless previously approved, Anthropic does not allow third party developers to offer claude.ai login or rate limits for their products, including agents built on the Claude Agent SDK. Use the API key authentication methods…"*. SDK use falls under the **Commercial Terms**, and subscriptions under the consumer terms, which restrict automated access.
- The **5h / weekly utilization % and reset times** are documented only in the **interactive Claude Code CLI's statusline data** (`rate_limits.five_hour|seven_day.used_percentage`, `resets_at`, Pro/Max only, after the first response). No SDK message type exposes them. The SDK only has `rate_limit_event` with `status`, optional `utilization` (units undocumented) and `resetsAt`, and no window label.
- The spec's main premise (SDK + Max plan + pacing on the weekly %) therefore rests on an unapproved path, with a signal the SDK does not provide.

| Option | What it means | Pacing signal | Compliance | Effort |
|---|---|---|---|---|
| **A. Wrap the real Claude Code CLI** | Each agent is a real `claude` process in a PTY pane (the chat UI comes for free). The app orchestrates around it with **hooks**, **statusline JSON**, **OpenTelemetry**, agent files in `.claude/agents/`, and MCP. | ✔ real 5h/weekly % from the statusline | Official client on your own subscription. Unattended runs still need confirmation from Anthropic | **Lowest**: no chat UI to rebuild (FR-203–206) |
| **B. Agent SDK + API key** | As specced, but billed per token | Tokens and € (exact); no plan windows | ✔ documented path | High; pacing becomes a € budget |
| **C. Agent SDK + subscription** | As specced | Estimated only | ✗ conflicts with the docs note unless Anthropic approves | High |

**Recommendation: A as the primary runtime**, with B as the second Provider (it already fits the "pluggable later" decision). A also answers the product reviewer's "build vs. reuse" finding: rebuilding Claude Code's chat is the most expensive and least differentiating part of the spec. **In every option**, ask Anthropic before relying on unattended or overnight subscription use (R9 becomes a P0 go/no-go).

### D2 — Scope of v1 · *major*
P1 currently includes nearly all of §4.1, 4.2, 4.9, 4.10, 4.11, the governor and the Inbox. All reviewers agree this is too large. Proposal: **P1 = projects + worktree sessions + usage collector + read-only pacing forecast, then the governor (concurrency/queue/pause/defer)**. Teams come in P2, once pacing has proven useful.

### D3 — Team builder: canvas or form/files? · *major*
Product and UX both flag the visual-only canvas as over-built for 3–7 roles. It also works against the keyboard-first goal and makes the "who can message whom" rules hard to read. Proposal: a **form/list editor** (or a team file editable in nvim) as the main tool, with an **auto-laid-out read-only graph** next to it. This reverses an earlier decision, so it needs your call.

### D4 — Worktree location · *major*
Nesting worktrees inside the repo works, but it has real costs:
- Claude Code loads **CLAUDE.md files from parent folders**, so every agent would also read the main checkout's CLAUDE.md, possibly from a different branch.
- Spotlight, Time Machine, IDE indexers, Docker build contexts and file watchers would crawl N copies of `node_modules` (CPU and RAM on this laptop).
- Moving the repo breaks the links (`git worktree repair` fixes them).
- The tracked `.orchestrator/features/` folder exists in every worktree, so it's unclear which copy of the spec is authoritative.

✎ The architecture reviewer's claim that `git clean -fdx` deletes the worktrees is **overstated**: git skips folders that contain a `.git` unless you pass `-ff`. Proposal: keep "inside the repo" as you asked, but add mitigations (`git worktree lock`, settings excludes) and pin the spec's source of truth to the integration worktree. Or switch the default to a sibling folder. Your call.

### D5 — Penpot gate · *major*
Product and UX both find the design-first gate (every screen × 4 states, plus clickable flows for J1–J8, before P1) heavy for one person. Proposal: Foundations/tokens + app shell + the 3–4 P1 screens; states as component variants; prototype only J2, J5 and J8; small changes may be made in code first and backported to Penpot in the same PR.

---

## 2. Blockers (fix regardless of decisions)

| # | Finding | Where | Fix |
|---|---|---|---|
| B1 | **Pacing has no control law**: no error metric, thresholds, hysteresis, de-escalation rule, or formula for the ideal curve. Statuses (under/on track/over) have no numbers. It cannot be built or tested. | FR-803–805, ADR-007 | Error `e = used − ideal` (in percentage points) plus a projected-exhaustion time. Two or three pressure states with enter/exit thresholds and a 30-min minimum dwell. Acceptance test: a simulated week ends between 85 and 100% without ever hitting the limit. See the v1 algorithm in §5. |
| B2 | **Shares, priorities, reserve and "never throttle interactive/critical" don't compose.** Three critical allocations can use up the week, and nothing defines unused share or caps. | FR-802/806, BudgetAllocation | Shares are **weights** (weighted fair queuing) and unused weight is redistributed. No rollover across weeks. Interactive/critical work draws from the reserve first; once the reserve is gone, warn instead of block and squeeze the other work harder. |
| B3 | **The Governor can't gate inside a turn.** One `send()` is an autonomous loop of many model and tool calls, and for a team agent it can run for hours. Built-in subagents (Task tool) bypass it. | FR-805, §6 principle 1 | State the granularity honestly: **turn/task start + each tool call through a PreToolUse hook** (awaiting admission). Add a per-turn watchdog (token or time ceiling, then interrupt at a tool boundary). Count or limit Task subagents. |
| B4 | **Branch names collide in git**: `orchestrator/<f>` and `orchestrator/<f>/<role>-<n>` can't both exist. J2 uses a third pattern. | ADR-003, FR-105, J2 | `orch/<f>/integration`, `orch/<f>/<role>-<n>`, `orch/solo/<slug>`. |
| B5 | **The "sandbox" isn't enforced.** Permission rules cover Claude's Read/Edit tools, not `cat`/`curl`/scripts run through Bash. Agents inherit your `gh`/git credentials, so they can push or open PRs and skip the PR gate. The network allow-list can't be checked from tool calls. | NFR-3, FR-701/702 | Turn on Claude Code's **OS-level Bash sandbox** (writes limited to the agent's worktree, network allow-list). Strip `GH_TOKEN` and credential helpers from agent environments. Push/PR only through gated orchestrator MCP tools. Implement gates and the audit log (FR-706) with **PreToolUse hooks**, because `canUseTool` is skipped for allow-listed tools. |
| B6 | **Concurrency vs memory.** "≥10 concurrent agents" means ~10 Claude Code processes (one per session, documented). The SDK hosting guide suggests ~1 GiB per agent as a starting point. That contradicts NFR-1b on a low-RAM laptop. | FR-202, NFR-1/1b | Measure in P0. Restate as "N queued, K live" with a measured default (likely 3–4). The 250 MB shell target must count WKWebView's helper processes. |
| B7 | **Keyboard focus model is missing.** It isn't defined who owns a key when nvim/zsh has focus, and pane commands beyond ⌘B/⌘I/⌘K aren't specified. | §9.1, FR-1204, ADR-006 | Keyboard contract: the app takes only ⌘-chords plus one configurable leader key; everything else goes to the PTY. Define the pane commands (split/focus/zoom/close). Add a shortcut map page in Penpot with a conflict check against nvim and kitty. |
| B8 | **No attention rule for 10 agents.** Notifications, badges, dots and toasts have no priority, and FR-210 notifies on every finish. | J5, FR-210/704/809 | Attention ladder: ① blocked on you (by wait time) ② errors ③ budget asks ④ info (digest only), each mapped to a channel. Add Do-Not-Disturb, batching, and a "next waiting agent" key. |

If D1 = B or C, also: **B9 — the Provider boundary is undefined.** §8 is an in-process TypeScript interface, but the Governor lives in Rust. Specify a versioned JSON-RPC protocol (commands, event stream, reverse requests for permissions and hooks, reconnect).

---

## 3. Major findings

**Architecture & runtime**
- **Memory savings are mostly illusory** (ADR-001). The sidecar only saves a thin wrapper; the real levers are suspension and the process cap. "Suspend" must mean terminate-and-resume (SIGSTOP frees no RAM). Resuming re-pays the context when the cache has expired, which costs budget, so the Governor must account for it. *(Option A in D1 removes the sidecar entirely.)*
- **Single point of failure.** If the sidecar crashes, every turn and pending permission is lost, and child processes can be orphaned. Run each agent in its own process group, reap orphans at startup, and record in-flight turns and gates.
- **Waiting at a gate pins memory.** Answer "deferred", end the turn, suspend the agent, and resume it when the user approves.
- **Internal MCP: transport, identity and delivery are unspecified.** Prefer an in-process or stdio MCP (no port, no token visible in `ps`). Define each edge type's meaning. Delivery: inject messages into busy agents (streaming input or PostToolUse `additionalContext`); resume idle agents through the Governor; never poll.
- **The integration branch has no owner.** One branch can't be checked out in two worktrees, so the Lead needs its own integration worktree.
- **Restart resilience is overstated** (NFR-2). PTYs don't survive an app restart without a PTY daemon: nvim and test watchers die. Say so, or add a tmux/abduco-style host. Add a startup reconciler (DB ↔ `git worktree list` ↔ processes ↔ Claude JSONL).
- **Off-hours runs ignore sleep** (FR-808). Require AC power plus a power assertion, and tell the user the lid must stay open.
- **Event sourcing duplicates Claude's own JSONL transcripts.** Use an append-only event table plus state tables, merge streaming deltas, store large outputs outside the rows, and enable WAL, FTS5 and migrations.
- **Data model gaps:** Turn, Worktree, InboxItem (`ask_user` is not a Gate), Artifact/Handoff, QueueEntry, Calibration, ForecastSnapshot. Other fields:
  - Session: parentSessionId, pid, lastActiveAt.
  - Task: assigned to an AgentNode, not a Session.
  - UsageSample: sessionId nullable, plus signal provenance (observed or estimated).
  - TeamRun: snapshot of the team version.

**SDK / Claude Code facts** (from the feasibility reviewer, with doc URLs in their report)
- Supported: resume/fork/continue, `interrupt()`, `streamInput()`, `setModel()` (no effort argument), `setPermissionMode()` including plan mode, hooks, `createSdkMcpServer`, `agents` definitions, `maxTurns`, `maxBudgetUsd`, per-model `modelUsage` with cost.
- Unverified: a "compact now" call in the SDK, a context breakdown API (FR-903), and changing effort mid-session. `total_cost_usd` is a list-price estimate, so rename `costEq` accordingly.
- Max plans also have a **separate weekly limit for some models**, which the pacing model must handle.

**Pacing**
- Treat the **5h and weekly windows** as two independent constraints and admit work against whichever is stricter. Keep ~15% of the 5h window as interactive headroom.
- **Queuing defers usage, it never reduces it.** Only downgrading or dropping work saves budget. Add a **catch-up rule** for weeks running under the curve, and aging so low-priority work isn't starved.
- **The estimator** (per-model tokens → %) can't be calibrated from a few data points a week. Use one scalar (cost estimate per 1% of the week), updated whenever a real % is observed, plus a 10–15 pp safety margin. Feedback control matters more than precision. *(Option A makes the observed % available directly.)*
- **Lever ranking:**

  | Lever | Value |
  |---|---|
  | Concurrency cap | Main knob |
  | Queue / pause | High |
  | Defer after reset | Decisive but coarse |
  | Downgrade | Quality risk, and it breaks the prompt cache |
  | Smoothing | 5h only |
  | Compaction | Weak or negative |

  Drop L5; allow L4 only at task boundaries and for opted-in roles (J8 currently downgrades the Lead).
- **"Ask" stalls when you're away.** Every ask needs a default action and a timeout.
- **Unattributed usage** must be forecast as its own rate, otherwise it absorbs all of the estimator's error.

**Product**
- **Missing journeys:**
  - Plan limit actually reached
  - Claude login expired
  - Laptop asleep
  - Removing a project (cleanup)
  - Backup/restore and upgrades
  - Onboarding (it has a screen but no journey or FR)
- **FR-806 caveat:** if most of your usage is interactive, the Governor has little to act on. Say so in the spec.
- **Phase/priority mismatches:**
  - FR-501 Linear is a Must but lands in P3.
  - FR-1203 is in no phase.
  - The gates FR-701 relies on arrive only in P2.
- **Untestable wording:** "Claude-Code-grade", "no jank", "renders like kitty", "stuck for X min". Replace each with a number or a pass/fail check.
- **Tracked `.orchestrator/` in client or open-source repos** should be opt-in per project.

**UX**
- **IA overlap:** live agents appear in four places, and "panes, not pages" contradicts a screen table. Make every screen a pane type, merge Projects home and Monitor into an "Overview", and put solo sessions under their project.
- **Laptop fit:** sidebar + inspector + two panes leaves ~70 columns per pane. Define breakpoints, an overlay inspector, a sidebar icon rail, and pane zoom.
- **Governor transparency:** every affected agent shows the reason, ETA, "Run now" with its cost, and undo. Rename "Throttled" to "Downgraded (model)". Show both windows plus the estimator's confidence. Run a shadow mode in the first week.
- **Chat parity gaps:** rewind/checkpoints, edit & resend, subagent transcripts, thinking blocks, background shells, `!`/`#` modes, plan approval UI, per-turn usage, the steer queue, **vim mode in the composer**. *(Option A removes this whole category.)*
- **Theme from kitty colors:** map only surface, foreground and accent, run a contrast check, and keep status colors fixed. Test three palettes, one of them light.
- **Missing global states:** limit reached, provider/sidecar down, login expired, estimator not calibrated, process cap reached, resuming, merge conflict, offline.
- **Inbox:** define the keys (j/k, a, r, ⏎, o, x, u) and show stale items. After approval, say when the Governor queued the agent anyway.

---

## 4. Minor findings
- **Three status vocabularies** (J4/FR-901, §6.1, §9.5): publish one mapping table.
- **Theme precedence** (system / dark-first / kitty-derived) is undefined.
- **G9 is listed before G8.** G8 (multi-provider) has no current user need: make it a C, unless D1 = A (then the API-key provider is the natural second one).
- **Setup script copies `.env` into worktrees**, so the agents send its secrets to the model. Make it opt-in, with a warning.
- **PTY streaming:** use Tauri 2 Channels with binary payloads. WebKit limits live WebGL contexts (~16), so hidden terminals need fallback rendering.
- **Monday planner:** make it an Inbox item instead of an auto-opening screen.
- **"Open in editor"** should reuse the project's existing nvim pane.
- **Accessibility:** add focus rings, throttled aria-live for streaming text, reduced motion, and font scaling.
- **Penpot token pipeline:** specify W3C token JSON → Style Dictionary → CSS variables, plus a CI drift check.

---

## 5. Proposed v1 pacing algorithm (from the pacing reviewer, endorsed)

```
every 60 s and on each usage / rate-limit signal:
  W = weekly_used()                    # observed % (statusline), else cost_est / k ± margin
  T = ideal(t) * (1 - reserve - unattributed_forecast)   # fraction of weighted working hours elapsed
  e = W - T ; eta = projected_exhaustion(W, ewma_burn)
  state = hysteresis(e, eta):          # min dwell 30 min
    GREEN  e < +3pp
    AMBER  +3..+8pp
    RED    > +8pp or eta < reset - 4h
    CATCHUP e < -10pp and < 24 working hours left
  max_live = {CATCHUP: hi, GREEN: base, AMBER: base/2, RED: 1}[state]
  5h guard: if projected 5h use > 85% → max_live = 1 ; if 5h use > 85% → interactive/critical only

admit(item) at turn/task start and at PreToolUse:
  interactive or critical → admit (draws reserve; warn if reserve < 0)
  RED and priority ≤ normal → queue (offer "defer after reset")
  live autonomous ≥ max_live → queue
  next = stride scheduling over allocation weights + priority tiers + aging (+1 tier / 2 h)
watchdog: turn > ceiling (cost or minutes) → interrupt at next tool boundary
```
**v1 ships:** concurrency, queue, pause, defer, reserve, Sprint mode as "borrowing" from later days, watchdog, explanations, shadow mode.
**Deferred:** smoothing, automatic downgrade, compaction lever, per-model estimator, off-hours auto-scheduling.

---

## 6. Suggested order of work
1. Make decisions **D1–D5**.
2. Rewrite ADR-007 with the control law (B1–B3) and the v1 algorithm; fix the branch names (B4) and the sandbox and gates (B5).
3. Shrink the phasing (D2) and make every M requirement testable.
4. Add the missing journeys and states, the keyboard contract (B7) and the attention ladder (B8).
5. Run the P0 spikes:
   - RSS per agent process
   - statusline/OTEL signals under the chosen runtime
   - PreToolUse as admission point
   - Bash sandbox scoped to the worktree
   - crash/restart reconciliation
   - kitty look in WKWebView

---

## 7. Resolution log (applied in SPEC v0.2)

### Decisions by Guillaume
| Decision | Choice | Applied in |
|---|---|---|
| D1 Runtime | **Claude Code CLI only** (Max plan; no SDK, no API key) | ADR-002, ADR-001, SPEC §1.2, §4.2, §6, §8 |
| D2 Scope | Smaller v1: P1 solo + forecast, P1b governor, P2 teams | SPEC §11 |
| D3 Team builder | **Keep the canvas** as the main editor, adding full keyboard operation, a Rules panel and inline validation | SPEC §4.3, §9.8 |
| D4 Worktrees | **Sibling folder** `<repo>.worktrees/` | ADR-003, FR-105 |
| D5 Penpot | Lighter gate: Foundations + Shell + P1 panes; states as variants; 3 prototypes; code-first backport allowed | ADR-008, SPEC §9.0 |

### Findings
| Finding | Status |
|---|---|
| B1 control law, B2 shares/priorities, B3 admission granularity | Fixed: ADR-007 rewritten (states with hysteresis, WFQ + aging, turn start + PreToolUse admission, watchdog); FR-803–812, FR-820, NFR-9 |
| B4 branch names | Fixed: `orch/solo/…`, `orch/<f>/integration`, `orch/<f>/<role>-<n>` |
| B5 sandbox & credentials | Fixed: ADR-009, FR-702–705, NFR-3 acceptance tests |
| B6 concurrency vs memory | Fixed: live-agent cap from the P0 measurement, FR-203/204, NFR-1b |
| B7 keyboard contract | Fixed: SPEC §9.7, ADR-006 |
| B8 attention ladder | Fixed: SPEC §9.6, FR-210, FR-707 |
| B9 Provider boundary | No longer applies (no sidecar); small Rust trait in §8 |
| Memory illusion / sidecar SPOF | No longer applies: no sidecar; each `claude` runs in its own process group |
| Gates pin memory | Fixed: FR-705 |
| MCP transport/delivery | Fixed: stdio `orch-mcp` + Unix socket, FR-402/403, edge meanings FR-304 |
| Integration branch owner | Fixed: FR-405, ADR-003 |
| Restart resilience overstated | Fixed: NFR-2 states what isn't recovered; reconciler J10 |
| Off-hours vs sleep | Fixed: J10, FR-816; disabled until R1 go/no-go |
| Event sourcing duplicates JSONL | Fixed: ADR-005 |
| Data model gaps | Fixed: §7 (Turn, Worktree, InboxItem, Artifact, QueueEntry, Calibration, ForecastSnapshot, provenance…) |
| SDK facts (setModel, compact, context breakdown) | No longer applies: SDK dropped; compaction removed as a lever |
| Per-model weekly limits | Added: FR-815 |
| Missing journeys | Added: J0 onboarding, J9 limit reached, J10 recovery, J11 remove project |
| Untestable wording | Fixed: success measures in §1.1, NFR acceptance tests |
| IA overlap / laptop fit | Fixed: everything is a pane, Overview merges Projects home + Monitor, breakpoints, overlay inspector, icon rail |
| Governor transparency | Fixed: FR-809/810/812, §9.4, two-window indicator |
| Chat parity gaps | No longer applies: the chat is Claude Code itself |
| Theme from kitty | Fixed: precedence and mapping rules in §9.1 |
| Status vocabularies | Fixed: single table §9.5 |
| `.env` secrets, tracked `.orchestrator/` opt-out | Fixed: FR-110, FR-109, ADR-005 |
| Compliance (R9 → R1) | Kept as the **P0 go/no-go**, now about automated use of the CLI on a subscription |

### New risks introduced by D1
- R2: hooks, statusline and flags are not a stable API, so a supported version range and contract tests are needed.
- R3: statusline data only appears after the first response of a session.
- R4: typing into idle agents through the PTY can collide with the user's own typing.
