#!/usr/bin/env python3
"""Behavior-atom differ: pi vs pi-rs.

Compares, per scenario:
  atom/system-prompt   byte-exact (doc paths normalized to <DOCS>)
  atom/tools           structural JSON equality (key order normalized)
  atom/wire-messages   per-turn structural equality (tool_call_id kept)
  atom/stdout          byte-exact
  atom/exit-code       exact
  atom/session         entry skeleton equality (ids/timestamps volatile;
                       usage ints exact; cost dropped — catalogs differ)
  atom/filesystem      relpath + sha256 of final workdir state

Usage: diff_atoms.py <recdir-pi> <recdir-pirs> <meta.json>
meta.json: {"stdout_pi":..., "stdout_pirs":..., "exit_pi":..., "exit_pirs":...,
            "sess_pi":..., "sess_pirs":..., "fs_pi":..., "fs_pirs":...}
Prints one PASS/FAIL line per atom + final summary; exit 1 if any FAIL.
"""
import hashlib
import json
import re
import sys


def norm_json(x):
    if isinstance(x, dict):
        return {k: norm_json(x[k]) for k in sorted(x)}
    if isinstance(x, list):
        return [norm_json(v) for v in x]
    return x


def load_request(recdir, turn):
    import os
    p = os.path.join(recdir, f"turn-{turn:02d}.request.json")
    with open(p) as f:
        return json.load(f)


def norm_volatile(s):
    # session filenames carry run-unique timestamps/uuids; mask them
    s = re.sub(r"\.sess/[^\s:\]\"']+\.jsonl", "<SESSFILE>", s)
    # per-arm workdir absolute paths differ by name only
    s = re.sub(r"/tmp/atom-[^/]+/w-(pirs|pi)", "<WORK>", s)
    return s


def norm_prompt(s):
    # doc paths are installation-specific; normalize the three doc lines
    s = re.sub(r"- Main documentation: \S+", "- Main documentation: <DOCS>/README.md", s)
    s = re.sub(r"- Additional docs: \S+", "- Additional docs: <DOCS>/docs", s)
    s = re.sub(r"- Examples: \S+", "- Examples: <DOCS>/examples", s)
    return s


def norm_session(path):
    out = []
    with open(path) as f:
        for line in f:
            v = json.loads(line)
            t = v.get("type")
            if t == "session":
                out.append({"type": "session", "version": v.get("version")})
                continue
            if t != "message":
                continue
            m = v.get("message", {})
            role = m.get("role")
            entry = {"role": role}
            if role == "assistant":
                entry["content"] = norm_json(m.get("content"))
                entry["stopReason"] = m.get("stopReason")
                u = dict(m.get("usage") or {})
                u.pop("cost", None)
                entry["usage"] = u
                entry["api"] = m.get("api")
                entry["provider"] = m.get("provider")
            elif role == "toolResult":
                entry["toolName"] = m.get("toolName")
                entry["isError"] = m.get("isError")
                entry["content"] = norm_json(m.get("content"))
            elif role == "user":
                entry["content"] = norm_json(m.get("content"))
            out.append(entry)
    return out


def fs_state(root):
    import os
    state = {}
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in (".sess", ".arena", "__pycache__")]
        for fn in filenames:
            p = os.path.join(dirpath, fn)
            rel = os.path.relpath(p, root)
            if rel.endswith(".pyc"):
                continue
            with open(p, "rb") as f:
                state[rel] = hashlib.sha256(f.read()).hexdigest()[:16]
    return state


