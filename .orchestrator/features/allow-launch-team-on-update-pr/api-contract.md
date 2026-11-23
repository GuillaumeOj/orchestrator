# Allow to Launch a Team on Update PRs · API contract

Backend (`back`) ↔ UI (`front`). Additions only; an unlinked feature's shapes are unchanged (new fields absent/null).

## 1. `project_detail(id, withPrs: true)` → `prs[]`

```jsonc
{
  "number": 42, "title": "chore(deps): update rust crate tokio to v1.48", "head_ref": "renovate/tokio-1.x",
  "base_ref": "main", "state": "OPEN", "url": "https://github.com/acme/shop/pull/42", "is_draft": false,
  "review_decision": null, "checks": { "passing": 5, "failing": 1, "pending": 0, "skipped": 0 },
  // new
  "author": { "login": "renovate[bot]", "is_bot": true },          // is_bot: gh author.is_bot, or login ends with "[bot]"
  "cross_repo": false,                                              // isCrossRepository
  "head_sha": "9f2c…",                                              // headRefOid
  "feature": { "id": "f_…", "slug": "pr-42-update-tokio", "run_status": "executing" } | null,
  "launch": { "ok": true } | { "ok": false, "code": "fork" | "active_feature", "reason": "Fork PR: the app pushes to origin only" }
}
```

`feature`: the newest feature of this project whose `pr_link.number == number`. `launch.ok == false` with
`active_feature` when that feature's run is not `done`/`failed` (and no run at all counts as not active).

## 2. `pr_prefill(project: string, number: number)` → command

```jsonc
{
  "launch": {
    "project_id": "p_…", "team_id": "t_…", "slug": "pr-42-update-rust-crate-tokio", "title": "PR #42 · chore(deps): update rust crate tokio to v1.48",
    "brief": "…", "base": "main", "off_hours": false, "issues": [], "pr_mode": null,
    "pr": { "number": 42, "url": "…/pull/42", "head_ref": "renovate/tokio-1.x", "base_ref": "main", "head_sha": "9f2c…", "author": "renovate[bot]" }
  },
  "origin": { "kind": "pr", "label": "PR #42", "url": "…/pull/42" }
}
```

Errors (`errText`-able `{code, message}`): `pr_closed`, `pr_merged`, `fork`, `active_feature`, `gh_missing`,
`gh_auth`, `offline`.

Brief template (Markdown; front shows it in the textarea as is):

```
Make PR #42 pass CI: chore(deps): update rust crate tokio to v1.48
<url> · renovate/tokio-1.x → main · opened by renovate[bot]

Work on the PR's branch and push the fix to it; do not open another pull request.

## Failing checks
- test (ubuntu-latest) · <check url>
  ```
  <last lines of the failed log, or the annotations>
  ```

## PR description
<the PR body, first 4 KB>
```

No failing check → the section says "No check is failing right now (n pending)."

## 3. `team_launch(form)`: `LaunchSpec.pr`

```jsonc
"pr": { "number": 42, "url": "…", "head_ref": "renovate/tokio-1.x", "base_ref": "main", "head_sha": "9f2c…", "author": "renovate[bot]" }   // optional
```

With `pr`: `base` is ignored (the PR base is used), `pr_mode` must be null/`single`, `issues` allowed (task tracking
only). Launch errors `{code, message}`: `fork`, `pr_closed`, `active_feature`, `branch_checked_out` (message names
the path), `local_ahead` (message names the commit count), `fetch_failed`.

## 4. Feature in `app_state.features[]` and the feature detail

```jsonc
"branch": "renovate/tokio-1.x",          // ALWAYS present from now on: the real integration branch (orch/<slug>/integration when unlinked)
"pr_link": { "number": 42, "url": "…", "head_ref": "…", "base_ref": "main", "head_sha_at_link": "9f2c…", "author": "renovate[bot]" } | null
```

## 5. Run PR status (`run.pr`, `run.pr_number`, `run.pr_url`)

For a linked run, `pr_number` / `pr_url` are set at launch and `run.pr` is filled by the watcher from launch.
`run.pr` gains:

```jsonc
"linked": true,              // the PR was not opened by the app
"foreign_head": false        // the head moved outside the app since the app last saw / pushed it
```

`prs_view` (the PR card rows) shows the linked PR as `state: "open"` from launch.

## 6. Gate payload (`push_pr`) for a linked feature

```jsonc
{ "action": "push_pr", "branch": "renovate/tokio-1.x", "base": "main", "head": "…", "commits": 3,
  "linked": { "number": 42, "url": "…" }, "lease": "9f2c…", "diff": { … } }   // diff over origin/<head>...HEAD
```

Front: title "Push 3 commits to PR #42", approve label "Push". Outcome message on success "Pushed to PR #42".

## 7. Lead tools (orch-mcp)

- `rebase_base { onto?: "base" | "pr_head" }`: `pr_head` only on a linked feature (else refused).
- `request_pr`: unchanged signature; on a linked feature it asks for a push (spec §5.5). Refusal texts:
  "Remove .orchestrator/features/<slug>/ before pushing to PR #42 (it is someone else's PR)",
  "PR #42 is closed: ask the user", "No commit to push beyond PR #42's head".
