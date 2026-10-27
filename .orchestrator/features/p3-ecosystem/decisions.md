# P3 — Ecosystem · Decisions

Decisions taken during the run. Numbers D1–D11 refer to [docs/design/p3-design-notes.md](../../../docs/design/p3-design-notes.md) §0.

## 2026-10-27 — review of T-1 (UX notes)

| # | Decision | Applies to |
|---|---|---|
| D1 | **Accepted.** J12 (design → code) runs on the **Full-stack** preset. Design sprint stays design-only (ui hands off to lead). | ui (T-3), demo |
| D2 | **Accepted.** The Full-stack starter gets an edge `ui → front: hands_off_to`, shipped with the `starter_version` upgrade. Verified: `can_handoff(ui, front)` is false today (`orch-team/src/rules.rs`). | back (T-4) |
| D3 | **Accepted.** The `mcp_needs_auth` Inbox item gets a third action `reconnect`: http/sse → a pane running `claude /mcp`; connector → the claude.ai connector settings in the browser; stdio → Settings › MCP servers with the server selected. Contract §2 updated. | back (T-2), front (T-5) |
| D4 | **Accepted.** User-scheduled off-hours work uses a distinct queue reason kind **`scheduled_off_hours`** with `starts_at`. P1's `off_hours` ("outside working hours") is unchanged. Contract §6 updated. | back (T-8), front (T-10) |
| D5, D6, D11 | **Accepted** (copy/Penpot fixes): 18:00 from `starts_at`; one AC banner (P1 title kept); 05 kill key `⇧+K`. | ui (T-3) |
| D7 | **Open, for Guillaume.** The report's $ column vs the "never dollars" copy rule. Proposal: pp first, $ only as a muted "≈ API $" column with a tooltip. The contract keeps `cost_usd` either way, so nothing is blocked. | Guillaume |
| D8 | **Accepted.** Switching a project's backend to Linear when it has tasks asks first: "Create N open tasks as Linear issues" (default) or "Leave them out" (they stay local, not synced). Done tasks are never pushed. Contract §7 updated. | back (T-9), front (T-12) |
| D9 | **Accepted.** UI/UX starter prompts say "always export frames; a link-only handoff can't be read by agents". The Design tile's **Ask ui to export** sends a user message through the existing `message_send` (FR-409). | back (T-4 prompts), front (T-7) |
| D10 | **Accepted.** The MCP-needs-auth global banner is added to SPEC §9.3 in the post-P3 SPEC edit. | lead (follow-up) |

Notes on the UX notes, for ui and front (not changes to the spec):
- J12 · 2: the tasks don't exist before the spec gate (`create_task` needs the approval). The gate preview shows the **planned tasks from the lead's `request_spec_approval` summary**, as text.
- J12 · 4: the "MCP chip" in the agent pane header is not in the contract. **Out of scope for P3**; MCP status lives in Settings, the inspector, the launch dialog line and the Inbox.
- §5.5: connector + strict MCP: the compile turns strict MCP off for that role and `teams_view` reports it as a **warning** issue (not an error that blocks launch).
- §5.13 "state isn't mapped": every Linear state gets a default column from its type, so this error only appears for a state created in Linear after the mapping was saved. Keep it, with that wording.

## Team graph of this run
The P3 team itself has no `ui → front` hands-off edge either. So ui hands its frames to **lead** with `handoff` and sends front a **message** with the links and exports.
