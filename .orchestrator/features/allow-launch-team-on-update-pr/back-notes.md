# Backend notes (B0) · read on origin/main (3c4ddd5f); paths are `crates/…`

## B1 · store, launch, branch, guards, D2
- **Migration**: last on main is **24** (`features.docs_location`, `orch-store/src/migrations.rs:740`) → ours is **25**:
  `ALTER TABLE features ADD COLUMN branch TEXT; ADD COLUMN pr_link TEXT;`. `FEATURE_COLS` + row mapping
  (`orch-store/src/teams.rs:842`), `Feature` struct (:45) gets `branch: Option<String>`, `pr_link: Option<PrLink>` (serde JSON).
  `Feature { .. }` literals to fix: `team.rs:~420`, `artifacts.rs:348`, `issues.rs:491`, `notion.rs:826,1005`, store tests.
  Hand-downgrade tests pin `user_version` (`runtime.rs:4254` drops docs_location, sets 22): check them after the bump.
- **`integration_branch()`** stays the one accessor: `Feature::integration_branch` (store teams.rs:49) → `branch` or
  `orch/<slug>/integration`; `FeatureCtx` (orch-team compile.rs:141) gets `branch: Option<String>` filled in `RunCtx::fctx`
  (team.rs:~90). Callers are already routed through those two (22 sites: artifacts.rs:166,188; flow.rs:159; issue_build.rs:102,146,213;
  issue_pr.rs:162; issues.rs:253; overwatch/chatw.rs:333; policy/rt.rs:182; pr.rs:290,522; rebase.rs:81,134; team.rs:885,946,1747,
  1947,1985,2075,2097,2171; orch-app teams.rs:88 (`app_state.features[].branch`: always present, add `pr_link`); orch-team
  compile.rs:278,325,378) → no per-site change except where a literal `orch/{slug}/…` prefix is built (below). orchd CLI: only doc text
  (`orch-protocol/src/ctl.rs:12`); no code derives it.
- **Worktree layout** (`orch-git/src/worktree.rs:57`): `layout(Integration{feature})` hard-codes the branch. Change
  `WorktreeKindSpec::Integration { feature, branch: Option<String> }` (`orch-protocol/src/ctl.rs:9`; `#[serde(default)]`), branch
  validated with `git check-ref-format`, path unchanged. `ensure(.., base, ..)` gets a separate `start: Option<&str>`; `WorktreeRow`
  keeps `base` = PR base. `complete()` (:200) runs `worktree add -b <branch> <path> <row.base>` – also called by the reconciler from the
  row alone, so a linked row needs its start ref persisted or derived (`origin/<branch>` when `row.branch != orch/*` and no local
  branch), else a crash-resume would cut the PR branch from the base. Fast-forward / ahead-refused logic lives in the launch (team.rs) before `ensure`.
- **Launch** (`agent-runtime/src/team.rs:355` `launch`/`launch_run`): `LaunchSpec.pr: Option<PrLink>` (serde default). Refusals before
  anything is stored (next to the issues/mode checks, :388): fork (`link.cross_repo` is not in the link: re-check with `gh pr view`
  in step 2), `issues`+per-issue, active feature (D3; `db.feature_by_pr(project, n)` + `runs()` state), head checked out elsewhere
  (`orch_git::worktree::list`, incl. main checkout), `local_ahead` (`ahead_behind(local, origin/<head>)`), `fetch_failed`
  (`gh::fetch_branches`). Then the `Feature` literal (:411): `branch=Some(head)`, `base=pr.base_ref`, `pr_link`; run `pr_number/pr_url`
  set at creation (:~470 `TeamRun {pr_number: None..}`) and **`status` stays `spec`** (D5). An existing feature relaunched (:405 arm) must
  keep its link (it does: `..f`). Existing feature with a *different* link → refuse.
- **Merge guard** `team.rs:1985`: `prefix = orch/<slug>/` + `branch == integration_branch()` already compare by accessor → correct as-is
  for `renovate/foo`; just add a test. Specialist branches (`orch/<slug>/<node>-<n>`, team.rs:882 `create_worktree(Team{..})`) unchanged.
- **Protected branch**: `policy/rt.rs:182` already pushes `f.integration_branch()`; nothing but a test.
- **D2**: `orch-git/src/cleanup.rs:403` (`remove` → `-D` after `keep_unmerged`/`is_merged(base)`) and `delete_branch` (:424) delete
  by "merged into base". Add a `protect: Option<(&str /*remote ref*/)>`/`Confirm` rule: for a linked branch keep unless `is_merged(branch,
  origin/<head>)` (log "kept renovate/foo: n commit(s) not pushed to PR #n"); never remote. Callers: `delete_feature`
  (team.rs:1466), `prune_worktrees` (projects.rs:220), `remove_project` (:401), `remove_worktree_of`. They read rows, not features: pass
  the linked branch set from `db.features(project)`.

## B2 · PR list, prefill
- `orch-git/src/gh.rs:45` `PullRequest` + `parse_pr_list` (:299) + `prs()` (:336, JSON fields list): add `author{login,is_bot}`
  (`author.is_bot` or login ends `[bot]`), `cross_repo`, `head_sha`, `head_owner`; `--json …,author,isCrossRepository,headRefOid,headRepositoryOwner`.
- `projects.rs:104` `project_detail`: after `prs()`, annotate each PR with `feature` (newest feature of the project with `pr_link.number==n`;
  id, slug, run status) and `launch {ok|code|reason}` (fork / active_feature). Serialize as `json!(prs)` today → build values, not the struct.
