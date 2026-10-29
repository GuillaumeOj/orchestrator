//! `orchd`: the headless core for development. See README.md.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use agent_runtime::paths::AppPaths;
use agent_runtime::runtime::{Runtime, RuntimeConfig};
use agent_runtime::server::{self, request};
use anyhow::{Context, Result, anyhow, bail};
use orch_protocol::{AgentSpec, CtlRequest, InboxDecision, WorktreeKindSpec, bus};

const USAGE: &str = "\
orchd: headless Orchestrator core (dev)

usage: orchd [--home DIR] <command> [args]

  serve [--enforce] [--no-recover] [--cap N] [--hold-secs N] [--claude PATH]
        [--otel URL | --no-otel] [--permission-hold-secs N] [--idle-min N] [--fetch-min N]
                                run the core in the foreground (Ctrl-C suspends every agent)
  install-helpers               copy orch-hook, orch-status and orch-mcp next to this binary into <home>/bin
  reconcile                     dry run of the startup reconciler (core not running)
  worktree <repo> solo <slug> | integration <feature> | team <feature> <role> <n>  [--base B]
  start <agent> --repo <path> [--slug S] [--base B] [--model M] [--effort E] [--mode manual|acceptEdits|auto|dontAsk|plan]
                [--prompt TEXT] [--strict-mcp] [--allow-host H]... [--allow RULE]... [--tier low|normal|high|critical] [--message TEXT]
  send <agent> <text...>        queue a message (admitted by the Governor, delivered when idle)
  attach <agent>                connect this terminal to the agent's PTY (detach: Ctrl-])
  interrupt <agent>             Esc
  suspend <agent>               stop the process, keep the session
  resume <agent> [--message TEXT]
  kill <agent>                  process group + processes whose cwd is in the worktree
  pause-all | resume-all        Governor Pause all (FR-811): autonomous work holds at its next tool call
  pause <agent> | unpause <agent>         pause one agent's autonomous work (until unpause)
  run-now <entry> | cancel <entry>        a queued entry (ids in `governor`): run it past the caps, or drop it
  undo <action> | sprint [pp] | answer-ask <action> apply|skip
  enforce on|off                shadow mode off (enforce) or back on (FR-812)
  report [--run <id>] [--since <iso>] [--until <iso>] [--json] [--baseline]
                                the run report: prompts by tool and decider, pane interventions, waiting
  state | usage | governor      print JSON
  now [--json] [--run <id>]     agents (level/status, reason, since), then runs (phase, level, blocker)
  stop                          shut the core down (suspends agents)

  projects                      registered projects with git status
  project add <path> | clone <url> <parent> [--name N] | show <id> [--prs] | fetch <id>
  project relocate <id> <new-path>        after a repo move: repair worktree links, re-lock
  project remove-plan <id>                J11: worktrees, branches, agents
  project remove <id> --history delete|archive [--discard PATH]... [--keep-merged]
  pr refresh <feature>          check the PR of a feature's latest team run now (id or slug)
  diff <worktree> [--against BRANCH]
  worktree-rm <project> <path> [--delete-branch] [--discard]
  commit <agent> <message...> [--path P]...   commit in the agent's worktree (same path as orch-mcp)
  inbox                         pending Inbox items
  decide <item> approve|reject|always|reply|dismiss [--scope project|role|agent] [--message TEXT]
  rules | rule add <project|role|agent> <id> <pattern> <allow|deny|ask> | rule revoke <id>
  audit [agent] | events [--agent A] [--kind PREFIX] [--limit N]
  search <query...> [--project ID]        full-text search over indexed transcripts

  team list [--project ID]      teams and roles, with validation issues and rules (P2)
  team presets                  add the starter roles and presets that are missing
  team import <file> | team export <team>
  team launch <project> <team> <slug> [--title T] [--brief TEXT] [--base B]
  team runs | team run <run>    runs; one run with agents, tasks, docs, timeline
  team pause|resume|stop <run>
  team tasks [--project ID] [--run RUN]
  team task add <project> <title...> [--node N] [--run RUN] [--after ID]... [--body TEXT]
  team task set <id> [--status S] [--node N] [--note TEXT]
  team msg <run> <node|agent> <text...>   message the team as the user (FR-409)
  team timeline <run>

