//! The bus server: accepts hook, statusline and control connections on the
//! core's Unix socket and drives the runtime's loops.

use std::io::BufReader;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use orch_protocol::bus;
use orch_protocol::{CtlReply, CtlRequest, Envelope, Msg, PROTOCOL_VERSION, Reply, ReplyEnvelope};
use serde_json::json;

use crate::runtime::Runtime;

pub const PUMP_EVERY: Duration = Duration::from_millis(200);
pub const TICK_EVERY: Duration = Duration::from_secs(30);
pub const HOUSEKEEPING_EVERY: Duration = Duration::from_secs(5);
pub const FETCH_EVERY: Duration = Duration::from_secs(60);

/// Serves until `stop` is set (by a `shutdown` request or the caller).
pub fn serve(rt: Arc<Runtime>, listener: UnixListener, stop: Arc<AtomicBool>) -> std::io::Result<()> {
    let loops = {
        let (rt, stop) = (rt.clone(), stop.clone());
        std::thread::Builder::new().name("orch-loops".into()).spawn(move || {
            let mut last_tick = Instant::now();
            // `Instant - Duration` can underflow shortly after boot: start "due" with None.
            let mut last_house: Option<Instant> = None;
            while !stop.load(Ordering::SeqCst) {
                rt.pump();
                if last_tick.elapsed() >= TICK_EVERY {
                    rt.tick();
                    last_tick = Instant::now();
                }
                if last_house.is_none_or(|t| t.elapsed() >= HOUSEKEEPING_EVERY) {
                    rt.housekeeping();
                    last_house = Some(Instant::now());
                }
                std::thread::sleep(PUMP_EVERY);
            }
        })?
    };
    // Network work (fetch) on its own thread: it can take a while.
    {
        let (rt, stop) = (rt.clone(), stop.clone());
        std::thread::Builder::new().name("orch-fetch".into()).spawn(move || {
            let mut last: Option<Instant> = None;
            while !stop.load(Ordering::SeqCst) {
                if last.is_none_or(|t| t.elapsed() >= FETCH_EVERY) {
                    rt.fetch_tick(&rt.cfg.fetch, rt.cfg.conditions.as_ref());
                    last = Some(Instant::now());
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        })?;
    }
    // poll() wakes at once for a connection and every 100 ms to check
    // `stop` (a wake-up connection would fail if the socket file was removed).
    use std::os::fd::AsRawFd;
    let fd = listener.as_raw_fd();
    while !stop.load(Ordering::SeqCst) {
        let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        // SAFETY: one pollfd we own, for the listener's open fd.
        let n = unsafe { libc::poll(&mut pfd, 1, 100) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if n == 0 {
            continue;
        }
        match listener.accept() {
            Ok((s, _)) => {
                let (rt, stop) = (rt.clone(), stop.clone());
                std::thread::spawn(move || handle(&rt, s, &stop));
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock) => {}
            Err(e) => return Err(e),
        }
    }
    let _ = loops.join();
    Ok(())
}

fn reply(s: &UnixStream, r: Reply) {
    let _ = bus::write_line(&mut &*s, &ReplyEnvelope::new(r));
}

fn handle(rt: &Runtime, s: UnixStream, stop: &AtomicBool) {
    let mut reader = BufReader::new(&s);
    let env: Envelope = match bus::read_line(&mut reader) {
        Ok(e) => e,
        Err(e) => return reply(&s, Reply::Error { message: e.to_string() }),
    };
    if env.v != PROTOCOL_VERSION {
        return reply(
            &s,
            Reply::Error { message: format!("protocol version {} (core speaks {PROTOCOL_VERSION})", env.v) },
        );
    }
    match env.msg {
        Msg::Hook(m) => {
            let output = rt.on_hook(&m, Some(&s));
            reply(&s, Reply::Hook { output });
        }
        Msg::Status(m) => {
            rt.on_status(&m);
            reply(&s, Reply::Ack);
        }
        Msg::Mcp(m) => {
            let (is_error, text, data) = rt.on_mcp(&m, Some(&s));
            reply(&s, Reply::Mcp { is_error, text, data });
        }
        Msg::Ctl(CtlRequest::Attach { agent_id, rows, cols }) => {
            if !rt.live_ids().contains(&agent_id) {
                return reply(&s, Reply::Ctl(CtlReply::err(format!("agent {agent_id} is not running"))));
            }
            reply(&s, Reply::Ctl(CtlReply::ok(json!("attached"))));
            // Input sent right after the request is already in the reader's buffer.
            let initial = reader.buffer().to_vec();
            if let Ok(clone) = s.try_clone() {
                let _ = rt.attach(&agent_id, clone, &initial, rows, cols);
            }
        }
        Msg::Ctl(CtlRequest::Shutdown) => {
            rt.shutdown();
            stop.store(true, Ordering::SeqCst);
            reply(&s, Reply::Ctl(CtlReply::ok(json!("stopped"))));
        }
        Msg::Ctl(req) => reply(&s, Reply::Ctl(ctl(rt, req))),
    }
}

fn ok_or(r: anyhow::Result<serde_json::Value>) -> CtlReply {
    match r {
        Ok(v) => CtlReply::ok(v),
        Err(e) => CtlReply::err(format!("{e:#}")),
    }
}

pub fn ctl(rt: &Runtime, req: CtlRequest) -> CtlReply {
    match req {
        CtlRequest::Ping => CtlReply::ok(json!({"pid": std::process::id(), "warnings": rt.warnings})),
        CtlRequest::CreateWorktree { repo, kind, base } => {
            ok_or(rt.create_worktree(&PathBuf::from(repo), &kind, base.as_deref()).map(|w| json!(w)))
        }
        CtlRequest::Start { spec } => ok_or(
            rt.start(&spec)
                .map(|a| json!({"id": a.id, "session_id": a.session_id, "worktree": a.worktree, "pid": a.pid})),
        ),
        CtlRequest::Resume { agent_id, message } => {
            ok_or(rt.resume(&agent_id, message.as_deref()).map(|_| json!("resumed")))
        }
        CtlRequest::Suspend { agent_id } => {
            ok_or(rt.suspend(&agent_id).map(|l| json!({"suspended": agent_id, "log": l})))
        }
        CtlRequest::Kill { agent_id } => ok_or(rt.kill(&agent_id).map(|l| json!({"killed": agent_id, "log": l}))),
        CtlRequest::DeletePlan { agent_id } => ok_or(rt.delete_plan(&agent_id)),
        CtlRequest::DeleteAgent { agent_id, remove_worktree, confirm_discard } => {
            ok_or(rt.delete_agent(&agent_id, remove_worktree, confirm_discard))
        }
        CtlRequest::DeleteFeature { feature_id, remove_worktrees, discard } => {
            ok_or(rt.delete_feature(&feature_id, remove_worktrees, discard))
        }
        CtlRequest::Interrupt { agent_id } => ok_or(rt.interrupt(&agent_id).map(|_| json!("esc sent"))),
        CtlRequest::Deliver { agent_id, text } => ok_or(rt.deliver(&agent_id, &text).map(|t| json!({"turn": t}))),
        CtlRequest::Resize { agent_id, rows, cols } => ok_or(rt.resize(&agent_id, rows, cols).map(|_| json!("ok"))),
        CtlRequest::Read { agent_id } => ok_or(rt.read_output(&agent_id).map(|b| json!(String::from_utf8_lossy(&b)))),
        CtlRequest::PauseAll => CtlReply::ok(json!({"action": rt.pause_all()})),
        CtlRequest::ResumeAll => CtlReply::ok(json!({"released": rt.resume_all()})),
        CtlRequest::PauseAgent { agent_id } => ok_or(rt.pause_agent(&agent_id).map(|a| json!({"action": a}))),
        CtlRequest::ResumeAgent { agent_id } => CtlReply::ok(json!({"released": rt.resume_agent(&agent_id)})),
        CtlRequest::RunNow { entry } => ok_or(rt.run_now(entry).map(|_| json!("released"))),
        CtlRequest::Cancel { entry } => ok_or(rt.cancel(entry).map(|_| json!("cancelled"))),
        CtlRequest::Undo { action } => ok_or(rt.undo(action).map(|_| json!("undone"))),
        CtlRequest::Sprint { pp } => ok_or(rt.sprint(pp).map(|a| json!({"action": a}))),
        CtlRequest::AnswerAsk { action, accept } => ok_or(rt.answer_ask(action, accept).map(|_| json!("answered"))),
        CtlRequest::Enforce { on } => ok_or(rt.set_shadow(!on).map(|_| json!({"enforced": on}))),
        CtlRequest::State => CtlReply::ok(rt.state_json()),
        CtlRequest::Usage => CtlReply::ok(rt.usage_json()),
        CtlRequest::Governor => CtlReply::ok(rt.governor_json()),
        CtlRequest::Inbox => CtlReply::ok(json!(rt.inbox())),
        CtlRequest::Decide { item_id, decision, scope, message } => {
            ok_or(rt.decide(item_id, decision, scope.as_deref(), message.as_deref()))
        }
        CtlRequest::Rules => ok_or(rt.db.rules().map(|r| json!(r))),
        CtlRequest::AddRule { scope, scope_id, pattern, effect } => ok_or((|| {
            let sc =
                orch_store::RuleScope::parse(&scope).ok_or_else(|| anyhow::anyhow!("scope: project | role | agent"))?;
            let ef =
                orch_store::RuleEffect::parse(&effect).ok_or_else(|| anyhow::anyhow!("effect: allow | deny | ask"))?;
            Ok(json!(rt.add_rule(sc, &scope_id, &pattern, ef, None)?))
        })()),
        CtlRequest::RevokeRule { id } => ok_or(rt.revoke_rule(id).map(|done| json!({"revoked": done}))),
        CtlRequest::Audit { agent_id, limit } => {
            ok_or(rt.db.audit_log(agent_id.as_deref(), limit.unwrap_or(100)).map(|v| json!(v)))
        }
        CtlRequest::RunReport { team_run_id, since_ms, until_ms } => ok_or(
            rt.db
                .run_report(&orch_store::ReportQuery { team_run_id, since_ms, until_ms }, orch_protocol::now_ms())
                .map(|r| json!(r)),
        ),
        CtlRequest::Events { agent_id, kind, since_ms, limit } => ok_or(
            rt.db
                .events(since_ms.unwrap_or(0), agent_id.as_deref(), kind.as_deref(), limit.unwrap_or(200))
                .map(|v| json!(v)),
        ),
        CtlRequest::ProjectAdd { path } => ok_or(rt.add_project(&PathBuf::from(path)).map(|p| json!(p))),
        CtlRequest::ProjectClone { url, parent, name } => {
            ok_or(rt.clone_project(&url, &PathBuf::from(parent), name.as_deref()).map(|p| json!(p)))
        }
        CtlRequest::Projects => ok_or(rt.projects_overview().map(|v| json!(v))),
        CtlRequest::Project { project_id, with_prs } => ok_or(rt.project_detail(&project_id, with_prs)),
        CtlRequest::Fetch { project_id } => {
            ok_or(rt.fetch_project(&project_id, &rt.cfg.fetch, rt.cfg.conditions.as_ref()).map(|o| json!(o)))
        }
        CtlRequest::Diff { worktree, against } => {
            ok_or(rt.diff(&PathBuf::from(worktree), against.as_deref()).map(|d| json!(d)))
        }
        CtlRequest::RemoveWorktree { project_id, path, delete_branch, confirm_discard } => ok_or(
            rt.remove_worktree(&project_id, &PathBuf::from(path), delete_branch, confirm_discard).map(|o| json!(o)),
        ),
        CtlRequest::RemovalPlan { project_id } => ok_or(rt.removal_plan(&project_id)),
        CtlRequest::RemoveProject { project_id, discard, keep_merged, history } => ok_or((|| {
            let history: orch_git::project::History = orch_store::untag(&history)
                .ok_or_else(|| anyhow::anyhow!("history: delete | archive (got {history:?})"))?;
            let choices = orch_git::project::RemovalChoices {
                remove_merged: !keep_merged,
                discard: discard.into_iter().map(PathBuf::from).collect(),
                history,
            };
            Ok(json!(rt.remove_project(&project_id, &choices)?))
        })()),
        CtlRequest::RelocateProject { project_id, new_path } => {
            ok_or(rt.relocate_project(&project_id, &PathBuf::from(new_path)).map(|r| json!(r)))
        }
        CtlRequest::Commit { agent_id, message, paths } => ok_or(
            rt.commit_for(&agent_id, &orch_git::commit::CommitRequest { message, paths }, "user").map(|c| json!(c)),
        ),
        CtlRequest::Search { query, project_id, limit } => {
            ok_or(rt.db.search_transcripts(&query, project_id.as_deref(), limit.unwrap_or(50)).map(|v| json!(v)))
        }
        CtlRequest::Teams { project_id } => ok_or(rt.teams_json(project_id.as_deref())),
        CtlRequest::InstallPresets => ok_or(rt.install_presets().map(|n| json!({"added": n}))),
        CtlRequest::TeamSave { team } => {
            ok_or(serde_json::from_value(team).map_err(Into::into).and_then(|t| rt.save_team(t)))
        }
        CtlRequest::RoleSave { role } => {
            ok_or(serde_json::from_value(role).map_err(Into::into).and_then(|r| rt.save_role(r)).map(|r| json!(r)))
        }
        CtlRequest::TeamImport { json: text } => ok_or(rt.import_team(&text).map(|t| json!(t))),
        CtlRequest::TeamExport { team_id, project_id } => {
            ok_or(rt.export_team(&team_id, project_id.as_deref()).map(|s| json!(s)))
        }
        CtlRequest::Launch { project_id, team_id, slug, title, brief, base } => ok_or(
            rt.launch(&crate::team::LaunchSpec {
                project_id,
                team_id,
                slug,
                title: title.unwrap_or_default(),
                brief: brief.unwrap_or_default(),
                base,
                off_hours: false,
            })
            .map(|r| json!(r)),
        ),
        CtlRequest::Runs => CtlReply::ok(rt.runs_json()),
        CtlRequest::Run { run_id } => ok_or(rt.run_json(&run_id)),
        CtlRequest::RunAct { run_id, action } => ok_or(rt.run_act(&run_id, &action)),
        CtlRequest::Tasks { project_id, run_id } => {
            ok_or(rt.db.tasks(project_id.as_deref(), None, run_id.as_deref()).map(|v| json!(v)))
        }
        CtlRequest::TaskCreate { project_id, title, body, node, run_id, depends_on } => ok_or(
            rt.user_create_task(
                &project_id,
                &title,
                body.as_deref().unwrap_or(""),
                node.as_deref(),
                run_id.as_deref(),
                depends_on,
            )
            .map(|t| json!(t)),
        ),
        CtlRequest::TaskUpdate { task_id, status, node, note } => {
            ok_or(rt.user_update_task(task_id, status.as_deref(), node.as_deref(), note.as_deref()).map(|t| json!(t)))
        }
        CtlRequest::Message { run_id, to, body } => {
            ok_or(rt.user_message(&run_id, &to, &body).map(|ids| json!({"ids": ids})))
        }
        CtlRequest::Timeline { run_id, limit } => {
            ok_or(rt.db.run_events(&run_id, limit.unwrap_or(200)).map(|v| json!(v)))
        }
        CtlRequest::Attach { .. } | CtlRequest::Shutdown => CtlReply::err("handled by the server"),
    }
}

/// Client side of `attach`: returns the raw stream and any PTY bytes that
/// arrived in the same read as the reply.
pub fn attach_client(
    socket: &std::path::Path,
    agent_id: &str,
    rows: u16,
    cols: u16,
) -> anyhow::Result<(UnixStream, Vec<u8>)> {
    let s = bus::connect(socket, Duration::from_secs(1))?;
    bus::write_line(&mut &s, &Envelope::new(Msg::Ctl(CtlRequest::Attach { agent_id: agent_id.into(), rows, cols })))?;
    let mut r = BufReader::new(s.try_clone()?);
    let reply: ReplyEnvelope = bus::read_line(&mut r)?;
    match reply.reply {
        Reply::Ctl(c) if c.ok => Ok((s, r.buffer().to_vec())),
        other => anyhow::bail!("attach refused: {other:?}"),
    }
}

/// Client side: one request, one reply.
pub fn request(socket: &std::path::Path, req: CtlRequest) -> anyhow::Result<CtlReply> {
    match bus::round_trip(socket, &Envelope::new(Msg::Ctl(req)), Duration::from_secs(1), None)? {
        Reply::Ctl(c) => Ok(c),
        Reply::Error { message } => anyhow::bail!(message),
        other => anyhow::bail!("unexpected reply {other:?}"),
    }
}