- `pr_prefill(project, number)`: new `Runtime` method (new file `pr_prefill.rs`, mirror `intake/start.rs:214 prefill_forms`, which
  builds the launch form + team/slug uniqueness) + `orch-app` command (`commands.rs`, `core.rs` like `project_detail`:970/1184) +
  orchd verb if `ctl.rs` mirrors commands. Calls: `gh pr view <n> --json …,body,statusCheckRollup`, per failed GitHub Actions run
  `gh run view <id> --log-failed` (tails: 60 lines/job, 4 jobs, 12 KB), annotations fallback `gh api repos/{o}/{r}/check-runs/{id}/annotations`
  (needs `gh` read only; sandbox: gh is user-side, the app runs it). Error codes as contract §2 (`gh_missing`/`gh_auth` from `gh::available`).

## B3 · push gate, watcher, rebase, prompt
- **`request_pr`** `inbox.rs:1162`: after the per-issue branch and `open_pr_run` (→ `request_pr_update`), add `if linked { return
  self.request_pr_linked(rec, cx) }` before `pr_text(args)` (title/body unused). Checks: closed link, D1 docs in `origin/<base>...HEAD`
  (only when `FeatureDocs::of(f).docs` is a repo dir), commits beyond `pr_link.head_sha`/last seen. Gate payload `action:"push_pr"` + `linked`,
  `lease`, `diff_summary(wt, "origin/<head>", branch)`. Gate title/copy: `attention.rs:881 copy::pr_gate`, `decision.rs:301` (kind/lane), `decision_patch` range.
- **Approval** `inbox.rs:1017` → `open_pr` (:1234): branch on `p["linked"]`: `gh pr view` (open, head==lease else refuse with the
  contract text) → `gh::push_with_lease` (plain `push_rev` if lease is an ancestor) → `self.pr_opened(agent, Some(linked url))` (pr.rs:508; the
  url has `/pull/N`, so no gh lookup; it resets `run.pr=None`: keep `linked`/`pushed_sha`) → `pr_check_now(.., pushed)`. Skip `gh_create_pr`/`cfg.open_pr`
  entirely. Audit `pr_push_linked`. `pr_write_back` (title/issue ids) not used.
- **Watcher** `pr.rs:164 pr_tick` only takes `effective(r)==PrOpen`; extend to runs with `feature.pr_link` and state not done/failed
  (`pr_check` already needs only `run.pr_number`, set at launch). `pr_check` (:300) applies changes only in `PrOpen` (:384): add a linked
  pre-`pr_open` branch handling only Closed/Merged/foreign head. Foreign head: `PrStatus` (orch-store, JSON on run) gets `linked: bool`,
  `foreign_head: bool`; detect `view.head_sha != old.head_sha && != pushed_sha` and not merged into HEAD; one `deliver_user` (dedup by
  stored flag). Closed linked (`pr_apply` :~405 text "mcp__orch__request_pr opens a new pull request") → linked text; `request_pr` refuses while closed.
  `prs_view`: linked PR `state:"open"` from launch (find it: `G prs_view`).
- **`rebase_base`** `rebase.rs:20`: arg `onto` (`orch-mcp/src/lib.rs:207` schema + README:19, `team.rs:1972` passes args). `pr_head`: refuse unless linked;
  `fetch_branches([head])`; target `origin/<head>`; FF when `merge-base --is-ancestor HEAD origin/<head>`; rest unchanged. State check
  (:~28) allows Integrating/PrOpen only: a fresh linked run in `executing` cannot bring the head in; spec says "before asking": fine, but see G4.
- **Lead prompt**: `orch-team/src/compile.rs:~270–400` (`## Team` block, lead rules) + `team.rs:519 lead_brief` for the PR sentence; `FeatureCtx`
  gets `pr: Option<(number,url,head,base,author)>`. D1 docs check is moot if G1 is taken.

## Gaps (for lead)
- **G1** D1 is avoidable: main already has `docs_location` (change-how-specs-are-saved; `team.rs:427`, `feature_docs.rs`). A linked feature can
  force the **app folder** for docs and designs (`Loc::App`) → no `.orchestrator/features/<slug>` ever enters the PR; drop the remove-before-push dance
  and the `request_pr` refusal (keep only a guard: repo docs in diff → refuse). Needs the spec gate to read app docs (already does).
- **G2** `LaunchSpec.pr` has no `cross_repo`; the launch must re-read the PR (`gh pr view`) – state it so front need not send it.
- **G3** Gate `lease` "last fetch": the head last seen is `run.pr.head_sha` (watcher), not a fetch; define = `run.pr.head_sha`, fallback `pr_link.head_sha_at_link`.
- **G4** `rebase_base` is refused outside integrating/pr_open and with open tasks; a foreign head noticed in `executing` can't be brought in until
  the tasks merge. Accept (tell the lead) or allow `pr_head` in `executing` with the same "no open task" rule.
- **G5** Worktree reconciler (`worktree.rs complete`) needs the start ref (see B1); contract silent. Persist start ref or derive from `origin/<branch>`.
- **G6** Spec §5.6 "reset when the branch contains it again": define as `is_merged(head_sha, HEAD)`; pr_opened resets `run.pr=None` so
  `foreign_head` must be recomputed on the next tick (no stored flag loss on push).
- **G7** Per-PR uniqueness is by `pr_link.number` within a project; a same-slug feature relaunched on another PR is refused (above); contract §1 `feature` = newest by `created_at_ms`.
- **G8** `branch` ALWAYS present in `app_state.features[]` (contract §4) – confirm `Feature` serializer used by the snapshot (orch-app teams.rs:88 builds it by hand, so add there).
