# P4.1 — Decisions

| # | Date | Decision | Why |
|---|---|---|---|
| D1 | 2026-10-29 | One Tauri event `now` carries attention deltas, decision transitions, new outcomes and **invalidation topics**; panes refetch their views on their topic instead of polling. `app_state` stays as a command, fetched on topics. | Exit criterion "no per-pane polling left" without moving every pane view into one giant pushed snapshot. |
| D2 | 2026-10-29 | Attention is recomputed by a pure function when a `dirty` flag is set (checked on the 200 ms pump) and on the 30 s tick; only changed records are sent. | ~20 mutation sites write agent state directly; a dirty flag + diff is robust to missing one, and cheap at this scale. |
| D3 | 2026-10-29 | A blocked or failed startup is level `error` with `waiting_on.kind = startup`, not `needs_you`. | ADR-011: `needs_you` ⇔ kind `user`. The startup decision is still counted in `decisions` (level 2), so the badge shows it. |
| D4 | 2026-10-29 | `external` = an MCP tool call running ≥ 5 min. | The only external wait seen in real runs (Penpot tab asleep, 44 min); no other external signal exists. |
| D5 | 2026-10-29 | Agent-on-agent waits are inferred from review requests, undecided plans, unanswered messages/handoffs, and a Lead's in-progress tasks (spec §3.1 rule 7). | Not modelled anywhere today; these are the cases the 2026-10-27 run showed. |
| D6 | 2026-10-29 | Synthesized items (trust, pane prompt, agent error, Governor ask) stay synthesized but get payloads, blocks and subject; stored kinds get the full lifecycle. | They mirror live state that already has its own source; storing them would duplicate it. |
| D7 | 2026-10-29 | `since_ms` with no stored source restarts at core boot. | Persisting every derived wait start is not worth a table; the stored sources (decisions, queue, tasks, turns) cover the long waits. |
| D8 | 2026-10-29 | The digest shows outcomes (tasks, merges, PRs, run phases); a solo session's turn is an outcome with its last assistant message. Team members' turns are not. | Brief: the digest showed prompts sent, not results. Solo sessions have no task to report done. |
| D9 | 2026-10-29 | The Inbox pane stays in P4.1 and renders Decisions; `inbox_outcome` is kept one phase, deprecated. | P4.3 removes the pane; P4.1 must not break triage meanwhile. |
| D10 | 2026-10-29 | UX and UI design components only (chip, palettes, context blocks, lifecycle marker, outcome row), not screens; the P4 Penpot gate stays before P4.2. | P4.1 has no layout change, but ships new visual elements that P4.2/P4.3 reuse. |
| D11 | 2026-10-29 | Reason strings follow T-23's notes: no status label repeated, no duration (UI appends `since`); every outcome has core-written `text` and `feature`. | One composed line per §9.5 (badge + reason + since) without doubled words; finished runs still name their feature. |
| D12 | 2026-10-29 | The weekly-report and task-sync-conflict info items become outcome kinds `report_ready` and `sync_conflict`; no level-4 item remains outside outcomes. | Level 4 = outcomes only; both were already silent info. |
