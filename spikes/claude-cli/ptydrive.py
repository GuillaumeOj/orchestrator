#!/usr/bin/env python3
"""PTY driver for the spike.
usage: ptydrive.py <script.json> <outdir> -- cmd args...
Script = list of steps:
  {"sleep": s}
  {"wait_screen": regex, "timeout": s}          # on ANSI-stripped output since last mark
  {"wait_file": path, "pattern": regex, "count": n, "timeout": s}
  {"paste": text}                               # bracketed paste, then Enter after 'enter_delay'
  {"type": text}                                # typed without bracketed paste, then Enter
  {"keys": "\\r"}                               # raw bytes (python escapes)
  {"mark": name}                                # reset screen search buffer
  {"rss": label}                                # record RSS (KB) of the process tree
  {"signal": "TERM"|"KILL"|"INT"}
  {"wait_exit": s}
Writes <outdir>/pty.raw, <outdir>/driver.jsonl."""
import json, os, pty, re, signal, subprocess, sys, threading, time, select, codecs, struct, fcntl, termios
script = json.load(open(sys.argv[1])); out = sys.argv[2]; cmd = sys.argv[sys.argv.index("--") + 1:]
os.makedirs(out, exist_ok=True)
log = open(os.path.join(out, "driver.jsonl"), "a")
def L(**k):
    k["ts"] = time.time(); log.write(json.dumps(k) + "\n"); log.flush(); print(json.dumps(k), flush=True)
pid, fd = pty.fork()
if pid == 0:
    os.execvp(cmd[0], cmd)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 50, 160, 0, 0))
raw = open(os.path.join(out, "pty.raw"), "ab")
buf = [""]; lock = threading.Lock(); alive = [True]
ANSI = re.compile(r"\x1b\[[0-9;?<>=]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(\x07|\x1b\\)|\x1b[()][0-9A-Za-z]|\x1b[=>78DEHMNOPZc]")
dec = codecs.getincrementaldecoder("utf-8")("replace")
def reader():
    while True:
        try:
            r, _, _ = select.select([fd], [], [], 0.2)
            if fd in r:
                d = os.read(fd, 65536)
                if not d: break
                raw.write(d); raw.flush()
                s = ANSI.sub(" ", dec.decode(d))
                # answer terminal queries (cursor position / device attrs) so TUIs don't hang
                if b"\x1b[6n" in d: os.write(fd, b"\x1b[1;1R")
                if b"\x1b[c" in d: os.write(fd, b"\x1b[?62;22c")
                with lock: buf[0] = (buf[0] + s)[-200000:]
        except OSError:
            break
    alive[0] = False
threading.Thread(target=reader, daemon=True).start()
def tree(root):
    ps = subprocess.run(["ps", "-axo", "pid=,ppid=,rss=,comm="], capture_output=True, text=True).stdout.split("\n")
    rows = [l.split(None, 3) for l in ps if l.strip()]
    kids = {}; info = {}
    for r in rows:
        if len(r) < 4: continue
        p, pp, rss, c = int(r[0]), int(r[1]), int(r[2]), r[3]
        kids.setdefault(pp, []).append(p); info[p] = (rss, c)
    res = []; st = [root]
    while st:
        p = st.pop()
        if p in info: res.append((p, info[p][0], info[p][1]))
        st += kids.get(p, [])
    return res
t0 = time.time(); ok = True
for i, st in enumerate(script):
    k = next(iter(st)); L(step=i, action=k, arg=st[k])
    if k == "sleep": time.sleep(st[k])
    elif k == "mark":
        with lock: buf[0] = ""
    elif k == "wait_screen":
        dl = time.time() + st.get("timeout", 60); hit = False
        while time.time() < dl and alive[0]:
            with lock: m = re.search(st[k], buf[0])
            if m: hit = True; break
            time.sleep(0.2)
        L(step=i, result="hit" if hit else "TIMEOUT", waited=round(time.time() - dl + st.get("timeout", 60), 2))
        if not hit and st.get("required", True): ok = False; break
    elif k == "wait_file":
        dl = time.time() + st.get("timeout", 60); hit = False
        while time.time() < dl:
            try: n = len(re.findall(st["pattern"], open(st[k]).read()))
            except FileNotFoundError: n = 0
            if n >= st.get("count", 1): hit = True; break
            time.sleep(0.2)
        L(step=i, result="hit" if hit else "TIMEOUT", waited=round(time.time() - dl + st.get("timeout", 60), 2))
        if not hit and st.get("required", True): ok = False; break
    elif k == "paste":
        os.write(fd, b"\x1b[200~" + st[k].encode() + b"\x1b[201~"); time.sleep(st.get("enter_delay", 0.3)); os.write(fd, b"\r")
    elif k == "type":   # no bracketed paste: chunks of raw chars, newlines sent as ESC+CR (meta-enter) are NOT used
        data = st[k].encode()
        for j in range(0, len(data), 64):
            os.write(fd, data[j:j+64]); time.sleep(0.01)
        time.sleep(st.get("enter_delay", 0.3)); os.write(fd, b"\r")
    elif k == "keys": os.write(fd, st[k].encode().decode("unicode_escape").encode())
    elif k == "rss":
        t = tree(pid)
        fp = subprocess.run(["footprint", "-p", str(pid)], capture_output=True, text=True).stdout
        m = re.search(r"Footprint: ([0-9.]+) (KB|MB|GB)", fp)
        fmb = (float(m.group(1)) * {"KB": 1/1024, "MB": 1, "GB": 1024}[m.group(2)]) if m else None
        L(step=i, rss_label=st[k], total_kb=sum(r[1] for r in t), claude_footprint_mb=fmb, procs=t)
    elif k == "signal":
        os.kill(pid, getattr(signal, "SIG" + st[k]))
    elif k == "wait_exit":
        dl = time.time() + st[k]
        while time.time() < dl:
            r = os.waitpid(pid, os.WNOHANG)
            if r[0]: L(step=i, exited=r[1]); break
            time.sleep(0.2)
        else: L(step=i, exited="TIMEOUT")
with lock: tail = buf[0][-3000:]
L(done=True, ok=ok, elapsed=round(time.time() - t0, 1), screen_tail=re.sub(r"\s+", " ", tail)[-1500:])
try: os.kill(pid, signal.SIGKILL)
except Exception: pass
