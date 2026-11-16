# add-missing-commits: API contract

No API change. The recovered commits change no command, event, MCP tool or payload.

- `orch-store`: `Store::upsert_run` no longer writes `prs` on an existing run (it still does
  on insert). `prs` changes only through `Store::update_run_prs`, which already exists on
  `main`. Callers on `main` already use `update_run_prs` for PR changes; only tests wrote
  `prs` through `upsert_run`, and the commit updates them.
- UI: a comment-only change in `TaskIssueChip.svelte`.
