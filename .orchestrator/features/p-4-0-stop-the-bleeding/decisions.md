# P4.0 — Decisions

| # | Date | Decision | Why |
|---|---|---|---|
| D1 | 2026-10-28 | The app uses the runtime's store-backed permission hold; the in-memory `PermissionBroker` is removed. | Two hold paths diverged: the broker kept no payload, no audit, no event, and is the cause of the pane-only prompts. One path also serves `orchd`. |
| D2 | 2026-10-28 | Hold default 300 s (SPEC FR-705), was 538 s in the app. MCP ask rules use the same budget. | SPEC value; parking makes a long hold unnecessary. The ~1 s MCP hold was a bug. |
| D3 | 2026-10-28 | A parked agent is suspended at once unless it has a reserved slot (Lead). | "Its slot is freed" (FR-705); a Lead's slot is its own. |
| D4 | 2026-10-28 | Hysteresis: a higher pressure level needs 60 s at or above it, a lower one 120 s at or below it (generalises "120 s of normal" to critical → warn). | SPEC gives warn↔normal only. |
| D5 | 2026-10-28 | One-shot allow = agent + tool + canonical `tool_input`, consumed on first match, 1 h expiry. | Exactly the parked call, nothing broader (ADR-009). |
| D6 | 2026-10-28 | Esc in the pane (EOF) is audited as `pane`; reconciler closes as `core`. | The user answered in the pane; the core closed it. |
| D7 | 2026-10-28 | The exit measurement is a run report (`orchd report`), not the weekly report. | FR-818 is not in the code; the exit criterion needs counts per run. |
| D8 | 2026-10-28 | No Penpot pass; UX and UI idle this phase. | SPEC's P4 design gate starts at P4.2; P4.0 changes stay inside existing card patterns. |