The home defaults to $ORCH_HOME or ~/Library/Application Support/Orchestrator.";

fn main() {
    if let Err(e) = run() {
        eprintln!("orchd: {e:#}");
        std::process::exit(1);
    }
}

struct Args(Vec<String>);

impl Args {
    fn flag(&mut self, name: &str) -> bool {
        if let Some(i) = self.0.iter().position(|a| a == name) {
            self.0.remove(i);
            true
        } else {
            false
        }
    }

    fn opt(&mut self, name: &str) -> Result<Option<String>> {
        match self.0.iter().position(|a| a == name) {
            Some(i) if i + 1 < self.0.len() => {
                let v = self.0.remove(i + 1);
                self.0.remove(i);
                Ok(Some(v))
            }
            Some(_) => bail!("{name} needs a value"),
            None => Ok(None),
        }
    }

    fn opts(&mut self, name: &str) -> Result<Vec<String>> {
        let mut v = vec![];
        while let Some(x) = self.opt(name)? {
            v.push(x);
        }
        Ok(v)
    }

    fn next(&mut self, what: &str) -> Result<String> {
        if self.0.is_empty() {
            bail!("missing {what}\n\n{USAGE}");
        }
        Ok(self.0.remove(0))
    }

    /// The next argument as a number.
    fn num<T: std::str::FromStr>(&mut self, what: &str) -> Result<T> {
        let v = self.next(what)?;
        v.parse().map_err(|_| anyhow!("{what}: {v:?} is not a number"))
    }

    /// The next argument, which must be `yes` (true) or `no` (false).
    fn choice(&mut self, yes: &str, no: &str) -> Result<bool> {
        match self.next(&format!("{yes}|{no}"))? {
            v if v == yes => Ok(true),
            v if v == no => Ok(false),
            other => bail!("{yes} or {no}, not {other:?}"),
        }
    }

    fn done(&self) -> Result<()> {
        if let Some(a) = self.0.first() {
            bail!("unexpected argument {a:?}");
        }
        Ok(())
    }
}

/// `2026-10-27T09:00:00Z` (or a date, at midnight UTC) as ms since the epoch.
fn iso_ms(s: &str) -> Result<u64> {
    let t: jiff::Timestamp = match s.parse() {
        Ok(t) => t,
        Err(_) => format!("{s}T00:00:00Z").parse().with_context(|| format!("not an ISO time: {s}"))?,
    };
    Ok(t.as_millisecond().max(0) as u64)
}

