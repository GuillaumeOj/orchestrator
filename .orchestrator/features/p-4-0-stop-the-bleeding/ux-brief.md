# P4.0 — UX brief

> Companion of [spec.md](spec.md). **No Penpot pass in P4.0**: the SPEC's P4 gate (§9.0) applies from P4.2. These are small changes inside existing Inbox card and agent pane patterns (tokens, StatusBadge, existing action rows). UX and UI are not assigned work in this phase; UI may be asked by front to review the card changes.

## Touch points
1. **Permission card** (Inbox, agent pane strip): tool name, cwd (shortened to the worktree name with full path on hover), full input in a monospace block, collapsed past 6 lines with "show all". The summary stays the title.
2. **Parked state**: a quiet marker on the card ("Parked · the agent stopped and resumes on your answer"). No "Answer in the pane" button when parked. Allow / Deny / Deny with message / Always allow (when offered).
3. **Startup card**: "<agent> did not start" / "could not resume", the PTY tail as preview (monospace, last ~12 lines), actions Open pane · Restart fresh · Dismiss (· Delete after a failed fresh restart).
4. **Restart gate**: a gate that came back after a restart shows its normal card with a "after restart" note; a legacy one shows its text and Approve/Reject, never "Spec gate".
5. **Toasts state the effect**, after the core answers: "Allowed · back-1 resumed", "Denied · reason sent", "Restarted fresh". No "Reply sent" before anything is sent.
6. **Sidebar "needs you"** counts permission and startup items too.

## States to cover
Open permission (hook held) · parked permission · startup (no SessionStart) · startup (resume failed / fresh failed) · restart gate · empty Inbox unchanged.
