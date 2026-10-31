#!/usr/bin/env bash
# Behavior-atom test: run real pi 0.74.2 and pi-rs through identical mock-LLM
# scenarios, then diff every behavior atom (prompt/tools/wire/stdout/exit/
# session/filesystem). Runs on the V100.
#
# Usage: bash run_atom.sh
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
PORT=8999
export PATH=/opt/node-v24.22.1/bin:$PATH
PI_BIN="${PI_BIN:-$HOME/pi-0.74.2/node_modules/.bin/pi}"
PIRS_BIN="${PIRS_BIN:-$HOME/pi-rs}"
set -a; source "$HOME/.config/xharness-bench/credentials.env"; set +a

# seed files copied into every arm workdir before the run
mkdir -p /tmp/atom-seed/sub
printf 'line one\nline two\nline three\n' > /tmp/atom-seed/sample.txt
printf 'needle_here\n' > /tmp/atom-seed/needle.txt
printf 'needle_here\nunrelated\nNEEDLE upper\n' > /tmp/atom-seed/sub/hay.txt
printf 'l1\nl2\nl3\nl4\nl5\n' > /tmp/atom-seed/big.txt
printf 'aaa\nbbb\nccc\n' > /tmp/atom-seed/multi.txt
mkdir -p /tmp/atom-seed/sub
printf 'needle_here\nunrelated\n' > /tmp/atom-seed/sub/hay.txt
# s10: lone \r must NOT act as a line break (upstream post-v1.4.0 getFileLines change)
printf 'alpha\rcr needle here\rbeta\ngamma\nmore needle\ntail\n' > /tmp/atom-seed/cr.txt

run_arm () {
  local arm="$1" scenario="$2" work="$3" rec="$4" port="$5" extra="$6"
  rm -rf "$rec" "$work"; mkdir -p "$rec" "$work"
  cp -r /tmp/atom-seed/. "$work/" 2>/dev/null || true
  python3 "$HERE/mock_server.py" "$scenario" "$rec" "$port" &
  local mock=$!
  sleep 0.6
  if [ "$arm" = pi ]; then
    (cd "$work" && MOCK_BASE_URL="http://127.0.0.1:$port" \
      "$PI_BIN" --no-extensions -e "$HERE/mock_provider.js" \
      --provider deepseek --model deepseek-chat --session-dir "$work/.sess" \
      $extra \
      "run the scenario" > "$rec/stdout.txt" 2> "$rec/stderr.txt")
  else
    (cd "$work" && PI_BASE_URL="http://127.0.0.1:$port" \
      "$PIRS_BIN" \
      --provider deepseek --model deepseek-chat --session-dir "$work/.sess" \
      $extra \
      "run the scenario" > "$rec/stdout.txt" 2> "$rec/stderr.txt")
  fi
  echo "$?" > "$rec/exit.txt"
  kill -9 $mock 2>/dev/null; wait $mock 2>/dev/null || true
}

