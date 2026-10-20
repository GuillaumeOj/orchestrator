#!/usr/bin/env python3
"""Throwaway stand-in for the Orchestrator core (P0 recovery spike). Not product code.

  core.py setup ROOT                 create a scratch repo ROOT/proj/demo (+ sibling ROOT/proj/demo.worktrees/)
  core.py serve ROOT [--recover]     run the core: Unix socket, SQLite state, agents in PTYs held by this process.
                                     --recover runs the reconciler first and applies its decisions.
  core.py reconcile ROOT             dry run: print what the reconciler sees and would do (JSON)
  core.py ctl ROOT '<json>'          send a control command to a running core

Layout: ROOT/app/{state.db, core.sock, core.log, policy.json, agents/<id>/...}, ROOT/proj/demo, ROOT/proj/demo.worktrees/.
"""
import codecs, fcntl, glob, json, os, pty, re, select, signal, socket, sqlite3, struct, subprocess, sys, termios
import threading, time, uuid

HERE = os.path.dirname(os.path.abspath(__file__))
HOME = os.path.expanduser("~")
CLAUDE = os.path.join(HOME, ".local/bin/claude")
LOGIN_PATH = f"{HOME}/.local/bin:{HOME}/.nvm/versions/node/v24.15.0/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
PREAMBLE = ("Text that starts with ORCH: in hook context or hook errors comes from the orchestrator app, "
            "on behalf of the user. Follow it.")
HOOK_EVENTS = ["SessionStart", "UserPromptSubmit", "PreToolUse", "PermissionRequest", "PostToolUse",
               "PostToolUseFailure", "PostToolBatch", "Notification", "Stop", "StopFailure", "SubagentStart",
               "SubagentStop", "PreCompact", "PostCompact", "SessionEnd"]
ANSI = re.compile(r"\x1b\[[0-9;?<>=]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(\x07|\x1b\\)|\x1b[()][0-9A-Za-z]|\x1b[=>78DEHMNOPZc]")
TURN_RE = re.compile(r"\[t:([0-9a-f]{8})\]")


def paths(root):
    app = os.path.join(root, "app")
    return dict(app=app, db=os.path.join(app, "state.db"), sock="/tmp/orchspike-%s.sock" % __import__("hashlib").sha1(app.encode()).hexdigest()[:8],
                log=os.path.join(app, "core.log"), policy=os.path.join(app, "policy.json"),
                agents=os.path.join(app, "agents"), repo=os.path.join(root, os.environ.get("ORCH_PROJ", "proj") + "/demo"),
                wtroot=os.path.join(root, os.environ.get("ORCH_PROJ", "proj") + "/demo.worktrees"))


def sh(*a, cwd=None, check=False):
    r = subprocess.run(list(a), cwd=cwd, capture_output=True, text=True)
    if check and r.returncode:
        raise RuntimeError(f"{a}: {r.stderr}")
    return r


# ---------------------------------------------------------------- DB
SCHEMA = """
create table if not exists worktrees(path text primary key, agent_id text, branch text, state text, created_at real);
create table if not exists agents(id text primary key, slug text, worktree text, session_id text, pid int, pgid int,
  state text, spawns int default 0, created_at real, last_event text, last_event_at real);
create table if not exists turns(id text primary key, agent_id text, body text, status text, created_at real,
  delivered_at real, done_at real);
create table if not exists holds(id text primary key, agent_id text, tool_use_id text, kind text, status text,
  command text, created_at real, decided_at real);
create table if not exists inbox(id integer primary key, agent_id text, kind text, body text, status text, created_at real);
create table if not exists events(id integer primary key, ts real, agent_id text, kind text, payload text);
"""


class DB:
    def __init__(self, path):
        self.c = sqlite3.connect(path, check_same_thread=False, isolation_level=None)
        self.c.execute("pragma journal_mode=wal")
        self.c.execute("pragma synchronous=normal")
        self.c.executescript(SCHEMA)
        self.lock = threading.Lock()

    def x(self, q, *a):
        with self.lock:
            return self.c.execute(q, a).fetchall()

    def rows(self, q, *a):
        with self.lock:
            cur = self.c.execute(q, a)
            cols = [d[0] for d in cur.description]
            return [dict(zip(cols, r)) for r in cur.fetchall()]


