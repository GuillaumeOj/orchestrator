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

/// Serves until `stop` is set (by a `shutdown` request or the caller).
pub fn serve(rt: Arc<Runtime>, listener: UnixListener, stop: Arc<AtomicBool>) -> std::io::Result<()> {
    listener.set_nonblocking(true)?;
    let loops = {
        let (rt, stop) = (rt.clone(), stop.clone());
        std::thread::Builder::new().name("orch-loops".into()).spawn(move || {
            let mut last_tick = Instant::now();
            while !stop.load(Ordering::SeqCst) {
                rt.pump();
                if last_tick.elapsed() >= TICK_EVERY {
                    rt.tick();
                    last_tick = Instant::now();
                }
                std::thread::sleep(PUMP_EVERY);
            }
        })?
    };
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((s, _)) => {
                let _ = s.set_nonblocking(false);
                let (rt, stop) = (rt.clone(), stop.clone());
                std::thread::spawn(move || handle(&rt, s, &stop));
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(50)),
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
    let env: Envelope = match bus::read_line(&mut BufReader::new(&s)) {
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
        Msg::Ctl(CtlRequest::Attach { agent_id, rows, cols }) => {
            if !rt.live_ids().contains(&agent_id) {
                return reply(&s, Reply::Ctl(CtlReply::err(format!("agent {agent_id} is not running"))));
            }
            reply(&s, Reply::Ctl(CtlReply::ok(json!("attached"))));
            if let Ok(clone) = s.try_clone() {
                let _ = rt.attach(&agent_id, clone, rows, cols);
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
        CtlRequest::Interrupt { agent_id } => ok_or(rt.interrupt(&agent_id).map(|_| json!("esc sent"))),
        CtlRequest::Deliver { agent_id, text } => ok_or(rt.deliver(&agent_id, &text).map(|t| json!({"turn": t}))),
        CtlRequest::Resize { agent_id, rows, cols } => ok_or(rt.resize(&agent_id, rows, cols).map(|_| json!("ok"))),
        CtlRequest::Read { agent_id } => ok_or(rt.read_output(&agent_id).map(|b| json!(String::from_utf8_lossy(&b)))),
        CtlRequest::PauseAll => CtlReply::ok(json!({"action": rt.pause_all()})),
        CtlRequest::ResumeAll => CtlReply::ok(json!({"released": rt.resume_all()})),
        CtlRequest::State => CtlReply::ok(rt.state_json()),
        CtlRequest::Usage => CtlReply::ok(rt.usage_json()),
        CtlRequest::Governor => CtlReply::ok(rt.governor_json()),
        CtlRequest::Attach { .. } | CtlRequest::Shutdown => CtlReply::err("handled by the server"),
    }
}

/// Client side: one request, one reply.
pub fn request(socket: &std::path::Path, req: CtlRequest) -> anyhow::Result<CtlReply> {
    let s = bus::connect(socket, Duration::from_secs(1))?;
    bus::write_line(&mut &s, &Envelope::new(Msg::Ctl(req)))?;
    let r: ReplyEnvelope = bus::read_line(&mut BufReader::new(&s))?;
    match r.reply {
        Reply::Ctl(c) => Ok(c),
        Reply::Error { message } => anyhow::bail!(message),
        other => anyhow::bail!("unexpected reply {other:?}"),
    }
}