atom_case () {
  local name="$1" scenario="$2"
  local base; base=$(mktemp -d "/tmp/atom-$name.XXXXXX")
  run_arm pi   "$scenario" "$base/w-pi"   "$base/rec-pi"   8991 "$TOOLS_FLAG"
  run_arm pirs "$scenario" "$base/w-pirs" "$base/rec-pirs" 8993 "$TOOLS_FLAG"

  local sess_pi sess_pirs
  sess_pi=$(ls "$base"/w-pi/.sess/*.jsonl | head -1)
  sess_pirs=$(ls "$base"/w-pirs/.sess/*.jsonl | head -1)

  local meta="$base/meta.json"
  python3 - "$meta" "$base" "$sess_pi" "$sess_pirs" <<'PYEOF'
import json, sys
meta, base, s_pi, s_pirs = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
json.dump({
 "stdout_pi":  open(f"{base}/rec-pi/stdout.txt").read(),
 "stdout_pirs": open(f"{base}/rec-pirs/stdout.txt").read(),
 "exit_pi":    int(open(f"{base}/rec-pi/exit.txt").read()),
 "exit_pirs":  int(open(f"{base}/rec-pirs/exit.txt").read()),
 "sess_pi":    s_pi, "sess_pirs": s_pirs,
 "fs_pi":      f"{base}/w-pi", "fs_pirs": f"{base}/w-pirs",
}, open(meta, "w"))
PYEOF
  echo "########## ATOM CASE: $name"
  python3 "$HERE/diff_atoms.py" "$base/rec-pi" "$base/rec-pirs" "$meta" | sed "s|^|$name |"
  echo "$base" > "/tmp/atom-$name.lastdir"
}

cat > /tmp/atom-s1.json <<'EOF'
[
 {"tool_calls":[{"id":"call_1","name":"bash","arguments":{"command":"printf 'hello atom\\nsecond line\\n' > marker.txt && cat marker.txt"}}]},
 {"text":"done"}
]
EOF

cat > /tmp/atom-s2.json <<'EOF'
[
 {"tool_calls":[{"id":"c1","name":"read","arguments":{"path":"sample.txt"}}]},
 {"tool_calls":[{"id":"c2","name":"edit","arguments":{"path":"sample.txt","edits":[{"oldText":"line two","newText":"line 2 (edited)"}]}}]},
 {"text":"edited"}
]
EOF

cat > /tmp/atom-s3.json <<'EOF'
[
 {"tool_calls":[{"id":"c1","name":"ls","arguments":{}}]},
 {"tool_calls":[{"id":"c2","name":"find","arguments":{"pattern":"**/*.txt"}}]},
 {"tool_calls":[{"id":"c3","name":"grep","arguments":{"pattern":"needle","path":"sub"}}]},
 {"echo_tool_results": true}
]
EOF

cat > /tmp/atom-s4.json <<'EOF'
[
 {"tool_calls":[{"id":"c1","name":"bash","arguments":{"command":"exit 3"}}]},
 {"tool_calls":[{"id":"c2","name":"edit","arguments":{"path":"sample.txt","edits":[{"oldText":"no-such-text","newText":"x"}]}}]},
 {"tool_calls":[{"id":"c3","name":"read","arguments":{"path":"sample.txt","offset":999}}]},
 {"echo_tool_results": true}
]
EOF

cat > /tmp/atom-s5.json <<'EOF'
[
 {"tool_calls":[{"id":"c1","name":"grep","arguments":{"pattern":"x","path":"."}}]},
 {"echo_tool_results": true}
]
EOF

TOOLS_FLAG=""
atom_case s1 /tmp/atom-s1.json
atom_case s2 /tmp/atom-s2.json
TOOLS_FLAG="--tools read,bash,edit,write,grep,find,ls"
atom_case s3 /tmp/atom-s3.json
TOOLS_FLAG=""
atom_case s4 /tmp/atom-s4.json
atom_case s5 /tmp/atom-s5.json

cat > /tmp/atom-s6.json <<'JSONEOF'
[
 {"tool_calls":[{"id":"r1","name":"read","arguments":{"path":"big.txt"}}]},
 {"tool_calls":[{"id":"r2","name":"read","arguments":{"path":"big.txt","offset":4}}]},
 {"tool_calls":[{"id":"r3","name":"read","arguments":{"path":"big.txt","offset":2,"limit":2}}]},
 {"echo_tool_results": true}
]
JSONEOF

cat > /tmp/atom-s7.json <<'JSONEOF'
[
 {"tool_calls":[{"id":"e1","name":"edit","arguments":{"path":"multi.txt","edits":[{"oldText":"aaa","newText":"AAA"},{"oldText":"ccc","newText":"CCC"}]}}]},
 {"tool_calls":[{"id":"e2","name":"edit","arguments":{"path":"multi.txt","edits":[{"oldText":"AAA\\nbbb","newText":"AAA\\nbbb (fuzzy-ok)"}]}}]},
 {"tool_calls":[{"id":"e3","name":"edit","arguments":{"path":"multi.txt","edits":[{"oldText":"AAA\\nbbb (fuzzy-ok)","newText":"XXX"},{"oldText":"bbb (fuzzy-ok)","newText":"YYY"}]}}]},
 {"echo_tool_results": true}
]
JSONEOF

cat > /tmp/atom-s8.json <<'JSONEOF'
[
 {"tool_calls":[{"id":"g1","name":"grep","arguments":{"pattern":"NEEDLE","path":"sub","ignoreCase":true}}]},
 {"tool_calls":[{"id":"g2","name":"grep","arguments":{"pattern":"needle.here","path":"sub","literal":true}}]},
 {"tool_calls":[{"id":"f1","name":"find","arguments":{"pattern":"sub/**/*.txt"}}]},
 {"tool_calls":[{"id":"l1","name":"ls","arguments":{"path":"sub","limit":1}}]},
 {"echo_tool_results": true}
]
JSONEOF

cat > /tmp/atom-s9.json <<'JSONEOF'
[
 {"tool_calls":[{"id":"b1","name":"bash","arguments":{"command":"printf out1; echo err1 1>&2; exit 7"}}]},
 {"tool_calls":[{"id":"b2","name":"bash","arguments":{"command":"yes | head -c 100000","timeout":2}}]},
 {"echo_tool_results": true}
]
JSONEOF

cat > /tmp/atom-s10.json <<'JSONEOF'
[
 {"tool_calls":[{"id":"g1","name":"grep","arguments":{"pattern":"needle","path":"cr.txt","context":2}}]},
 {"tool_calls":[{"id":"g2","name":"grep","arguments":{"pattern":"needle","path":"cr.txt"}}]},
 {"echo_tool_results": true}
]
JSONEOF
atom_case s6 /tmp/atom-s6.json
atom_case s7 /tmp/atom-s7.json
atom_case s8 /tmp/atom-s8.json
atom_case s9 /tmp/atom-s9.json
TOOLS_FLAG="--tools read,bash,edit,write,grep,find,ls"
atom_case s10 /tmp/atom-s10.json
TOOLS_FLAG=""
