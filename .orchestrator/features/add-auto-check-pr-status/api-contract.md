# Add Auto Check for PR Status · API contract

No new command, field or event. One change to an existing contract:

## `now` event · `topics`

| Write | Topics noted (before → after) |
|---|---|
| `Store::set_run_pr` (every PR check: background `pr_tick` every 60 s, `pr_refresh`, the check after an update push) | none → `feature:<run.feature_id>` |

The Feature pane (`onTopics(['feature:<id>', …], load)`) refetches `feature_view` on it; `feature_view.run.pr` carries the stored `PrStatus` unchanged (`state`, `head_sha`, `base`, `ci`, `position`, `unpushed`, `rewritten`, `pushed_sha`, `checked_at_ms`, `error`).

`pr_refresh(feature)` is unchanged: it still returns the checked run (the PR line applies it at once), and now also raises the topic.
