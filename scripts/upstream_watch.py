#!/usr/bin/env python3
"""Upstream watcher for pi-rs.

Detects new releases of earendil-works/pi, and when the pinned version in
`UPSTREAM` drifts:
  1. files ONE issue, auto-split into a per-component checklist of changed
     upstream files (tools / system-prompt / agent-loop / session / cli /
     provider-catalog),
  2. opens a tracking PR that bumps `UPSTREAM` (merge = re-verification pass
     starts; see docs/upstream-changes.md).

Idempotent: skips if an open issue for the same target tag already exists.
Requires `gh` authenticated. Run locally or from the upstream-watch workflow.
"""
import json
import os
import subprocess
import sys
import urllib.request

REPO = os.environ.get("GITHUB_REPOSITORY", "123123213weqw/pi-rs")
UPSTREAM_REPO = "earendil-works/pi"
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
UPSTREAM_FILE = os.path.join(ROOT, "UPSTREAM")

# component -> upstream paths whose changes imply behavior drift in pi-rs
COMPONENTS = {
    "tools (read/bash/edit/write/grep/find/ls)": [
        "packages/coding-agent/src/core/tools/",
    ],
    "system-prompt & context": [
        "packages/coding-agent/src/core/system-prompt.ts",
        "packages/coding-agent/src/core/resource-loader.ts",
    ],
    "agent loop & tool execution": [
        "packages/agent/src/",
    ],
    "session format": [
        "packages/coding-agent/src/core/session-manager.ts",
        "packages/coding-agent/src/core/messages.ts",
    ],
    "CLI surface": [
        "packages/coding-agent/src/cli/args.ts",
        "packages/coding-agent/src/main.ts",
    ],
    "provider/model catalog & wire": [
        "packages/ai/src/",
    ],
}


def gh(*args):
    r = subprocess.run(["gh"] + list(args), capture_output=True, text=True)
    if r.returncode != 0:
        return None, r.stderr.strip()
    return r.stdout, None


def api(path):
    out, err = gh("api", path)
    if err:
        print(f"api {path} failed: {err}", file=sys.stderr)
        sys.exit(1)
    return json.loads(out)


def pinned_version():
    with open(UPSTREAM_FILE) as f:
        return f.read().strip()


def latest_tag():
    rel = api(f"repos/{UPSTREAM_REPO}/releases/latest")
    return rel.get("tag_name") or rel.get("name")


def open_issue_exists(tag):
    out, _ = gh("issue", "list", "--repo", REPO, "--state", "open",
                "--label", "upstream", "--search", tag, "--json", "number")
    if out:
        items = json.loads(out)
        return any(tag in json.dumps(i) for i in items)
    return False


def compare_files(base, head):
    data = api(f"repos/{UPSTREAM_REPO}/compare/{base}...{head}")
    files = data.get("files", [])
    return data.get("total_commits", 0), files


def bucket(path):
    for comp, prefixes in COMPONENTS.items():
        for p in prefixes:
            if path.startswith(p) or path == p:
                return comp
    return None