def main():
    rec_pi, rec_pirs, meta_path = sys.argv[1], sys.argv[2], sys.argv[3]
    meta = json.load(open(meta_path))
    results = []

    def check(name, ok, detail=""):
        results.append((name, ok, detail))

    # turn count must match
    import os
    n_pi = len([f for f in os.listdir(rec_pi) if f.endswith(".request.json")])
    n_pirs = len([f for f in os.listdir(rec_pirs) if f.endswith(".request.json")])
    check("turn-count", n_pi == n_pirs, f"pi={n_pi} pirs={n_pirs}")
    n = min(n_pi, n_pirs)

    for t in range(n):
        req_pi = load_request(rec_pi, t)
        req_pirs = load_request(rec_pirs, t)

        sys_roles = ("system", "developer")
        sp_pi = "".join(
            m.get("content", "") for m in req_pi.get("messages", []) if m.get("role") in sys_roles
        )
        sp_pirs = "".join(
            m.get("content", "") for m in req_pirs.get("messages", []) if m.get("role") in sys_roles
        )
        check(f"t{t}/system-prompt", norm_volatile(norm_prompt(sp_pi)) == norm_volatile(norm_prompt(sp_pirs)),
              _first_diff(norm_volatile(norm_prompt(sp_pi)), norm_volatile(norm_prompt(sp_pirs))))

        tools_pi = norm_json(req_pi.get("tools"))
        tools_pirs = norm_json(req_pirs.get("tools"))
        check(f"t{t}/tools", tools_pi == tools_pirs, _first_diff(
            json.dumps(tools_pi, ensure_ascii=False), json.dumps(tools_pirs, ensure_ascii=False)))

        msgs_pi = [m for m in req_pi.get("messages", []) if m.get("role") not in ("system", "developer")]
        msgs_pirs = [m for m in req_pirs.get("messages", []) if m.get("role") not in ("system", "developer")]
        # doc paths are installation-specific; normalize within message contents
        def norm_msgs(msgs):
            msgs = norm_json(msgs)
            # KNOWN DIVERGENCE: pi sends role "developer" (catalog flag), the
            # real DeepSeek API rejects it; pi-rs sends "system". Normalize.
            for m in msgs:
                if m.get("role") == "developer":
                    m["role"] = "system"
            for m in msgs:
                c = m.get("content")
                if isinstance(c, str):
                    m["content"] = norm_prompt(c)
                elif isinstance(c, list):
                    for blk in c:
                        if isinstance(blk, dict) and isinstance(blk.get("text"), str):
                            blk["text"] = norm_prompt(blk["text"])
            return norm_volatile(json.dumps(msgs, ensure_ascii=False))
        check(f"t{t}/wire-messages", norm_msgs(msgs_pi) == norm_msgs(msgs_pirs), _first_diff(
            norm_msgs(msgs_pi)[:400], norm_msgs(msgs_pirs)[:400]))

        extra_pi = {k: v for k, v in req_pi.items() if k not in ("messages", "tools")}
        extra_pirs = {k: v for k, v in req_pirs.items() if k not in ("messages", "tools")}
        check(f"t{t}/request-params", norm_json(extra_pi) == norm_json(extra_pirs), _first_diff(
            json.dumps(norm_json(extra_pi), ensure_ascii=False),
            json.dumps(norm_json(extra_pirs), ensure_ascii=False)))

    check("stdout", norm_volatile(meta["stdout_pi"]) == norm_volatile(meta["stdout_pirs"]),
          _first_diff(norm_volatile(meta["stdout_pi"]), norm_volatile(meta["stdout_pirs"])))
    check("exit-code", meta["exit_pi"] == meta["exit_pirs"],
          f"pi={meta['exit_pi']} pirs={meta['exit_pirs']}")
    def norm_sess_dump(p):
        return norm_volatile(json.dumps(norm_session(p), ensure_ascii=False, sort_keys=True))
    check("session", norm_sess_dump(meta["sess_pi"]) == norm_sess_dump(meta["sess_pirs"]),
          _first_diff(norm_sess_dump(meta["sess_pi"])[:500], norm_sess_dump(meta["sess_pirs"])[:500]))
    fs_pi, fs_pirs = fs_state(meta["fs_pi"]), fs_state(meta["fs_pirs"])
    check("filesystem", fs_pi == fs_pirs, _first_diff(
        json.dumps(fs_pi, sort_keys=True), json.dumps(fs_pirs, sort_keys=True)))

    fails = 0
    for name, ok, detail in results:
        mark = "PASS" if ok else "FAIL"
        if not ok:
            fails += 1
        print(f"{mark} {name}" + (f"  -- {detail[:300]}" if (detail and not ok) else ""))
    print(f"\n== {len(results) - fails}/{len(results)} atoms identical")
    sys.exit(1 if fails else 0)


def _first_diff(a, b):
    if a == b:
        return ""
    for i in range(min(len(a), len(b))):
        if a[i] != b[i]:
            ctx = max(0, i - 40)
            return f"@{i}: ...{a[ctx:i+60]!r} vs ...{b[ctx:i+60]!r}"
    return f"length {len(a)} vs {len(b)}"


if __name__ == "__main__":
    main()
