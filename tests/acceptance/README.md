# Isolated acceptance (server-side, Docker)

Runs the agent inside a minimal container (debian 12 + python3/pytest + rg/fd +
the pi-rs binary) so host environment quirks (global pytest plugins, AGENTS.md,
~/.pi) cannot leak into tasks or results.

Image: `tests/acceptance/Dockerfile.pirs-env` (build with `--network host` on
hosts whose docker bridge NAT is broken). Runner: `run_docker_v100.sh`:

- mounts ONLY the task directory (`/work`) and the pi-rs binary (read-only)
- injects just `DEEPSEEK_API_KEY`; HOME is fresh; no user configs
- resource-capped: `--memory=1g --cpus=2`
- differential: same tasks through real pi 1.4.0 (node + bundle mounted via a
  wrapper script) — identical rubric both arms

Note: uses `--network host` because this server's docker bridge NAT is broken;
drop it if your docker networking is healthy (network isolation then applies too).

V100 results (2026-10-28, clean containers, both arms 4/4 across bugfix /
feature / comprehension / continuity tasks):

| arm | A-bugfix | B-feature | RSS |
|---|---|---|---|
| pi-rs | PASS 5.9s | PASS 5.3s | **24 MB** |
| pi 1.4.0 | PASS 9.0s | PASS 10.8s | 149–155 MB |

Clean-env proof: artifacts contain zero host-pollution shims (the nonebug
workaround the Mac run needed does not appear here).
