# Fix Write Back on GitHub · API contract

Backend (`back`) ↔ UI (`front`). Additions only; nothing existing is removed.

## 1. `gh_scope` errors

`SourceError` (orch-tasks `intake.rs`) gains:

```jsonc
{ "code": "gh_scope", "message": "gh needs the project scope: run `gh auth refresh -h github.com -s project`",
  "fix": "gh_scope", "scope": "project" }   // scope: "project" | "read:project" (string, whatever GitHub named)
```

`scope` is present only when `code == "gh_scope"` (skip when None). It is parsed from GitHub's raw text before
`SourceError::new` clips the message to `MESSAGE_MAX`. It reaches the UI wherever a `SourceError` does
(intake options, Preview, Work refresh errors).

Write-back node status (`WorkNode.writeback`, `run.source.writeback`, setting `work.writeback:<project>:<node>`).
`wb_record` must store `scope` next to `code`, and `wb_statuses` (the projection both views read) must pass `code`,
`scope` and `op` through: today it copies only `status`, `what`, `at_ms` and `message`, so the UI never sees `code`:

```jsonc
{ "status": "failed", "what": "started", "at_ms": 0, "message": "…", "code": "gh_scope", "scope": "project", "op": {…} }
```

Decision `writeback_failing` payload `text`: `"<message>. Fix it in Project settings › Tracker or turn write-back off."`
(actions unchanged: `settings`, `turn_off`).

## 2. `tracker_gh_access(project: string)` → command

```jsonc
{
  "gh": "ok" | "missing" | "logged_out" | "offline",   // gh_missing / gh_auth map to the Integrations fixes; `gh auth status` fails offline too: do not call that logged out
  "account": "octocat" | null,
  "needed": ["read:project", "project"],    // from the saved tracker: board set → read:project; + write-back on → project
  "granted": ["repo", "read:org", "read:project"] | null,   // null: gh lists no scopes (fine-grained / GH_TOKEN)
  "missing": ["project"],                   // needed − granted ("project" covers "read:project"); [] when granted is null
  "failed_writes": 3,                       // nodes of the project with writeback.status == failed
  "last_failure": { "node": "gh:acme/shop#12", "message": "…", "code": "gh_scope", "scope": "project" } | null
}
```

Errors: `{code: "not_found"}` for an unknown project. Not a GitHub tracker → `needed: []`, `missing: []`.

## 3. `gh_grant_scope(scope: string)` → command, `gh.grant` event

`scope` must be `project` or `read:project`; anything else answers `{code: "bad_scope"}` (it is passed to
`gh auth refresh -s`, where a comma or a leading `-` would add scopes or flags). The scope in a `gh_scope` error comes
from GitHub's text: the UI only offers Grant access for these two.

Answer (immediately, while `gh auth refresh -h github.com -s <scope>` runs):

```jsonc
{ "state": "waiting", "code": "ABCD-1234", "url": "https://github.com/login/device", "scope": "project" }
```

Errors (as `{code, message}`): `interactive_only` (gh did not print a code without a TTY: UI shows Copy command),
`gh_missing`, `gh_auth` (not logged in: `refresh` needs a login), `failed`.

A call while a grant runs answers the running one (same code; its `scope` tells the caller when it is another scope than asked). `gh_grant_cancel()` kills it (posts `failed`,
`message: "cancelled"`).

Event `gh.grant` (core event stream, like `writeback.done`):

```jsonc
{ "state": "done" | "failed", "scope": "project", "message": "…" | null,
  "retried": { "<project id>": { "ok": 3, "failed": 0 } } | null }   // done: writeback_retry_all of every GitHub-tracker project, by project (the grant has no project argument)
```

## 4. `writeback_retry_all(project: string)` → command

```jsonc
{ "retried": 3, "ok": 3, "failed": 0, "last_failure": null | { "node": "…", "message": "…", "code": "…", "scope": "…" } }
```

When `failed == 0` after the run the project's open `writeback_failing` decision is resolved, also when nothing was
retried (the decision can be stale). `writeback_retry` (one node) does the same when its success leaves no failed node. The intake topic is noted so Work and settings refresh (as `wb_record` already does).