# ---------------------------------------------------------------- setup
def setup(root):
    P = paths(root)
    os.makedirs(P["agents"], exist_ok=True)
    os.makedirs(P["wtroot"], exist_ok=True)
    repo = P["repo"]
    if not os.path.isdir(os.path.join(repo, ".git")):
        os.makedirs(repo, exist_ok=True)
        env = dict(os.environ, GIT_CONFIG_GLOBAL=os.devnull)
        for cmd in (["git", "init", "-q", "-b", "main"],
                    ["git", "config", "user.name", "Spike"], ["git", "config", "user.email", "spike@example.invalid"]):
            subprocess.run(cmd, cwd=repo, check=True, env=env)
        open(os.path.join(repo, "README.md"), "w").write("# demo\n")
        open(os.path.join(repo, "notes.txt"), "w").write("line 1\nline 2\nline 3\n")
        subprocess.run(["git", "add", "."], cwd=repo, check=True, env=env)
        subprocess.run(["git", "commit", "-qm", "init"], cwd=repo, check=True, env=env)
    if not os.path.exists(P["policy"]):
        json.dump({"pre": [], "perm": "allow"}, open(P["policy"], "w"))
    print(json.dumps(P, indent=1))


# ---------------------------------------------------------------- process inspection
def procargs(pid):
    """argv + env via KERN_PROCARGS2. macOS 27 returns no env for Apple platform binaries (zsh, bash, sleep)."""
    import ctypes
    libc = ctypes.CDLL(None)
    mib = (ctypes.c_int * 3)(1, 49, pid)
    size = ctypes.c_size_t(0)
    if libc.sysctl(mib, 3, None, ctypes.byref(size), None, 0) != 0:
        return None, []
    buf = ctypes.create_string_buffer(size.value)
    if libc.sysctl(mib, 3, buf, ctypes.byref(size), None, 0) != 0:
        return None, []
    raw = buf.raw[:size.value]
    argc = struct.unpack("i", raw[:4])[0]
    parts = [p for p in raw[4:].split(b"\0") if p]
    rest = [p.decode(errors="replace") for p in parts[1:]]
    return rest[:argc], [e for e in rest[argc:] if "=" in e and not e.startswith(("ptr_munge", "main_stack", "executable_", "dyld_", "th_port", "security_config"))]


def ps_all():
    out = sh("ps", "-axo", "pid=,ppid=,pgid=,stat=,tty=,command=").stdout
    res = []
    for l in out.splitlines():
        f = l.split(None, 5)
        if len(f) == 6:
            res.append(dict(pid=int(f[0]), ppid=int(f[1]), pgid=int(f[2]), stat=f[3], tty=f[4], cmd=f[5]))
    return res


def cwd_map():
    out = sh("lsof", "-nP", "-d", "cwd", "-Fpn").stdout
    m, pid = {}, None
    for l in out.splitlines():
        if l.startswith("p"):
            pid = int(l[1:])
        elif l.startswith("n") and pid:
            m[pid] = l[1:]
    return m


def footprint_mb(pid):
    fp = sh("footprint", "-p", str(pid)).stdout
    m = re.search(r"Footprint: ([0-9.]+) (KB|MB|GB)", fp)
    return round(float(m.group(1)) * {"KB": 1 / 1024, "MB": 1, "GB": 1024}[m.group(2)], 1) if m else None


def kill_tree(pgids, pids, grace=5.0):
    """SIGTERM process groups + stray pids, SIGKILL whatever is left after `grace` s."""
    log = []
    for sig in (signal.SIGTERM, signal.SIGKILL):
        for g in pgids:
            try:
                os.killpg(g, sig); log.append(f"killpg({g},{sig.name})")
            except ProcessLookupError:
                pass
        for p in pids:
            try:
                os.kill(p, sig); log.append(f"kill({p},{sig.name})")
            except ProcessLookupError:
                pass
        if sig == signal.SIGTERM:
            end = time.time() + grace
            while time.time() < end:
                alive = [p for p in pids if _alive(p)] + [g for g in pgids if _alive_pg(g)]
                if not alive:
                    return log
                time.sleep(0.2)
    return log


def _alive(pid):
    try:
        os.kill(pid, 0); return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def _alive_pg(g):
    try:
        os.killpg(g, 0); return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


# ---------------------------------------------------------------- transcript
def transcript_path(session_id):
    hits = glob.glob(os.path.join(HOME, ".claude/projects/*", f"{session_id}.jsonl"))
    return hits[0] if hits else None


