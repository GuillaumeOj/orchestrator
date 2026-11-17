# gh behaviour notes · fix-write-back-github (T-450)

Verified here: `gh 2.104.0` (`--help`, and runs with `GH_CONFIG_DIR=$TMPDIR/x`; the sandbox blocks `~/.config/gh`, so
no real login). **[V]** = run here, **[S]** = from gh's source/docs as I know it, not run (no login, no network
to GitHub): the implementation task must confirm on a real login before relying on it.

## 1. `gh auth refresh -h github.com -s project` without a TTY

- **[V]** `-s/--scopes` adds scopes and keeps the existing ones ("If no scopes are provided, the command maintains
  previously added scopes"). `-h github.com` is given, so gh never asks for the host.
- **[V]** `GH_TOKEN` set: prints to **stderr** `The value of the GH_TOKEN environment variable is being used for
  authentication.` / `To refresh credentials stored in GitHub CLI, first clear the value from the environment.`, exit **1**,
  immediately. Same for `GITHUB_TOKEN`. → `interactive_only`/Copy-command path (spec §4.3).
- **[V]** Not logged in: stderr `not logged in to any hosts. Use 'gh auth login' to authenticate with a host`, exit 1.
- **[S]** Flow: device flow. gh prints, all on **stderr** (stdout stays empty):
  `! First copy your one-time code: ABCD-1234` then, when not interactive, `Open this URL to continue in your web
  browser: https://github.com/login/device` (interactive: `Press Enter to open github.com in your browser... `).
  Interactive = stdin **and** stdout are TTYs and `GH_PROMPT_DISABLED` unset. With stdin closed / piped or
  `GH_PROMPT_DISABLED=1`, gh skips the Enter wait and the browser launch, prints the URL and polls GitHub until the
  user authorizes. So: spawn with stdin `Stdio::null()`, stdout+stderr piped, `GH_PROMPT_DISABLED=1`, `NO_COLOR=1`,
  and match `[A-Z0-9]{4}-[A-Z0-9]{4}` and `https://\S+` on both streams (do not depend on the exact sentence).
- **[S]** End: success prints `✓ Authentication complete.` (and `- gh config set -h github.com git_protocol https` style
  lines are only for `login`), exit **0**; the new token replaces the stored one (keyring or hosts.yml). Expired code
  (≈15 min) or denied: exit **1** with `authentication timed out` / `access_denied`-style text on stderr. Killed
  (SIGKILL on our timeout) leaves the old token untouched. gh's own exit 2 = cancelled (Ctrl-C), 4 = needs auth.
- Decision for impl: treat exit 0 = `done`, anything else = `failed` with the last stderr line; if no code appears within
  ~10 s of spawn (or exit 1 before any code), answer `interactive_only` and the UI shows Copy command.

## 2. `gh auth status` and the scopes line

Since gh ≥ 2.40 (multi-account), everything goes to **stdout**, exit 0 when logged in, 1 when not (older gh: stderr; the
existing `SystemNet::gh_auth_status` already concatenates both):

```
github.com
  ✓ Logged in to github.com account octocat (keyring)
  - Active account: true
  - Git operation protocol: https
  - Token: gho_************************************
  - Token scopes: 'gist', 'read:org', 'repo', 'workflow'
```
- Scopes are single-quoted and comma-separated (older gh ≤ 2.3x: `✓ Token scopes: gist, read:org, repo`, no quotes, `✓`
  or `-` prefix). Parse tolerant: find the line containing `Token scopes:`, take the text after it, split on `,`, trim
  whitespace and `'`/`"`. Several accounts/hosts: one block per host; take the block of `github.com` with
  `Active account: true`.
- **No `Token scopes:` line** (or `Token scopes: none`) for fine-grained PATs, GitHub App/Actions tokens and some
  `GH_TOKEN` values: `granted: null`. A classic token from `GH_TOKEN` still lists scopes, with `(GH_TOKEN)` instead of
  `(keyring)` in the "Logged in" line → use that suffix to prefer Copy command ("clear GH_TOKEN"). Logged out:
  `You are not logged into any GitHub hosts. To log in, run: gh auth login`, exit 1 **[V]** → code `gh_auth`.
- `project` implies `read:project`: a login with `project` has both only if GitHub lists both; treat `project` as
  satisfying a `read:project` need, never the reverse. `repo` does not cover Projects v2.

## 3. Insufficient-scopes GraphQL text

`gh api graphql` prints the JSON body and exits **1** on a GraphQL `errors` answer (gh ≥ 2.x: stdout keeps the JSON,
stderr gets `gh: <first message>`), HTTP status is 200. The write path (`gh_graphql` → `gh_json`) therefore sees an
`Err` from `Gh::run` whose text is the stderr line; the `errors[]` loop is only reached by gh versions that exit 0.
Both must classify (`INSUFFICIENT_SCOPES` / `required one of the following scopes`). Shape **[S]**:
```json
{"data":{"addProjectV2ItemById":null},"errors":[{"type":"INSUFFICIENT_SCOPES","path":["addProjectV2ItemById"],
 "locations":[{"line":1,"column":50}],
 "message":"Your token has not been granted the required scopes to execute this query. The 'addProjectV2ItemById' field requires one of the following scopes: ['project'], but your token has only been granted the: ['gist', 'read:org', 'read:project', 'repo'] scopes. Please modify your token's scopes at: https://github.com/settings/tokens."}]}
```
- `updateProjectV2ItemFieldValue`: same text with that field name, `['project']`.
- Board **read** (`repositoryOwner{projectV2}`): `The 'projectV2' field requires one of the following scopes:
  ['read:project']` (a `project` token passes it too). Read errors may also come with `data` partly filled and
  type `INSUFFICIENT_SCOPES`; some endpoints say `Resource not accessible by personal access token` (type
  `FORBIDDEN`, fine-grained tokens): not a scope fix → `gh_write`.
- Parse: regex `requires one of the following scopes: \[([^\]]*)\]` → first quoted name; the second bracket list is
  the *granted* set and must not be read (it holds `read:project`, which is the bug in `gh_error`). When no required
  list is found: `read:project` for reads, `project` for writes.

## 4. Where the app spawns gh; a long-running child

- `orch_git::gh::gh(dir, args)` (crates/orch-git/src/gh.rs) = `Command::new("gh")` + `run_with_deadline` (30 s),
  blocking; `Gh` trait in orch-tasks `github.rs` wraps it (`GhCli`); tests use fake `Gh`.
- `Net::gh_auth_status` in agent-runtime `net.rs` (trait `Net`, `SystemNet`, fake with `gh: Mutex<Option<Result>>`):
  `gh auth status`, env `GH_PROMPT_DISABLED=1 NO_COLOR=1`, 15 s deadline. Add `gh_auth_refresh(scope)` here (returns a
  child handle / spawns the reader) so tests inject a fake that yields code+URL lines and an exit status.
- Long child: `std::thread::spawn` like `wb_spawn` (writeback.rs): spawn, read stdout/stderr lines until the code and
  URL are seen, send `{code,url}` back through a channel to the command, keep the thread waiting for exit (poll
  `try_wait` + 15-minute deadline, then `kill`). Post `gh.grant` with `self.event("-", "gh.grant", json!({...}))`
  (same call as `writeback.done`); keep the running grant in a `Mutex<Option<Grant>>` for "one at a time". On `done`,
  call `writeback_retry_all`.
- Write-back failures: `wb_record` stores `{status, what, code, message}` at `status_key`, counts `fails_key`, opens the
  `writeback_failing` inbox item at `FAILING_AFTER`; `writeback_retry_all` must reset/resolve those (spec §4.4).
