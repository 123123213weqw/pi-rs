#!/usr/bin/env bash
# V100 parity + occupancy benchmark: pi (node) vs pi-rs (rust) on arena tasks.
# Usage: bash parity_v100.sh <tasks-src-dir> <arm> <task-ids...>
#   arm: "pi" | "pirs"
set -u
TASKS_SRC="$1"; ARM="$2"; shift 2
PI_BIN="${PI_BIN:-$HOME/pi-0.74.2/node_modules/.bin/pi}"
PIRS_BIN="${PIRS_BIN:-$HOME/pi-rs/target/release/pi}"
KEY_FILE="$HOME/.config/xharness-bench/credentials.env"
export PATH=/opt/node-v24.22.1/bin:$PATH

set -a; source "$KEY_FILE"; set +a   # exports DEEPSEEK_API_KEY etc (never printed)

run_arm () {
  local task_id="$1"
  local task_dir="$TASKS_SRC/$task_id"
  local work; work=$(mktemp -d "/tmp/parity-$ARM-$task_id.XXXXXX")
  cp -r "$task_dir/env/." "$work/"
  cd "$work"

  local prompt; prompt=$(cat "$task_dir/prompt.md")
  local t0 t1 maxrss exit_code
  t0=$(date +%s.%N)
  if [ "$ARM" = "pi" ]; then
    /usr/bin/time -v "$PI_BIN" --provider deepseek --model deepseek-chat \
      --session-dir "$work/.sess" "$prompt" > "$work/.agent.out" 2> "$work/.agent.err"
  else
    /usr/bin/time -v "$PIRS_BIN" --provider deepseek --model deepseek-chat \
      --session-dir "$work/.sess" "$prompt" > "$work/.agent.out" 2> "$work/.agent.err"
  fi
  exit_code=$?
  t1=$(date +%s.%N)
  maxrss=$(grep 'Maximum resident' "$work/.agent.err" | grep -o '[0-9]*')

  # verify: pytest tasks copy tests flat; script tasks keep secret/ layout
  local vout vres
  if [ -f "$task_dir/secret/verify.sh" ]; then
    cp -r "$task_dir/secret" "$work/secret"
    vout=$(cd "$work" && bash secret/verify.sh 2>&1 | tail -2)
    if echo "$vout" | grep -qi "verified"; then vres=PASS; else vres=FAIL; fi
  else
    cp -r "$task_dir/secret/." "$work/" 2>/dev/null
    vout=$(cd "$work" && python3 -m pytest test_*.py -q 2>&1 | tail -2)
    if echo "$vout" | grep -q "failed\|error"; then vres=FAIL; else vres=PASS; fi
  fi

  local dur
  dur=$(echo "$t1 $t0" | awk '{printf "%.1f", $1-$2}')
  echo "RESULT arm=$ARM task=$task_id verdict=$vres seconds=$dur max_rss_kb=${maxrss:-NA} agent_exit=$exit_code"
  echo "  workdir=$work"
  echo "  last_output: $(tail -c 200 "$work/.agent.out" | tr '\n' ' ')"
}

for t in "$@"; do
  run_arm "$t"
done