def transcript_summary(session_id):
    tp = transcript_path(session_id)
    if not tp:
        return {"found": False}
    entries = []
    for l in open(tp, errors="replace"):
        try:
            entries.append(json.loads(l))
        except Exception:
            pass
    uses, results, markers, last_type, last_ts = {}, set(), [], None, None
    last_stop_like = None
    for e in entries:
        t = e.get("type")
        if t in ("user", "assistant"):
            last_type, last_ts = t, e.get("timestamp")
            content = (e.get("message") or {}).get("content")
            if isinstance(content, str):
                markers += TURN_RE.findall(content)
            elif isinstance(content, list):
                for c in content:
                    if c.get("type") == "tool_use":
                        uses[c["id"]] = (c.get("name"), json.dumps(c.get("input"))[:120])
                    elif c.get("type") == "tool_result":
                        results.add(c.get("tool_use_id"))
                        txt = c.get("content")
                        markers += TURN_RE.findall(json.dumps(txt)) if txt else []
                    elif c.get("type") == "text" and t == "user":
                        markers += TURN_RE.findall(c.get("text", ""))
            if t == "assistant":
                last_stop_like = (e.get("message") or {}).get("stop_reason")
        elif t == "queue-operation":
            markers += TURN_RE.findall(json.dumps(e))
    pending = {k: v for k, v in uses.items() if k not in results}
    return {"found": True, "path": tp.replace(HOME, "~"), "entries": len(entries), "last_type": last_type,
            "last_ts": last_ts, "last_stop_reason": last_stop_like, "pending_tool_uses": pending,
            "turn_markers": sorted(set(markers))}


# ---------------------------------------------------------------- git
def git_worktrees(repo):
    out = sh("git", "-C", repo, "worktree", "list", "--porcelain").stdout
    res, cur = [], None
    for l in out.splitlines():
        if l.startswith("worktree "):
            cur = {"path": l[9:]}; res.append(cur)
        elif cur is not None and l:
            k, _, v = l.partition(" ")
            cur[k] = v if v else True
    return res


def worktree_state(path):
    st = {"exists": os.path.isdir(path)}
    if not st["exists"]:
        return st
    gd = sh("git", "-C", path, "rev-parse", "--absolute-git-dir")
    if gd.returncode:
        st["git_error"] = gd.stderr.strip()[:200]
        return st
    gd = gd.stdout.strip()
    st["gitdir"] = gd
    for name in ("MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD", "REBASE_HEAD", "rebase-merge", "rebase-apply",
                 "index.lock", "locked"):
        if os.path.exists(os.path.join(gd, name)):
            st.setdefault("markers", []).append(name)
    st["unmerged"] = len(set(l.split("\t")[-1] for l in sh("git", "-C", path, "ls-files", "-u").stdout.splitlines()))
    st["dirty"] = len(sh("git", "-C", path, "status", "--porcelain").stdout.splitlines())
    return st


