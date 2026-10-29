#!/usr/bin/env python3
"""Upstream watcher for pi-rs.

Detects upstream pi drift and files auto-split parity issues:

1. New release (tag != pinned `UPSTREAM`):
   files ONE release issue with a per-component checklist + a tracking PR that
   bumps `UPSTREAM`. Any open "main ahead" issue is closed as superseded.
2. Unreleased commit drift (release tag == pin, but upstream main moved):
   files/refreshes ONE "main ahead" issue — at most one outstanding, refreshed
   in place on every run so daily cron never spams.

Idempotent throughout. Requires `gh` authenticated.
"""
import json
import os
import sys

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
    import subprocess
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
                "--label", "upstream", "--json", "number,title")
    if out:
        items = json.loads(out)
        return any(tag in json.dumps(i) for i in items)
    return False


def open_unreleased_issue():
    """Return the number of an open 'main ahead' issue, if any (at most one)."""
    out, _ = gh("issue", "list", "--repo", REPO, "--state", "open",
                "--label", "upstream", "--json", "number,title")
    if out:
        for i in json.loads(out):
            if "main ahead" in i.get("title", ""):
                return i["number"]
    return None


def compare_files(base, head):
    data = api(f"repos/{UPSTREAM_REPO}/compare/{base}...{head}")
    return data.get("total_commits", 0), data.get("files", [])


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
        "2. Re-run the full gates: behavior atoms, tmux TUI E2E, arena spot tasks.",
        "3. Bump `UPSTREAM` (done in the tracking PR) and record in",
        "   `docs/upstream-changes.md`.",
        "",
        f"_Auto-filed by `scripts/upstream_watch.py` · compare: {UPSTREAM_REPO}@{pinned}...{tag}_",
    ]
    return "\n".join(lines)


def open_tracking_pr(pinned, tag, issue_no):
    """Branch `upstream-sync/<tag>` bumping the UPSTREAM pin; PR closes the issue."""
    import base64
    ref = api(f"repos/{REPO}/git/ref/heads/main")
    base_sha = ref["object"]["sha"]
    _, err = gh("api", "-X", "POST", f"repos/{REPO}/git/refs",
                "-f", f"ref=refs/heads/upstream-sync/{tag}",
                "-f", f"sha={base_sha}")
    if err and "already exists" not in err:
        print(f"branch create failed: {err}", file=sys.stderr)
        return None

    cur = api(f"repos/{REPO}/contents/UPSTREAM?ref=main")
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
        "Merge **only after** the issue checklist passes: behavior atoms, "
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


def check_commit_drift(pinned):
    """Release tag matches the pin, but upstream main may have moved. File (or
    refresh, in place) ONE issue summarizing unreleased drift — commit-level
    changes between the pinned tag and upstream HEAD."""
    data = api(f"repos/{UPSTREAM_REPO}/compare/{pinned}...HEAD")
    ahead = data.get("ahead_by", 0)
    head_sha = data.get("sha") or (
        (data.get("commits") or [{}])[-1].get("sha", ""))
    head_sha = str(head_sha)[:12]
    if ahead <= 0:
        print(f"up to date: {pinned} (main even)")
        return
    commits, files = ahead, data.get("files", [])
    body = build_issue_body(pinned, f"main@{head_sha} (unreleased)", commits, files)
    title = f"Upstream main ahead of {pinned}: +{ahead} unreleased commits"
    existing = open_unreleased_issue()
    if existing:
        gh("issue", "edit", str(existing), "--repo", REPO,
           "--title", title, "--body", body)
        print(f"refreshed unreleased-drift issue #{existing}")
    else:
        out, err = gh("issue", "create", "--repo", REPO, "--title", title,
                      "--body", body, "--label", "upstream")
        if err:
            out, err = gh("issue", "create", "--repo", REPO, "--title", title, "--body", body)
            if err:
                print(f"issue create failed: {err}", file=sys.stderr)
                sys.exit(1)
        print(f"filed: {out.strip()}")


def file_release_issue(pinned, tag):
    if open_issue_exists(tag):
        print(f"issue for {tag} already open; skipping")
        return None

    # a new release supersedes any open unreleased-drift issue
    stale = open_unreleased_issue()
    if stale:
        gh("issue", "close", str(stale), "--repo", REPO,
           "--comment", f"Superseded: upstream released {tag}; tracking continues in the release issue.")

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
    return issue_url.rstrip("/").split("/")[-1]


def ensure_label():
    out, _ = gh("label", "create", "--repo", REPO, "upstream",
                "--description", "drift detected in upstream pi",
                "--color", "5319E7")
    _ = out  # "already exists" is fine


def main():
    ensure_label()
    pinned = pinned_version()
    tag = latest_tag()
    if not tag:
        print("no releases upstream; nothing to do")
        return

    if tag == pinned:
        # releases are in sync — watch unreleased commit drift on main
        check_commit_drift(pinned)
        return

    issue_no = file_release_issue(pinned, tag)
    if issue_no is None:
        return
    pr_url = open_tracking_pr(pinned, tag, issue_no)
    if pr_url:
        print(f"tracking PR: {pr_url}")


if __name__ == "__main__":
    main()