def build_issue_body(pinned, tag, commits, files):
    buckets = {}
    other = []
    for f in files:
        comp = bucket(f["filename"])
        if comp:
            buckets.setdefault(comp, []).append(f)
        else:
            other.append(f)

    lines = [
        f"Upstream **pi** moved: `{pinned}` → `{tag}` ({commits} commits, "
        f"{len(files)} files touched).",
        "",
        "pi-rs pins behavior to the tag in [`UPSTREAM`](../UPSTREAM). Re-verify",
        "each component below, in priority order, then merge the tracking PR.",
        "",
        "## Verification checklist",
        "",
    ]
    for comp in COMPONENTS:
        fs = buckets.get(comp)
        if not fs:
            continue
        lines.append(f"- [ ] **{comp}** — {len(fs)} file(s) changed")
        for f in sorted(fs, key=lambda x: -x.get("changes", 0))[:6]:
            lines.append(f"      `{f['filename']}` (+{f.get('additions', 0)}/-{f.get('deletions', 0)})")
        if len(fs) > 6:
            lines.append(f"      … and {len(fs) - 6} more")
        lines.append("      re-run: `bash tests/atom/run_atom.sh`, diff against pinned tag")
    if other:
        lines.append(f"- [ ] other upstream paths — {len(other)} file(s) (informational)")
    lines += [
        "",
        "## Protocol",
        "",
        "1. For each checked component: extract the new spec from upstream source,",
        "   port it, update the atom scenarios if behavior legitimately changed.",
        "2. Re-run the full gates: 85 atoms, tmux TUI E2E, arena spot tasks.",
        "3. Bump `UPSTREAM` (done in the tracking PR) and record in",
        "   `docs/upstream-changes.md`.",
        "",
        f"_Auto-filed by `scripts/upstream_watch.py` · compare: {UPSTREAM_REPO}@{pinned}...{tag}_",
    ]
    return "\n".join(lines)


def main():
    pinned = pinned_version()
    tag = latest_tag()
    if not tag:
        print("no releases upstream; nothing to do")
        return
    if tag == pinned:
        print(f"up to date: {pinned}")
        return
    if open_issue_exists(tag):
        print(f"issue for {tag} already open; skipping")
        return

    commits, files = compare_files(pinned, tag)
    body = build_issue_body(pinned, tag, commits, files)
    title = f"Upstream pi updated: {pinned} → {tag}"

    out, err = gh("issue", "create", "--repo", REPO, "--title", title,
                  "--body", body, "--label", "upstream")
    if err:
        # label may not exist yet; retry without
        out, err = gh("issue", "create", "--repo", REPO, "--title", title, "--body", body)
        if err:
            print(f"issue create failed: {err}", file=sys.stderr)
            sys.exit(1)
    issue_url = out.strip()
    print(f"filed: {issue_url}")
    issue_no = issue_url.rstrip("/").split("/")[-1]

    pr_url = open_tracking_pr(pinned, tag, issue_no)
    if pr_url:
        print(f"tracking PR: {pr_url}")


def open_tracking_pr(pinned, tag, issue_no):
    """Branch `upstream-sync/<tag>` bumping the UPSTREAM pin; PR closes the issue."""
    ref = api(f"repos/{REPO}/git/ref/heads/main")
    base_sha = ref["object"]["sha"]
    _, err = gh("api", "-X", "POST", f"repos/{REPO}/git/refs",
                "-f", f"ref=refs/heads/upstream-sync/{tag}",
                "-f", f"sha={base_sha}")
    if err and "already exists" not in err:
        print(f"branch create failed: {err}", file=sys.stderr)
        return None

    cur = api(f"repos/{REPO}/contents/UPSTREAM?ref=main")
    import base64
    content = base64.b64encode(f"{tag}\n".encode()).decode()
    _, err = gh("api", "-X", "PUT", f"repos/{REPO}/contents/UPSTREAM",
                "-f", "message=" + f"chore: track upstream pi {tag}",
                "-f", f"content={content}",
                "-f", f"branch=upstream-sync/{tag}",
                "-f", f"sha={cur['sha']}")
    if err:
        print(f"UPSTREAM bump failed: {err}", file=sys.stderr)
        return None

    body = (
        f"Closes #{issue_no}\n\n"
        f"Bumps the tracked upstream tag `{pinned}` → `{tag}`.\n\n"
        "Merge **only after** the issue checklist passes: behavior atoms (85/85), "
        "tmux TUI E2E, arena spot tasks. Record the outcome in "
        "`docs/upstream-changes.md`."
    )
    out, err = gh("pr", "create", "--repo", REPO,
                  "--head", f"upstream-sync/{tag}", "--base", "main",
                  "--title", f"chore: track upstream pi {tag}",
                  "--body", body)
    if err:
        print(f"pr create failed: {err}", file=sys.stderr)
        return None
    return out.strip()


if __name__ == "__main__":
    main()