# ---------------------------------------------------------------- reconciler
def reconcile(root, db, apply=False, log=print):
    """Compare DB with git, processes and transcripts; decide resume / suspend / flag per agent."""
    P = paths(root)
    report = {"ts": time.time(), "worktrees": [], "agents": [], "turns": [], "holds": [], "actions": []}
    act = report["actions"].append
    spooled_state = {}
    # 0. spool files written by orch-hook while the core was down
    for sp in glob.glob(os.path.join(P["agents"], "*", "spool.jsonl")):
        lines = open(sp).read().splitlines()
        evs = [json.loads(l) for l in lines]
        aid = os.path.basename(os.path.dirname(sp))
        for e in evs:   # replay state transitions the core missed while it was down (SessionEnd: no change)
            st = {"Stop": "idle", "UserPromptSubmit": "working", "PostToolUse": "working"}.get(e["event"])
            if st:
                spooled_state[aid] = st
        report.setdefault("spool", []).append({"agent": os.path.basename(os.path.dirname(sp)), "events": [
            (e["event"], e["why"]) for e in evs]})
        if apply:
            for e in evs:
                db.x("insert into events(ts,agent_id,kind,payload) values(?,?,?,?)", e["ts"], e["agent"],
                     "spooled:" + e["event"], json.dumps(e["payload"]))
            os.rename(sp, sp + f".replayed.{int(time.time())}")

    # 1. git worktrees vs DB
    gw = {w["path"]: w for w in git_worktrees(P["repo"])}
    dbw = {w["path"]: w for w in db.rows("select * from worktrees")}
    for path in sorted(set(gw) | set(dbw)):
        if path == os.path.realpath(P["repo"]) or path == P["repo"]:
            continue
        g, d = gw.get(path), dbw.get(path)
        item = {"path": path, "db": d and d["state"], "git": bool(g), "locked": g and g.get("locked"),
                "prunable": g and g.get("prunable"), "state": worktree_state(path)}
        if d and not g:
            item["verdict"] = "missing from git (moved or removed outside the app)"
        elif g and not d:
            item["verdict"] = "unknown to DB (crash between `git worktree add` and the DB write?)"
        elif d and d["state"] == "creating":
            item["verdict"] = "finish creation"
            if apply:
                db.x("update worktrees set state='ready' where path=?", path)
        elif g.get("prunable"):
            item["verdict"] = "prunable: folder gone"
        else:
            item["verdict"] = "ok"
        st = item["state"]
        if "index.lock" in st.get("markers", []):
            cw = cwd_map()
            holders = [p for p in ps_all() if "git" in p["cmd"].split(" ")[0] and cw.get(p["pid"], "").startswith(path)]
            item["index_lock"] = "stale" if not holders else "held"
            act({"worktree": path, "do": "remove stale index.lock"})
            if apply:
                os.remove(os.path.join(st["gitdir"], "index.lock"))
        report["worktrees"].append(item)

    # 2. processes
    procs = ps_all()
    cwds = cwd_map()
    me = os.getpid()
    for a in db.rows("select * from agents"):
        if a["id"] in spooled_state and a["state"] not in ("archived", "exited"):
            a["state"] = spooled_state[a["id"]]
            if apply:
                db.x("update agents set state=? where id=?", a["state"], a["id"])
        adir = os.path.join(P["agents"], a["id"])
        found = []
        for p in procs:
            why = []
            if a["pgid"] and p["pgid"] == a["pgid"]:
                why.append("pgid")
            if adir in p["cmd"]:
                why.append("argv")
            cwd = cwds.get(p["pid"], "")
            if a["worktree"] and (cwd == a["worktree"] or cwd.startswith(a["worktree"] + "/")):
                why.append("cwd")
            if why and p["pid"] != me:
                _, env = procargs(p["pid"])
                if f"ORCH_AGENT_ID={a['id']}" in env:
                    why.append("env")
                found.append(dict(pid=p["pid"], ppid=p["ppid"], pgid=p["pgid"], tty=p["tty"], stat=p["stat"],
                                  cmd=p["cmd"][:100], how=why))
        tr = transcript_summary(a["session_id"]) if a["session_id"] else {"found": False}
        wt = worktree_state(a["worktree"]) if a["worktree"] else {}
        ag = {"id": a["id"], "db_state": a["state"], "last_event": a["last_event"], "procs": found,
              "transcript": tr, "worktree": wt}
        # decision
        if found:
            act({"agent": a["id"], "do": "kill orphans", "pids": [p["pid"] for p in found]})
            if apply:
                ag["kill_log"] = kill_tree(sorted({p["pgid"] for p in found if p["pgid"] > 1}),
                                           [p["pid"] for p in found])
        held = db.rows("select * from holds where agent_id=? and status='held'", a["id"])
        last_stop = db.x("select payload from events where agent_id=? and kind in ('Stop','spooled:Stop') "
                         "order by ts desc limit 1", a["id"])
        bg_lost = [b for b in (json.loads(last_stop[0][0]).get("background_tasks") or []) if b.get("status") == "running"] \
            if last_stop and a["state"] == "idle" else []
        merge = [m for m in wt.get("markers", []) if m in ("MERGE_HEAD", "REBASE_HEAD", "rebase-merge",
                                                            "rebase-apply", "CHERRY_PICK_HEAD", "REVERT_HEAD")]
        if a["state"] in ("exited", "archived"):
            decision = "none"
        elif merge:
            decision = "flag"
            ag["reason"] = f"{'/'.join(merge)} in worktree, {wt.get('unmerged')} unmerged path(s)"
            if apply:
                db.x("insert into inbox(agent_id,kind,body,status,created_at) values(?,?,?,?,?)", a["id"], "alert",
                     "Merge in progress after a restart: " + ag["reason"], "pending", time.time())
        elif any(h["kind"] == "gate" for h in held):
            decision = "suspend (gate stays in Inbox)"
        elif a["state"] in ("working", "queued") or tr.get("pending_tool_uses"):
            decision = "resume"
        elif bg_lost:
            decision = "resume"
            ag["reason"] = f"{len(bg_lost)} background task(s) were running at the last Stop: " + "; ".join(
                b.get("command", "")[:60] for b in bg_lost)
        else:
            decision = "suspend"
        ag["decision"] = decision
        act({"agent": a["id"], "do": decision})
        if apply:
            new_state = {"resume": "resuming", "flag": "error"}.get(decision, "suspended")
            if decision == "none":
                new_state = a["state"]
            db.x("update agents set state=?, pid=null, pgid=null where id=?", new_state, a["id"])
        report["agents"].append(ag)

        # 3. holds: a hold that was open when the core died was answered by orch-hook's fallback (deny)
        for h in held:
            nh = {"gate": "pending (re-ask on resume)", "gov": "requeued"}.get(h["kind"], "interrupted")
            report["holds"].append({"id": h["id"], "kind": h["kind"], "was": "held", "now": nh})
            if apply:
                db.x("update holds set status=? where id=?", "interrupted", h["id"])
                if h["kind"] == "gate":
                    db.x("insert into inbox(agent_id,kind,body,status,created_at) values(?,?,?,?,?)", a["id"],
                         "gate", f"Gate still pending after restart: {h['command']}", "pending", time.time())

        # 4. turns: delivered or not? the transcript is the source of truth (marker [t:<id>])
        seen = set(tr.get("turn_markers", []))
        for t in db.rows("select * from turns where agent_id=? and status in ('delivering','delivered','queued')",
                         a["id"]):
            if t["id"] in seen:
                new = "delivered" if t["status"] != "queued" else "delivered (DB lagged)"
            else:
                new = "queued (redeliver)" if t["status"] != "queued" else "queued"
            report["turns"].append({"id": t["id"], "agent": a["id"], "db": t["status"], "in_transcript": t["id"] in seen,
                                    "now": new})
            if apply:
                db.x("update turns set status=? where id=?", "delivered" if t["id"] in seen else "queued", t["id"])

    return report


