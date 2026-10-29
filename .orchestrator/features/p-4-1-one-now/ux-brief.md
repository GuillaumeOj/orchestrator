# P4.1 — UX brief

> Companion of [spec.md](spec.md). **No new screen and no layout change**: the existing panes render the core's attention records and the Inbox renders Decisions. The full P4 Penpot gate (§9.0: new shell, Mission control, cards storyboard) is due before P4.2, not here. P4.1 designs only the pieces it ships, as **components** in the existing Penpot file, so P4.2/P4.3 reuse them.

## 1. Principles applied (SPEC §9.1)
- **One source of state** (10): a surface shows the record's `status`, `reason` and `since`, and the core's counts. No surface words a wait differently.
- **Every number says what it is** (11): "2 need you", "3 live / cap 3", "queued 2", "waiting 12 min", "+142 −9 lines", "3 files changed". No bare `(+2)`, `±31`, `7%`.
- **Nothing claims before it happens**: a toast or marker shows *sent* only after the core sent, and the effect only when it happened.

## 2. Copy — the `reason` strings (core-written; UX owns the wording)

> **Superseded by `docs/design/p4-design-notes.md` §0 and §2 (T-23)**: the reason never repeats the status label (the surface shows the badge before it) and carries no duration (the UI appends `since`); clock times stay. The table below is the first draft.
| waiting_on.kind | Pattern | Examples |
|---|---|---|
| user | `<what>: <subject>` | "permission: Bash pnpm add zod" · "question: which auth provider?" · "spec gate · checkout" · "PR gate · checkout" · "trust dialog in its pane" · "a prompt in its pane" · "Run now? ≈ +2 pp" |
| agent | `→ <agent> · <what>` | "→ lead · review of T-4" · "→ lead · plan for T-6" · "→ back-1 · message" · "→ back-1, front-1 · T-3, T-4" |
| task | `blocked by T-<n> (<owner>, <status>)` | "blocked by T-2 (back-1, in progress)" |
| governor | `Queued · <reason> · <position> · ≈<eta>` | "Queued · live cap 3/3 · 2nd · ≈14:20" · "Queued · 5h guard · until 13:00" · "Not started · live cap 3/3 · Start now ≈ +1 pp" · "Paused by you" · "Paused · Pause all" |
| limit | `<window> full · resets <time>` | "5h window full · resets 16:00" · "week full · resets Mon 09:00" |
| startup | `Starting · <s> s` / `Not started · <why>` | "Starting · 12 s" · "Not started · no session after 30 s" · "Could not resume · restarted fresh failed" |
| external | `Waiting · <server> <tool> · <duration>` | "Waiting · penpot execute_code · 7 min" |
| (running) activity | `<tool> <short input>` / `writing` | "Edit src/feed.ts" · "Bash cargo test" · "writing" |

`since` is shown as a duration next to the reason ("· 12 min"), in the UI's relative-time format. Times are local, 24 h.

Toasts (effect, after the core says so): "Allowed · back-1 resumed" · "Denied · reason sent" · "Answer sent · front-1 resumed" · "Spec approved · lead resumed" · "PR opened ↗" · "Queued · starts ≈14:20" · "Restarted fresh" · "Failed: <why>".

Outcome rows: "T-4 done · checkout API (back-1)" · "Merged back-1 into integration" · "PR opened · checkout ↗" · "checkout: planning → executing" · "checkout done" · solo: "blog › fix-feed: Fixed the feed pagination…".

## 3. Components to design (Penpot, Foundations + Components pages)
1. **Palettes as tokens** (SPEC §9.5 "Separate palettes"): agent status (exists: check `not_started` has its own), **attention level** (7), **run phase** (11), **decision kind** (permission, question, gate·spec, gate·pr, gate·restart, startup, ask, budget, trust, wait, error/alert, notion), **decision lifecycle** (open, parked, sent, effect, failed, stale). None borrows another's color/icon. New tokens go to `docs/design/tokens.json`.
2. **Waiting-on chip**: icon per kind + reason + duration; compact (sidebar/rail tooltip, Overview row) and full (agent header, Inspector). Overflow: ellipsis with the full text on hover.
3. **Context blocks**: tool call (tool name, cwd shortened to the worktree name, input monospace collapsed past 6 lines — P4.0 pattern), markdown, docs list with line counts, diff summary (per-file +/− bars, totals, "show patch"), options (several questions, multi-select), last message, PTY tail (monospace, last ~12 lines), cost (pp of week and 5h, ETA), countdown ("applies in 18 min"), error.
4. **Lifecycle marker** on an Inbox detail: open · parked · sent (spinner-free text) · effect · failed (with why) · stale (with why).
5. **Outcome row** (digest) with its kind icon, and the digest's **Clear** action.
6. **Count labels**: title badge, sidebar feature badge, rail dot, status bar figures with units.

## 4. States to cover per surface
Sidebar, icon rail, Overview row, agent pane header + strip, Inspector, status bar: one example per `level` and per `waiting_on.kind`, plus `not_started`, `paused`, `suspended`, external ≥ 5 min. Inbox: every decision kind open; permission parked; decided → sent → effect; failed; stale; empty digest; digest with 5 outcomes.

## 5. Who does what
- `ux` → validates and completes the copy in §2 (every Governor reason of `QueueReason`, every decision kind's title pattern), the per-surface states matrix (§4) as annotations; notes in `docs/design/p4-design-notes.md`. Hands off to `ui`.
- `ui` → §3 in Penpot (components, not screens), `00 Cover` status **In review**, exports under `.orchestrator/features/p-4-1-one-now/design/` (one PNG per component sheet), new tokens in `docs/design/tokens.json`, `docs/design/README.md` rows for P4.1. Hands off to `front`; reviews front's surfaces on request.
