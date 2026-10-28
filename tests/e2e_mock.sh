#!/usr/bin/env bash
# E2E mock test for pi-rs on node5: agent loop + tools + session JSONL + print mode.
set -e
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PI="$ROOT/target/release/pi"
WORK=$(mktemp -d /tmp/pi-rs-e2e.XXXXXX)
cd "$WORK"
PORT=8999

run_case () {
  local name="$1"; shift
  local scenario="$1"; shift
  python3 "$ROOT/tests/mock_server.py" "$scenario" $PORT &
  MOCK_PID=$!
  sleep 0.4
  local out
  out=$(PI_BASE_URL="http://127.0.0.1:$PORT" "$PI" --api-key test-key --provider deepseek --model mock \
        --session-dir "$WORK/sessions" -p "$@" 2>"$WORK/stderr.txt") || true
  kill $MOCK_PID 2>/dev/null || true
  wait $MOCK_PID 2>/dev/null || true
  echo "== case $name"
  echo "-- stdout: $out"
  echo "-- exit-status file: $(cat "$WORK/stderr.txt" 2>/dev/null | head -2)"
}

# --- Case 1: bash tool round-trip -------------------------------------------
cat > s1.json <<'EOF'
[
 {"tool_calls":[{"id":"call_1","name":"bash","arguments":{"command":"echo hello-from-tool > marker.txt && cat marker.txt"}}],
  "usage":{"prompt_tokens":120,"completion_tokens":30,"prompt_tokens_details":{"cached_tokens":64}}},
 {"text":"created the marker","usage":{"prompt_tokens":300,"completion_tokens":15,"prompt_tokens_details":{"cached_tokens":256}}}
]
EOF
run_case bash-roundtrip s1.json "create the marker"

echo "-- marker: $(cat marker.txt 2>/dev/null)"
echo "-- session files:"
ls sessions/
SESS=$(ls sessions/*.jsonl | head -1)
echo "-- session lines: $(wc -l < "$SESS")"
head -c 400 "$SESS"; echo
echo "-- usage entry check:"
grep -o '"usage":{[^}]*}' "$SESS" | head -2

# --- Case 2: read + edit round-trip -----------------------------------------
printf 'line one\nline two\nline three\n' > sample.txt
cat > s2.json <<'EOF'
[
 {"tool_calls":[{"id":"c1","name":"read","arguments":{"path":"sample.txt"}}]},
 {"tool_calls":[{"id":"c2","name":"edit","arguments":{"path":"sample.txt","edits":[{"oldText":"line two","newText":"line 2 (edited)"}]}}]},
 {"text":"edited ok","usage":{"prompt_tokens":500,"completion_tokens":10}}
]
EOF
run_case read-edit s2.json "edit sample"

echo "-- sample.txt after edit:"
cat sample.txt
grep -q "line 2 (edited)" sample.txt && echo "EDIT-OK" || echo "EDIT-FAIL"

# --- Case 3: ls/grep/find read-only tools -----------------------------------
mkdir -p sub && echo "needle_here" > sub/hay.txt
cat > s3.json <<'EOF'
[
 {"tool_calls":[{"id":"c1","name":"ls","arguments":{}}]},
 {"tool_calls":[{"id":"c2","name":"find","arguments":{"pattern":"**/*.txt"}}]},
 {"tool_calls":[{"id":"c3","name":"grep","arguments":{"pattern":"needle","path":"."}}]},
 {"echo_tool_results": true}
]
EOF
run_case readonly-tools s3.json "find the needle"