# ---------------------------------------------------------------- core server
class Agent:
    def __init__(self, aid):
        self.id, self.fd, self.pid, self.buf, self.ready_at, self.alive = aid, None, None, "", 0, False
        self.last_input = 0


class Core:
    def __init__(self, root):
        self.root, self.P = root, paths(root)
        os.makedirs(self.P["agents"], exist_ok=True)
        self.db = DB(self.P["db"])
        self.agents = {}
        self.logf = open(self.P["log"], "a")
        self.stop = False

    def log(self, **k):
        k["ts"] = round(time.time(), 3)
        self.logf.write(json.dumps(k) + "\n"); self.logf.flush()

    def policy(self):
        try:
            return json.load(open(self.P["policy"]))
        except Exception:
            return {}

    # ---- worktree + spawn
    def ensure_worktree(self, aid, slug, base="main"):
        path = os.path.join(self.P["wtroot"], "solo", slug)
        branch = f"orch/solo/{slug}"
        if self.db.x("select 1 from worktrees where path=? and state='ready'", path):
            return path
        self.db.x("insert or replace into worktrees values(?,?,?,?,?)", path, aid, branch, "creating", time.time())
        if os.environ.get("ORCH_CRASH_AT") == "worktree-add":
            sh("git", "-C", self.P["repo"], "worktree", "add", "-q", "-b", branch, path, base, check=True)
            os.kill(os.getpid(), signal.SIGKILL)
        if not os.path.isdir(path):
            sh("git", "-C", self.P["repo"], "worktree", "add", "-q", "-b", branch, path, base, check=True)
        sh("git", "-C", self.P["repo"], "worktree", "lock", "--reason", f"orch agent {aid}", path)
        self.db.x("update worktrees set state='ready' where path=?", path)
        return path

    def write_agent_files(self, aid, wt, hook_cmd=None):
        adir = os.path.join(self.P["agents"], aid)
        os.makedirs(adir, exist_ok=True)
        hook = {"type": "command", "command": hook_cmd or os.path.join(HERE, "orch_hook.py"), "timeout": 600}
        settings = {
            "hooks": {e: [{"matcher": "*", "hooks": [hook]}] for e in HOOK_EVENTS},
            "statusLine": {"type": "command", "command": os.path.join(HERE, "orch_status.py"), "refreshInterval": 5},
            "permissions": {"allow": ["Bash", "Read", "Edit", "Write", "Task", "Agent"]},
        }
        json.dump(settings, open(os.path.join(adir, "settings.json"), "w"), indent=1)
        open(os.path.join(adir, "gitconfig"), "w").write(
            "[user]\n\tname = Spike Agent\n\temail = agent@example.invalid\n[credential]\n\thelper =\n")
        return adir

    def spawn(self, aid, slug=None, resume=False, extra_args=(), extra_env=None, cwd=None, hook_cmd=None):
        row = self.db.rows("select * from agents where id=?", aid)
        slug = slug or (row[0]["slug"] if row else aid)
        wt = cwd or (row[0]["worktree"] if row and row[0]["worktree"] else self.ensure_worktree(aid, slug))
        adir = self.write_agent_files(aid, wt, hook_cmd)
        sid = row[0]["session_id"] if row and row[0]["session_id"] else str(uuid.uuid4())
        if not row:
            self.db.x("insert into agents(id,slug,worktree,session_id,state,created_at) values(?,?,?,?,?,?)",
                      aid, slug, wt, sid, "spawning", time.time())
        args = [CLAUDE, "--settings", os.path.join(adir, "settings.json"), "--model", "haiku",
                "--permission-mode", "manual", "--strict-mcp-config", "--append-system-prompt", PREAMBLE,
                "-n", slug] + (["--resume", sid] if resume else ["--session-id", sid]) + list(extra_args)
        env = {"HOME": HOME, "USER": os.environ["USER"], "LOGNAME": os.environ["USER"], "SHELL": "/bin/zsh",
               "LANG": "en_US.UTF-8", "TERM": "xterm-256color", "PATH": LOGIN_PATH, "ORCH_AGENT_ID": aid,
               "ORCH_SOCK": self.P["sock"], "ORCH_SPOOL": os.path.join(adir, "spool.jsonl"), "ORCH_AGENT_DIR": adir,
               "DISABLE_AUTOUPDATER": "1", "GIT_CONFIG_GLOBAL": os.path.join(adir, "gitconfig"),
               "GIT_TERMINAL_PROMPT": "0"}
        env.update(extra_env or {})
        pid, fd = pty.fork()          # child: setsid() + controlling tty => own session and process group
        if pid == 0:
            os.chdir(wt)
            os.execve(CLAUDE, args, env)
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 50, 160, 0, 0))
        ag = Agent(aid); ag.fd, ag.pid, ag.alive = fd, pid, True
        self.agents[aid] = ag
        self.db.x("update agents set pid=?, pgid=?, state=?, spawns=spawns+1 where id=?", pid, os.getpgid(pid),
                  "starting", aid)
        self.log(spawn=aid, pid=pid, resume=resume, args=args[1:])
        threading.Thread(target=self.reader, args=(ag, os.path.join(adir, "pty.raw")), daemon=True).start()
        return pid

    def reader(self, ag, rawpath):
        raw = open(rawpath, "ab")
        dec = codecs.getincrementaldecoder("utf-8")("replace")
        while True:
            try:
                r, _, _ = select.select([ag.fd], [], [], 0.5)
                if ag.fd in r:
                    d = os.read(ag.fd, 65536)
                    if not d:
                        break
                    raw.write(d); raw.flush()
                    if b"\x1b[6n" in d: os.write(ag.fd, b"\x1b[1;1R")
                    if b"\x1b[c" in d: os.write(ag.fd, b"\x1b[?62;22c")
                    ag.buf = (ag.buf + ANSI.sub(" ", dec.decode(d)))[-100000:]
            except OSError:
                break
        ag.alive = False
        try:
            _, status = os.waitpid(ag.pid, 0)
        except ChildProcessError:
            status = None
        self.log(exited=ag.id, status=status)
        self.db.x("update agents set state=case when state in ('suspending') then 'suspended' else 'exited' end, "
                  "pid=null where id=?", ag.id)

    # ---- hook handling
    def on_hook(self, aid, ev, p):
        now = time.time()
        self.db.x("insert into events(ts,agent_id,kind,payload) values(?,?,?,?)", now, aid, ev, json.dumps(p))
        self.db.x("update agents set last_event=?, last_event_at=? where id=?", ev, now, aid)
        ag = self.agents.get(aid)
        setst = lambda s: self.db.x("update agents set state=? where id=?", s, aid)
        if ev == "SessionStart":
            setst("idle")
            if ag: ag.ready_at = now + 1.5
        elif ev == "UserPromptSubmit":
            setst("working")
            for t in TURN_RE.findall(p.get("prompt", "")):
                self.db.x("update turns set status='delivered' where id=?", t)
        elif ev in ("PostToolUse", "PostToolUseFailure"):   # Subagent* can fire after Stop: not a state change
            setst("working")
        elif ev == "Stop":
            setst("idle")
            self.db.x("update turns set status='done', done_at=? where agent_id=? and status='delivered'", now, aid)
            if ag: ag.ready_at = now + 0.5
        elif ev == "StopFailure":
            setst("error")
        elif ev == "SessionEnd":
            setst("exited")
        elif ev == "PermissionRequest":
            beh = self.policy().get("perm", "allow")
            return {"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": {"behavior": beh}}}
        elif ev == "PreToolUse":
            setst("working")
            cmd = json.dumps(p.get("tool_input", {}))
            for rule in self.policy().get("pre", []):
                if rule.get("match", "\0") in cmd:
                    return self.pre_decision(aid, p, rule, cmd)
        return None

    def pre_decision(self, aid, p, rule, cmd):
        out = lambda d, r: {"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": d,
                                                   "permissionDecisionReason": r}}
        if rule["action"] in ("allow", "deny"):
            return out(rule["action"], rule.get("reason", "ORCH: policy"))
        # hold: 'gate' (waits for the user) or 'gov' (governor, waits a few seconds then denies)
        hid = uuid.uuid4().hex[:8]
        kind = rule["action"]
        self.db.x("insert into holds values(?,?,?,?,?,?,?,?)", hid, aid, p.get("tool_use_id"), kind, "held",
                  cmd[:300], time.time(), None)
        self.db.x("update agents set state=? where id=?", "waiting_gate" if kind == "gate" else "queued", aid)
        self.log(hold=hid, agent=aid, kind=kind, cmd=cmd[:120])
        end = time.time() + rule.get("hold_s", 5)
        while time.time() < end:
            st = self.db.x("select status from holds where id=?", hid)[0][0]
            if st != "held":
                break
            time.sleep(0.2)
        st = self.db.x("select status from holds where id=?", hid)[0][0]
        if st == "held":
            st = rule.get("on_timeout", "deny")
            self.db.x("update holds set status=?, decided_at=? where id=?", "timeout-" + st, time.time(), hid)
        self.db.x("update agents set state='working' where id=?", aid)
        reason = {"gate": "ORCH: awaiting user approval", "gov": "ORCH: paused by governor"}[kind]
        return out("allow" if st.endswith("allow") else "deny", reason if not st.endswith("allow") else "ORCH: approved")

    # ---- control
    def on_ctl(self, c):
        cmd = c["cmd"]
        if cmd == "spawn":
            return {"pid": self.spawn(c["agent"], c.get("slug"), c.get("resume", False), c.get("args", ()),
                                      c.get("env"), c.get("cwd"), c.get("hook_cmd"))}
        if cmd == "deliver":
            tid = uuid.uuid4().hex[:8]
            self.db.x("insert into turns values(?,?,?,?,?,?,?)", tid, c["agent"], c["body"], "queued", time.time(),
                      None, None)
            return {"turn": tid}
        if cmd == "type":
            ag = self.agents[c["agent"]]
            os.write(ag.fd, c["text"].encode().decode("unicode_escape").encode() if c.get("escapes") else c["text"].encode())
            ag.last_input = time.time()
            return {"ok": True}
        if cmd == "release":
            self.db.x("update holds set status=?, decided_at=? where id=?", c["decision"], time.time(), c["hold"])
            return {"ok": True}
        if cmd == "screen":
            return {"screen": re.sub(r"\s+", " ", self.agents[c["agent"]].buf[-c.get("n", 3000):])}
        if cmd == "clear":
            self.agents[c["agent"]].buf = ""; return {"ok": True}
        if cmd == "footprint":
            ag = self.agents[c["agent"]]
            return {"pid": ag.pid, "mb": footprint_mb(ag.pid)}
        if cmd == "suspend":
            ag = self.agents[c["agent"]]
            self.db.x("update agents set state='suspending' where id=?", ag.id)
            return {"log": kill_tree([os.getpgid(ag.pid)], [ag.pid])}
        if cmd == "state":
            return {t: self.db.rows(f"select * from {t}") for t in ("agents", "worktrees", "turns", "holds", "inbox")}
        if cmd == "pid":
            return {"pid": os.getpid()}
        return {"error": "unknown"}

    def conn(self, s):
        f = s.makefile("rb")
        try:
            m = json.loads(f.readline())
            if m["type"] == "hook":
                r = self.on_hook(m["agent"], m["event"], m["payload"])
                resp = {"stdout": json.dumps(r) if r else "", "exit": 0}
            elif m["type"] == "status":
                self.db.x("insert into events(ts,agent_id,kind,payload) values(?,?,?,?)", time.time(), m["agent"],
                          "status", m["raw"])
                resp = {}
            else:
                resp = self.on_ctl(m)
            s.sendall((json.dumps(resp) + "\n").encode())
        except Exception as e:
            self.log(conn_error=repr(e))
            try:
                s.sendall((json.dumps({"error": repr(e)}) + "\n").encode())
            except OSError:
                pass
        finally:
            s.close()

    def deliver_loop(self):
        while not self.stop:
            now = time.time()
            for aid, ag in list(self.agents.items()):
                if not ag.alive or not ag.ready_at or now < ag.ready_at or now - ag.last_input < 2:
                    continue
                st = self.db.x("select state from agents where id=?", aid)[0][0]
                if st != "idle":
                    continue
                q = self.db.rows("select * from turns where agent_id=? and status='queued' order by created_at limit 1", aid)
                if not q:
                    continue
                t = q[0]
                self.db.x("update turns set status='delivering', delivered_at=? where id=?", now, t["id"])
                self.db.x("update agents set state='working' where id=?", aid)
                os.write(ag.fd, b"\x1b[200~" + f"[t:{t['id']}] {t['body']}".encode() + b"\x1b[201~")
                time.sleep(0.3)
                os.write(ag.fd, b"\r")
                ag.ready_at = 0
                self.log(delivered=t["id"], agent=aid)
            time.sleep(0.2)

    def serve(self, recover=False):
        if recover:
            rep = reconcile(self.root, self.db, apply=True)
            json.dump(rep, open(os.path.join(self.P["app"], f"reconcile-{int(time.time())}.json"), "w"), indent=1)
            self.log(reconciled=rep["actions"])
            for a in rep["agents"]:
                if a["decision"] == "resume":
                    self.spawn(a["id"], resume=True)
                    self.db.x("insert into turns values(?,?,?,?,?,?,?)", uuid.uuid4().hex[:8], a["id"],
                              "ORCH: the orchestrator restarted while you were working. Check what your last step "
                              "actually did before redoing anything, then finish the task.", "queued", time.time(),
                              None, None)
        if os.path.exists(self.P["sock"]):
            os.unlink(self.P["sock"])          # stale socket left by a kill -9
        srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        srv.bind(self.P["sock"]); os.chmod(self.P["sock"], 0o600); srv.listen(64)
        open(os.path.join(self.P["app"], "core.pid"), "w").write(str(os.getpid()))
        self.log(serving=os.getpid(), recover=recover)
        threading.Thread(target=self.deliver_loop, daemon=True).start()
        signal.signal(signal.SIGTERM, lambda *a: sys.exit(0))
        while True:
            s, _ = srv.accept()
            threading.Thread(target=self.conn, args=(s,), daemon=True).start()


def ctl(root, msg):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(paths(root)["sock"])
    m = json.loads(msg); m["type"] = "ctl"
    s.sendall((json.dumps(m) + "\n").encode())
    print(s.makefile().readline().strip())


if __name__ == "__main__":
    cmd, root = sys.argv[1], os.path.abspath(sys.argv[2])
    if cmd == "setup":
        setup(root)
    elif cmd == "serve":
        Core(root).serve(recover="--recover" in sys.argv)
    elif cmd == "reconcile":
        print(json.dumps(reconcile(root, DB(paths(root)["db"]), apply=False), indent=1, default=str))
    elif cmd == "ctl":
        ctl(root, sys.argv[3])