fn run() -> Result<()> {
    let mut a = Args(std::env::args().skip(1).collect());
    let home = a
        .opt("--home")?
        .or_else(|| std::env::var("ORCH_HOME").ok())
        .map(PathBuf::from)
        .unwrap_or_else(AppPaths::default_root);
    let paths = AppPaths::new(home);
    if a.0.is_empty() || a.flag("--help") || a.flag("-h") {
        println!("{USAGE}");
        return Ok(());
    }
    let cmd = a.next("command")?;
    let sock = paths.socket();
    let print = |r: orch_protocol::CtlReply| -> Result<()> {
        println!("{}", serde_json::to_string_pretty(&r.data)?);
        if r.ok { Ok(()) } else { Err(anyhow!("request failed")) }
    };
    match cmd.as_str() {
        "serve" => serve(paths, a),
        "install-helpers" => {
            a.done()?;
            let exe_dir = std::env::current_exe()?.parent().map(PathBuf::from).context("no exe dir")?;
            let h = agent_runtime::preflight::install_helpers(&exe_dir, &paths.bin_dir())?;
            println!("installed {}, {} and {}", h.hook.display(), h.status.display(), h.mcp.display());
            Ok(())
        }
        "reconcile" => {
            a.done()?;
            if bus::is_listening(&sock) {
                bail!("the core is running: the reconciler runs at its start");
            }
            let rt = Runtime::new(RuntimeConfig::new(paths))?;
            println!("{}", serde_json::to_string_pretty(&rt.recover(false)?)?);
            Ok(())
        }
        "worktree" => {
            let repo = a.next("repo")?;
            let base = a.opt("--base")?;
            let kind = match a.next("kind")?.as_str() {
                "solo" => WorktreeKindSpec::Solo { slug: a.next("slug")? },
                "integration" => WorktreeKindSpec::Integration { feature: a.next("feature")? },
                "team" => WorktreeKindSpec::Team {
                    feature: a.next("feature")?,
                    role: a.next("role")?,
                    n: a.next("n")?.parse()?,
                },
                k => bail!("unknown worktree kind {k}"),
            };
            a.done()?;
            print(request(&sock, CtlRequest::CreateWorktree { repo: abs(&repo)?, kind, base })?)
        }
        "start" => {
            let agent_id = a.next("agent")?;
            let spec = AgentSpec {
                agent_id,
                repo: abs(&a.opt("--repo")?.context("--repo is required")?)?,
                slug: a.opt("--slug")?,
                base: a.opt("--base")?,
                model: a.opt("--model")?,
                effort: a.opt("--effort")?,
                permission_mode: a.opt("--mode")?,
                prompt: a.opt("--prompt")?,
                strict_mcp: Some(a.flag("--strict-mcp")),
                network_allow: a.opts("--allow-host")?,
                allow_rules: a.opts("--allow")?,
                allocation: a.opt("--allocation")?,
                tier: a.opt("--tier")?,
                first_message: a.opt("--message")?,
            };
            a.done()?;
            print(request(&sock, CtlRequest::Start { spec: Box::new(spec) })?)
        }
        "send" => {
            let agent_id = a.next("agent")?;
            if a.0.is_empty() {
                bail!("missing text");
            }
            let text = a.0.join(" ");
            print(request(&sock, CtlRequest::Deliver { agent_id, text })?)
        }
        "resume" => {
            let agent_id = a.next("agent")?;
            let message = a.opt("--message")?;
            a.done()?;
            print(request(&sock, CtlRequest::Resume { agent_id, message })?)
        }
        "interrupt" | "suspend" | "kill" => {
            let agent_id = a.next("agent")?;
            a.done()?;
            let req = match cmd.as_str() {
                "interrupt" => CtlRequest::Interrupt { agent_id },
                "suspend" => CtlRequest::Suspend { agent_id },
                _ => CtlRequest::Kill { agent_id },
            };
            print(request(&sock, req)?)
        }
        "state" | "usage" | "governor" => {
            a.done()?;
            let req = match cmd.as_str() {
                "state" => CtlRequest::State,
                "usage" => CtlRequest::Usage,
                _ => CtlRequest::Governor,
            };
            print(request(&sock, req)?)
        }
        "now" => {
            let json = a.flag("--json");
            let run = a.opt("--run")?;
            a.done()?;
            let r = request(&sock, CtlRequest::Now)?;
            if !r.ok {
                return print(r);
            }
            let snap = agent_runtime::now::only_run(r.data, run.as_deref());
            if json {
                println!("{}", serde_json::to_string_pretty(&snap)?);
            } else {
                print!("{}", agent_runtime::now::render(&snap, orch_protocol::now_ms()));
            }
            Ok(())
        }
        "pause-all" | "resume-all" => {
            a.done()?;
            print(request(&sock, if cmd == "pause-all" { CtlRequest::PauseAll } else { CtlRequest::ResumeAll })?)
        }
        "pause" | "unpause" => {
            let agent_id = a.next("agent")?;
            a.done()?;
            let req =
                if cmd == "pause" { CtlRequest::PauseAgent { agent_id } } else { CtlRequest::ResumeAgent { agent_id } };
            print(request(&sock, req)?)
        }
        "run-now" | "cancel" | "undo" => {
            let id = a.num("id")?;
            a.done()?;
            let req = match cmd.as_str() {
                "run-now" => CtlRequest::RunNow { entry: id },
                "cancel" => CtlRequest::Cancel { entry: id },
                _ => CtlRequest::Undo { action: id },
            };
            print(request(&sock, req)?)
        }
        "sprint" => {
            let pp = if a.0.is_empty() { None } else { Some(a.num("pp")?) };
            a.done()?;
            print(request(&sock, CtlRequest::Sprint { pp })?)
        }
        "answer-ask" => {
            let action = a.num("action")?;
            let accept = a.choice("apply", "skip")?;
            a.done()?;
            print(request(&sock, CtlRequest::AnswerAsk { action, accept })?)
        }
        "enforce" => {
            let on = a.choice("on", "off")?;
            a.done()?;
            print(request(&sock, CtlRequest::Enforce { on })?)
        }
        "stop" => {
            a.done()?;
            print(request(&sock, CtlRequest::Shutdown)?)
        }
        "attach" => {
            let agent_id = a.next("agent")?;
            a.done()?;
            attach(&sock, &agent_id)
        }
        "projects" => {
            a.done()?;
            print(request(&sock, CtlRequest::Projects)?)
        }
        "project" => {
            let req = match a.next("project subcommand")?.as_str() {
                "add" => CtlRequest::ProjectAdd { path: abs(&a.next("path")?)? },
                "clone" => {
                    let url = a.next("url")?;
                    let parent = abs(&a.next("parent folder")?)?;
                    CtlRequest::ProjectClone { url, parent, name: a.opt("--name")? }
                }
                "show" => {
                    let with_prs = a.flag("--prs");
                    CtlRequest::Project { project_id: a.next("project id")?, with_prs }
                }
                "fetch" => CtlRequest::Fetch { project_id: a.next("project id")? },
                "relocate" => {
                    let project_id = a.next("project id")?;
                    CtlRequest::RelocateProject { project_id, new_path: abs(&a.next("new path")?)? }
                }
                "remove-plan" => CtlRequest::RemovalPlan { project_id: a.next("project id")? },
                "remove" => {
                    let history = a.opt("--history")?.context("--history delete|archive is required")?;
                    let discard = a.opts("--discard")?.iter().map(|p| abs(p)).collect::<Result<Vec<_>>>()?;
                    let keep_merged = a.flag("--keep-merged");
                    CtlRequest::RemoveProject { project_id: a.next("project id")?, discard, keep_merged, history }
                }
                other => bail!("unknown project subcommand {other:?}\n\n{USAGE}"),
            };
            a.done()?;
            print(request(&sock, req)?)
        }
        "pr" => {
            match a.next("pr subcommand")?.as_str() {
                "refresh" => {}
                other => bail!("unknown pr subcommand {other:?}\n\n{USAGE}"),
            }
            let feature = a.next("feature")?;
            a.done()?;
            print(request(&sock, CtlRequest::PrRefresh { feature })?)
        }
        "diff" => {
            let against = a.opt("--against")?;
            let worktree = abs(&a.next("worktree")?)?;
            a.done()?;
            print(request(&sock, CtlRequest::Diff { worktree, against })?)
        }
        "worktree-rm" => {
            let delete_branch = a.flag("--delete-branch");
            let confirm_discard = a.flag("--discard");
            let project_id = a.next("project id")?;
            let path = abs(&a.next("worktree path")?)?;
            a.done()?;
            print(request(&sock, CtlRequest::RemoveWorktree { project_id, path, delete_branch, confirm_discard })?)
        }
        "commit" => {
            let paths = a.opts("--path")?;
            let agent_id = a.next("agent")?;
            if a.0.is_empty() {
                bail!("missing message");
            }
            let message = a.0.join(" ");
            print(request(
                &sock,
                CtlRequest::Commit { agent_id, message, paths: Some(paths).filter(|p| !p.is_empty()) },
            )?)
        }
        "inbox" => {
            a.done()?;
            print(request(&sock, CtlRequest::Inbox)?)
        }
        "decide" => {
            let scope = a.opt("--scope")?;
            let message = a.opt("--message")?;
            let item_id: u64 = a.next("item id")?.parse().context("item id")?;
            let decision = match a.next("decision")?.as_str() {
                "approve" | "allow" | "a" => InboxDecision::Approve,
                "reject" | "deny" | "r" => InboxDecision::Reject,
                "always" | "always-allow" => InboxDecision::AlwaysAllow,
                "reply" => InboxDecision::Reply,
                "dismiss" => InboxDecision::Dismiss,
                other => bail!("unknown decision {other:?}"),
            };
            a.done()?;
            print(request(&sock, CtlRequest::Decide { item_id, decision, scope, message })?)
        }
        "rules" => {
            a.done()?;
            print(request(&sock, CtlRequest::Rules)?)
        }
        "rule" => {
            let req = match a.next("rule subcommand")?.as_str() {
                "add" => CtlRequest::AddRule {
                    scope: a.next("scope")?,
                    scope_id: a.next("scope id")?,
                    pattern: a.next("pattern")?,
                    effect: a.next("effect")?,
                },
                "revoke" => CtlRequest::RevokeRule { id: a.next("rule id")?.parse().context("rule id")? },
                other => bail!("unknown rule subcommand {other:?}"),
            };
            a.done()?;
            print(request(&sock, req)?)
        }
        "audit" => {
            let agent_id = if a.0.is_empty() { None } else { Some(a.next("agent")?) };
            a.done()?;
            print(request(&sock, CtlRequest::Audit { agent_id, limit: None })?)
        }
        "report" => {
            let team_run_id = a.opt("--run")?;
            let since_ms = a.opt("--since")?.map(|t| iso_ms(&t)).transpose()?;
            let until_ms = a.opt("--until")?.map(|t| iso_ms(&t)).transpose()?;
            let (json, baseline) = (a.flag("--json"), a.flag("--baseline"));
            a.done()?;
            let r = request(&sock, CtlRequest::RunReport { team_run_id, since_ms, until_ms })?;
            if json || !r.ok {
                return print(r);
            }
            let report: orch_store::RunReport = serde_json::from_value(r.data)?;
            print!("{}", report.render(baseline));
            Ok(())
        }
        "events" => {
            let agent_id = a.opt("--agent")?;
            let kind = a.opt("--kind")?;
            let limit = a.opt("--limit")?.map(|l| l.parse()).transpose()?;
            a.done()?;
            print(request(&sock, CtlRequest::Events { agent_id, kind, since_ms: None, limit })?)
        }
        "search" => {
            let project_id = a.opt("--project")?;
            if a.0.is_empty() {
                bail!("missing query");
            }
            let query = a.0.join(" ");
            print(request(&sock, CtlRequest::Search { query, project_id, limit: None })?)
        }
        "team" => {
            let req = match a.next("team command")?.as_str() {
                "list" => CtlRequest::Teams { project_id: a.opt("--project")? },
                "presets" => CtlRequest::InstallPresets,
                "import" => {
                    let path = a.next("file")?;
                    CtlRequest::TeamImport { json: std::fs::read_to_string(&path).with_context(|| path.clone())? }
                }
                "export" => CtlRequest::TeamExport { team_id: a.next("team")?, project_id: a.opt("--project")? },
                "launch" => {
                    let (title, brief, base) = (a.opt("--title")?, a.opt("--brief")?, a.opt("--base")?);
                    CtlRequest::Launch {
                        project_id: a.next("project")?,
                        team_id: a.next("team")?,
                        slug: a.next("feature slug")?,
                        title,
                        brief,
                        base,
                    }
                }
                "runs" => CtlRequest::Runs,
                "run" => CtlRequest::Run { run_id: a.next("run")? },
                action @ ("pause" | "resume" | "stop") => {
                    CtlRequest::RunAct { run_id: a.next("run")?, action: action.to_string() }
                }
                "tasks" => CtlRequest::Tasks { project_id: a.opt("--project")?, run_id: a.opt("--run")? },
                "task" => match a.next("add|set")?.as_str() {
                    "add" => {
                        let (node, run_id, body) = (a.opt("--node")?, a.opt("--run")?, a.opt("--body")?);
                        let depends_on = a
                            .opts("--after")?
                            .iter()
                            .map(|s| s.parse::<u64>().context("--after takes a task id"))
                            .collect::<Result<Vec<_>>>()?;
                        let project_id = a.next("project")?;
                        if a.0.is_empty() {
                            bail!("missing title");
                        }
                        CtlRequest::TaskCreate {
                            project_id,
                            title: std::mem::take(&mut a.0).join(" "),
                            body,
                            node,
                            run_id,
                            depends_on,
                        }
                    }
                    "set" => {
                        let (status, node, note) = (a.opt("--status")?, a.opt("--node")?, a.opt("--note")?);
                        CtlRequest::TaskUpdate {
                            task_id: a.next("task id")?.parse().context("task id")?,
                            status,
                            node,
                            note,
                        }
                    }
                    other => bail!("team task {other:?}: add | set"),
                },
                "msg" => {
                    let (run_id, to) = (a.next("run")?, a.next("node or agent")?);
                    if a.0.is_empty() {
                        bail!("missing message");
                    }
                    CtlRequest::Message { run_id, to, body: std::mem::take(&mut a.0).join(" ") }
                }
                "timeline" => CtlRequest::Timeline { run_id: a.next("run")?, limit: None },
                other => bail!("unknown team command {other:?}\n\n{USAGE}"),
            };
            a.done()?;
            print(request(&sock, req)?)
        }
        other => bail!("unknown command {other:?}\n\n{USAGE}"),
    }
}

