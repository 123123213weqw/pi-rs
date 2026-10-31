# Upstream change log

pi-rs pins a specific upstream pi release (tracked in [`UPSTREAM`](../UPSTREAM)).
Every time upstream ships a new release, the `upstream-watch` workflow files an
auto-split issue and opens a tracking PR. Merged entries are recorded below.

| Date | Upstream | pi-rs action |
|---|---|---|
| 2026-10-28 | v0.74.2 | initial pin (behavior atoms + arena verified against this tag) |
| 2026-10-31 | main@35dfa7573 (v1.4.0 + 221 unreleased commits; issue #5) | audited all drift on the pi-rs behavior surface: provider/AI layer (streaming-json refactor, openrouter cost, constrained-sampling), agent proxy, session-manager, exec.ts, tools-manager — all atom-neutral for the ported surface. One real behavior change ported: grep `getFileLines` no longer treats a lone `\r` as a line break (src/tools/grep.rs). Help-text wording synced (args.rs). New atom scenario s10 (grep lone-CR context block) pins it — reverts to 13/17 without the fix. Verified: 186/186 atoms identical vs pi@main (built from source), mock e2e 3/3, deepseek-chat spot check algo/data/hard 3/3. Pin moved to the verified SHA; v1.4.0 remains the latest upstream tag. |
