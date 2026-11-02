# Fix Team Getting Stuck · UX brief

> Companion of [spec.md](spec.md) and [api-contract.md](api-contract.md). **No FigCraft pass**: these are small changes inside existing components. They use existing tokens, `StatusBadge`, `WaitingChip`, `AgentChip`, the decision card layout of the `startup` kind, and `ReportPane` sections. UX and UI are not assigned work; `ui` reviews front's task.

## Principle
A stuck message must be **visible where the user already looks**, with its reason in plain words and one action that fixes it. When delivery works, nothing new shows: no badges for messages that arrive within a minute.

## Touch points
1. **Delivery decision card** (chat column and Inbox, same as the `startup` card).
   - Title: "back-1 did not receive 2 messages", or "lead-1 looks stuck".
   - Reason line from the blocker label (api-contract §1), plus "since 12 min".
   - The message list: from, kind, a 1-line preview, and attempts when more than 1.
   - The PTY tail, monospace, last ~12 lines, expandable.
   - Actions: **Open pane** (primary when the blocker is `no_ack` or `user_typing`), **Retry now** (primary otherwise), Restart fresh, Dismiss.
   - The toast states the effect: "Retrying · back-1", "Restarted fresh".
2. **Undelivered chip** on the agent pane strip and the mission control `MemberChip`: "2 undelivered · held: off hours". Muted when the wait is under 10 min, warning colour after. Clicking it opens the decision if there is one, else the agent's timeline filtered to Delivery.
3. **Timeline** (FeaturePane): a small state glyph after each message line.
   - queued ○, handed ◐, seen ●, read ✓, failed ✕.
   - "×n" when redelivered.
   - Tooltip: the times, `via`, and the blocker.
   - New lines for `redelivered`, `recovered`, `stuck` and `delivery_failed`, all under a new "Delivery" filter.
4. **Report pane**: a "Delivery" section shaped like the existing sections. Counts, p50/p95 latency, and three small tables (redelivered by cause, recovered by state, stuck by kind).

## States to cover
- No undelivered messages (nothing shown).
- Waiting under 1 min (nothing).
- Waiting 1–10 min (muted chip).
- Waiting over 10 min (warning chip + card).
- Failed after 3 attempts (card, ✕ in the timeline).
- Recovered (timeline line only).
- Lead stuck (card addressed to the user).
- A report with zero deliveries (section shows "No team messages in this run").