fn abs(p: &str) -> Result<String> {
    Ok(std::fs::canonicalize(p).with_context(|| format!("no such path {p}"))?.display().to_string())
}

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

fn serve(paths: AppPaths, mut a: Args) -> Result<()> {
    let mut cfg = RuntimeConfig::new(paths.clone());
    cfg.shadow = !a.flag("--enforce");
    let recover = !a.flag("--no-recover");
    if let Some(c) = a.opt("--cap")? {
        cfg.base_cap = c.parse()?;
    }
    if let Some(h) = a.opt("--hold-secs")? {
        cfg.hold = Duration::from_secs(h.parse()?);
    }
    cfg.otel_endpoint = a.opt("--otel")?;
    let no_otel = a.flag("--no-otel");
    if let Some(c) = a.opt("--claude")? {
        cfg.claude = PathBuf::from(c);
    }
    if let Some(h) = a.opt("--permission-hold-secs")? {
        cfg.permission_hold = Duration::from_secs(h.parse()?);
    }
    if let Some(m) = a.opt("--idle-min")? {
        let m: u64 = m.parse()?;
        cfg.idle_suspend = (m > 0).then(|| Duration::from_secs(m * 60));
    }
    if let Some(p) = a.opt("--projects-root")? {
        // Tests only: where transcripts are looked up instead of ~/.claude/projects.
        cfg.projects_root = PathBuf::from(p);
    }
    if let Some(m) = a.opt("--fetch-min")? {
        cfg.fetch.interval = Duration::from_secs(m.parse::<u64>()?.max(1) * 60);
    }
    a.done()?;
    // Refuse early if another core is up (before touching any agent).
    if bus::is_listening(&paths.socket()) {
        bail!("another core is listening on {}", paths.socket().display());
    }
    if agent_runtime::preflight::check_helpers(&cfg.helpers).is_err() {
        eprintln!(
            "orchd: helpers missing in {}: run `orchd install-helpers` (agents can't start without them)",
            paths.bin_dir().display()
        );
    }
    let external_otel = cfg.otel_endpoint.is_some();
    let rt = Runtime::new(cfg)?;
    for w in &rt.warnings {
        eprintln!("orchd: warning: {w}");
    }
    if !external_otel && !no_otel {
        match rt.start_otlp(0) {
            Ok(ep) => eprintln!("orchd: OTLP receiver on {ep} (local only)"),
            Err(e) => eprintln!("orchd: warning: OTLP receiver not started: {e:#}"),
        }
    }
    if recover {
        let r = rt.recover(true)?;
        eprintln!(
            "orchd: reconciled: {} spooled event(s), {} worktree(s), resumed {:?}",
            r.spool.iter().map(|s| s.events.len()).sum::<usize>(),
            r.worktrees.len(),
            r.to_resume()
        );
    }
    let listener = bus::bind_listener(&paths.socket())?;
    // SAFETY: installing a handler that only stores to an atomic.
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    let stop = Arc::new(AtomicBool::new(false));
    {
        let (stop, rt) = (stop.clone(), rt.clone());
        std::thread::spawn(move || {
            while !STOP.load(Ordering::SeqCst) && !stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(100));
            }
            if STOP.load(Ordering::SeqCst) {
                eprintln!("orchd: stopping: suspending agents");
                rt.shutdown();
                stop.store(true, Ordering::SeqCst);
            }
        });
    }
    eprintln!(
        "orchd: core {} listening on {} (governor {})",
        std::process::id(),
        paths.socket().display(),
        if rt.cfg.shadow { "in shadow mode" } else { "enforced" }
    );
    server::serve(rt, listener, stop)?;
    let _ = std::fs::remove_file(paths.socket());
    eprintln!("orchd: stopped");
    Ok(())
}

