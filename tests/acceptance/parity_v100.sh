#!/usr/bin/env bash
# Differential acceptance on V100: pi 1.4.0 vs pi-rs on the same 4 tasks.
set -u
A=/tmp/acceptance
PASS=0; FAIL=0
verdict () { if [ "$2" = 0 ]; then echo "PASS $1"; PASS=$((PASS+1)); else echo "FAIL $1"; FAIL=$((FAIL+1)); fi }

run_arm () {
  local arm="$1" bin="$2"
  export PATH=/opt/node-v24.22.1/bin:$PATH
  set -a; source ~/.config/xharness-bench/credentials.env; set +a
  PASS=0; FAIL=0

  rm -rf "$A/w-$arm-a"; mkdir -p "$A/w-$arm-a"; cp -R "$A/bugfix/" "$A/w-$arm-a/"
  (cd "$A/w-$arm-a" && /usr/bin/time -v "$bin" -p "test_median.py 有测试失败。修复 med.py 让全部测试通过。只允许修改 med.py。" >out.txt 2>time.txt)
  python3 -m pytest -q test_median.py >/dev/null 2>&1; verdict "$arm/A-bugfix" $?
  echo "   $(grep 'Maximum resident' time.txt | grep -o '[0-9]*')KB $(grep 'Elapsed' time.txt | grep -o '[0-9:.]*$')s"

  rm -rf "$A/w-$arm-b"; mkdir -p "$A/w-$arm-b"; cp -R "$A/feature/" "$A/w-$arm-b/"
  (cd "$A/w-$arm-b" && /usr/bin/time -v "$bin" -p "实现 lru.py：LRUCache 类，构造参数 capacity；get(key) 命中返回值、miss 返回 -1；put(key,value) 容量满时淘汰最久未使用的条目；get 会刷新近期度。让 test_lru.py 全部通过。" >out.txt 2>time.txt)
  python3 -m pytest -q test_lru.py >/dev/null 2>&1; verdict "$arm/B-feature" $?
  echo "   $(grep 'Maximum resident' time.txt | grep -o '[0-9]*')KB $(grep 'Elapsed' time.txt | grep -o '[0-9:.]*$')s"

  cd "$A"
  ANS=$(/usr/bin/time -v "$bin" -p "读 /home/wzu/acceptance/schema_ref.txt，回答：1) Task 类定义了哪些字段（逐个列出字段名）2) 任务目录中的任务定义文件名是什么？只输出答案。" 2>time.txt)
  OK=0
  for tok in id domain difficulty points verifier timeout_minutes task.yaml; do
    echo "$ANS" | grep -q "$tok" || OK=1
  done
  verdict "$arm/C-comprehension" $OK

  "$bin" -p "记住暗号: quokka。只回复 OK。" >/dev/null 2>&1
  ANS2=$("$bin" --continue -p "暗号是什么？只输出那个词" 2>/dev/null)
  echo "$ANS2" | grep -q quokka; verdict "$arm/D-continuity" $?
  echo "== $arm: $PASS passed, $FAIL failed"
}
