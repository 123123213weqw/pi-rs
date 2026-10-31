#!/usr/bin/env python3
"""Drive pi-rs TUI in a pty with scripted keystrokes; verify the aligned
interactive behaviors: footer, hint line, Tab completion, /commands,
Ctrl+P model cycling (+ session model_change), scroll keys, mouse wheel."""
import fcntl
import glob
import json
import os
import pty
import re
import select
import shutil
import struct
import subprocess
import sys
import termios
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
WORK = "/tmp/tui-e2e-work"
SESS = "/tmp/tui-e2e-sess"
PORT = 8791

shutil.rmtree(WORK, ignore_errors=True)
shutil.rmtree(SESS, ignore_errors=True)
os.makedirs(WORK)
os.makedirs(SESS)

with open("/tmp/tui-e2e-scenario.json", "w") as f:
    json.dump([{"text": "hi from mock", "usage": {"prompt_tokens": 100, "completion_tokens": 20,
                                                  "prompt_tokens_details": {"cached_tokens": 64}}}], f)

mock = subprocess.Popen([sys.executable, f"{ROOT}/tests/mock_server.py",
                         "/tmp/tui-e2e-scenario.json", str(PORT)])
time.sleep(0.5)

env = os.environ.copy()
env["PI_BASE_URL"] = f"http://127.0.0.1:{PORT}"
env["TERM"] = "xterm-256color"
cmd = [f"{ROOT}/target/release/pi", "--provider", "deepseek", "--model", "deepseek-chat",
       "--session-dir", SESS, "--models", "deepseek-chat,deepseek-reasoner"]

master, slave = pty.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
proc = subprocess.Popen(cmd, stdin=slave, stdout=slave, stderr=slave, env=env, cwd=WORK, close_fds=True)
os.close(slave)

checks = []
total_raw = ""
last_raw = ""


def check(name, cond):
    checks.append((name, bool(cond)))
    print(("PASS " if cond else "FAIL ") + name)


def read_all(t=0.6):
    global total_raw, last_raw
    out = b""
    end = time.time() + t
    while time.time() < end:
        r, _, _ = select.select([master], [], [], 0.1)
        if r:
            try:
                data = os.read(master, 65536)
                if not data:
                    break
                out += data
            except OSError:
                break
    total_raw += out.decode("utf-8", "replace")
    last_raw = out.decode("utf-8", "replace")
    return last_raw


def norm(s):
    """Strip ANSI, collapse whitespace: wrap-safe substring matching."""
    s = re.sub(r"\x1b\[[0-9;<>?]*[a-zA-Z~]", "", s)
    s = re.sub(r"\x1b\][^\x07]*\x07", "", s)
    return re.sub(r"\s+", " ", s)


def send(s):
    os.write(master, s.encode())
    time.sleep(0.15)


def full_redraw():
    """Resize the pty to force ratatui into a full-frame redraw, then return
    the captured frame — differential rendering otherwise only emits the cells
    that changed, so state like the updated footer never appears verbatim."""
    global total_raw, last_raw
    fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 104, 0, 0))
    time.sleep(0.3)
    fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    return read_all(0.6)


try:
    read_all(1.2)
    N = norm(total_raw)
    check("footer: model label", "deepseek/deepseek-chat" in N)
    check("footer: cwd name", "tui-e2e-work" in N)
    check("hint line: ctrl+p", "ctrl+p" in N)

    # Tab completion: "/m" + Tab -> hint line lists /model /models; editor holds common prefix
    send("/m\t")
    read_all(0.4)
    N = norm(total_raw)
    check("tab: candidates shown", "/model" in N and "/models" in N)

    # editor is already "/model" (common prefix); run it
    send("\r")
    read_all(0.5)
    N = norm(total_raw)
    check("/model: shows cycle list", "ctrl+p cycles:" in N and "deepseek-reasoner" in N)

    # Ctrl+P cycles to deepseek-reasoner
    send("\x10")
    read_all(0.5)
    N = norm(total_raw)
    check("ctrl+p: system notice", "model → deepseek/deepseek-reasoner" in N)

    # Ctrl+P again wraps back to deepseek-chat
    send("\x10")
    read_all(0.5)
    N = norm(total_raw)
    check("ctrl+p wraps back to first", "model → deepseek/deepseek-chat" in N)

    # scroll: PageUp/PageDown + SGR mouse wheel (capture is on, crossterm parses SGR)
    send("\x1b[5~")
    send("\x1b[6~")
    send("\x1b[<64;10;10M")
    send("\x1b[<65;10;10M")
    read_all(0.6)
    send("/help\r")  # force a visible change so a redraw must occur
    read_all(0.5)
    N = norm(total_raw)
    check("scroll+wheel: still rendering", "ctrl+shift+p cycle model" in N)

    send("/nope\r")
    read_all(0.4)
    N = norm(total_raw)
    check("unknown command: error line", "unknown command /nope" in N)

    send("hello\r")
    read_all(3.0)
    N = norm(total_raw)
    check("assistant reply rendered", "hi from mock" in N)
    frame = norm(full_redraw())
    check("footer: token stats", "in 36 · out 20 · cached 64" in frame)

    send("/exit\r")
    proc.wait(timeout=8)
finally:
    if proc.poll() is None:
        proc.kill()
    mock.terminate()
    with open("/tmp/tui-e2e-screen.txt", "w") as f:
        f.write(total_raw)

sess_files = sorted(glob.glob(f"{SESS}/*.jsonl"))
data = open(sess_files[-1]).read() if sess_files else ""
entries = [json.loads(l) for l in data.splitlines() if l.strip()]
mc = [e for e in entries if e.get("type") == "model_change"]
check("session: model_change count == 3 (setup + fwd + wrap)", len(mc) == 3)
check("session: model_change target recorded", any(e.get("modelId") == "deepseek-reasoner" for e in mc))

print(f"\n{sum(1 for _, ok in checks if ok)}/{len(checks)}")
sys.exit(0 if all(ok for _, ok in checks) else 1)