// ------------------------------------------------------------ attach

fn winsize() -> (u16, u16) {
    // SAFETY: TIOCGWINSZ fills a winsize struct we own.
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_row > 0 {
            (ws.ws_row, ws.ws_col)
        } else {
            (50, 160)
        }
    }
}

struct RawMode(Option<libc::termios>);

impl RawMode {
    fn enable() -> RawMode {
        // SAFETY: termios calls on our own stdin.
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut t) != 0 {
                return RawMode(None);
            }
            let saved = t;
            libc::cfmakeraw(&mut t);
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t);
            RawMode(Some(saved))
        }
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        if let Some(t) = self.0 {
            // SAFETY: restoring the saved attributes.
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t);
            }
        }
    }
}

const DETACH: u8 = 0x1d; // Ctrl-]

fn attach(sock: &std::path::Path, agent_id: &str) -> Result<()> {
    let (rows, cols) = winsize();
    let (s, leftover) = server::attach_client(sock, agent_id, rows, cols)?;
    eprint!("[attached to {agent_id}; detach with Ctrl-]]\r\n");
    let raw = RawMode::enable();
    let done = Arc::new(AtomicBool::new(false));
    // Core → terminal.
    let out_thread = {
        let done = done.clone();
        let mut sock_r = s.try_clone()?;
        std::thread::spawn(move || {
            let mut stdout = std::io::stdout();
            let _ = stdout.write_all(&leftover);
            let _ = stdout.flush();
            let mut buf = [0u8; 65536];
            loop {
                match sock_r.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if stdout.write_all(&buf[..n]).and_then(|_| stdout.flush()).is_err() {
                            break;
                        }
                    }
                }
            }
            done.store(true, Ordering::SeqCst);
        })
    };
    // Window size changes.
    {
        let (done, sock, id) = (done.clone(), sock.to_path_buf(), agent_id.to_string());
        std::thread::spawn(move || {
            let mut last = (rows, cols);
            while !done.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(300));
                let now = winsize();
                if now != last {
                    last = now;
                    let _ = request(&sock, CtlRequest::Resize { agent_id: id.clone(), rows: now.0, cols: now.1 });
                }
            }
        });
    }
    // Terminal → core. stdin is read on a thread; the main thread watches for the end.
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buf = [0u8; 4096];
        while let Ok(n) = stdin.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let mut w = s;
    while !done.load(Ordering::SeqCst) {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(bytes) => {
                if let Some(i) = bytes.iter().position(|b| *b == DETACH) {
                    let _ = w.write_all(&bytes[..i]);
                    break;
                }
                if w.write_all(&bytes).is_err() {
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => break,
        }
    }
    let _ = w.shutdown(std::net::Shutdown::Both);
    let _ = out_thread.join();
    drop(raw);
    eprint!("\r\n[detached from {agent_id}]\r\n");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_times_parse_as_iso() {
        assert_eq!(iso_ms("1970-01-01T00:00:01Z").unwrap(), 1_000);
        assert_eq!(iso_ms("1970-01-02").unwrap(), 86_400_000);
        assert!(iso_ms("yesterday").is_err());
    }
}
