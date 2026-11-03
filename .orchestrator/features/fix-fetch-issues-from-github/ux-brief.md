# Fix Fetch Issues From GitHub · UX brief

No design pass: small additions to the existing Board, in its current visual language (inline `.form` row, card `.menu`, `Kbd` hints in the footer).

- **Sync on open:** on mount the header's Sync button shows `Syncing…` while `board_open_sync` runs; cards appear when it ends. Errors use the existing `backend disconnected: …` line.
- **Edit (`e`, or card menu "Edit…"):** the inline form row (like `n`) with the title input and, below it, a body textarea (4 rows, grows). ⏎ in the title or ⌘⏎ anywhere saves; Esc cancels and refocuses the board. The form shows the task handle (`Edit #42`).
- **New (`n`):** the same body textarea under the title.
- **Open on GitHub (`g`, or card menu "Open on GitHub"):** only when the task has `remote_url`; label says the remote's name (`Open in Linear` for Linear).
- Footer hints gain `e edit` and `g open on GitHub` (the latter only for a GitHub/Linear board).
