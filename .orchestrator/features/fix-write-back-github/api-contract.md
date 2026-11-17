# Fix Write Back on GitHub · API contract

Backend (`back`) ↔ UI (`front`). Additions only; nothing existing is removed.

## 1. `gh_scope` errors

`SourceError` (orch-tasks `intake.rs`) gains:

```jsonc
{ "code": "gh_scope", "message": "gh needs the project scope: run `gh auth refresh -h github.com -s project`",
  "fix": "gh_scope", "scope": "project" }   // scope: "project" | "read:project" (string, whatever GitHub named)
```

`scope` is present only when `code == "gh_scope"` (skip when None). It reaches the UI wherever a `SourceError` does
(intake options, Preview, Work refresh errors).

Write-back node status (`WorkNode.writeback`, `run.source.writeback`, setting `work.writeback:<project>:<node>`):

```jsonc
{ "status": "failed", "what": "started", "at_ms": 0, "message": "…", "code": "gh_scope", "scope": "project", "op": {…} }
```

Decision `writeback_failing` payload `text`: `"<message>. Fix it in Project settings › Tracker or turn write-back off."`
(actions unchanged: `settings`, `turn_off`).

## 2. `tracker_gh_access(project: string)` → command

```jsonc
{
  "gh": "ok" | "missing" | "logged_out",   // gh_missing / gh_auth map to the Integrations fixes
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

Answer (immediately, while `gh auth refresh -h github.com -s <scope>` runs):

```jsonc
{ "state": "waiting", "code": "ABCD-1234", "url": "https://github.com/login/device", "scope": "project" }
```

Errors (as `{code, message}`): `interactive_only` (gh did not print a code without a TTY: UI shows Copy command),
`gh_missing`, `gh_auth` (not logged in: `refresh` needs a login), `failed`.

A call while a grant runs answers the running one (same code). `gh_grant_cancel()` kills it (posts `failed`,
`message: "cancelled"`).

Event `gh.grant` (core event stream, like `writeback.done`):

```jsonc
{ "state": "done" | "failed", "scope": "project", "message": "…" | null,
  "retried": { "ok": 3, "failed": 0 } | null }   // done: the result of writeback_retry_all on every GitHub-tracker project
```

## 4. `writeback_retry_all(project: string)` → command

```jsonc
{ "retried": 3, "ok": 3, "failed": 0, "last_failure": null | { "node": "…", "message": "…", "code": "…", "scope": "…" } }
```

When `failed == 0` after the run (and something was retried) the project's open `writeback_failing` decision is
resolved. The intake topic is noted so Work and settings refresh (as `wb_record` already does).
