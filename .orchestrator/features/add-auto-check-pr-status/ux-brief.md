# Add Auto Check for PR Status · UX brief

No new screen, control or state. The PR line keeps the design of `improve-end-of-feature` (ux-brief §1, Penpot 04 Feature · PR line · states).

What changes for the user: the line updates **by itself** about once a minute while the PR is open, exactly as if ↻ had been pressed (without the busy spinner):

- `checked just now` after each background check, then `checked 1 min ago` … as today until the next one.
- A failed background check shows the existing stale state (`last checked …`).
- Merged / closed show the existing end states.
- Nothing the user is doing in the pane is reset by an update: the selected doc tab, the message draft and a pinned CI popover stay.

The ↻ button stays for an immediate check. No designer task.
