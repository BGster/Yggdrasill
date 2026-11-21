#!/bin/sh
# final-verify.sh —— P4「superagent game」终验四条冒烟（计划 §6 逐条命令的脚本化）。
#
# 用法（仓库根 = 泳道工作树根；需先构建 egg 并可用）：
#   YGG_PACKAGES_ROOT=packages/libs packages/mods/superagent/test/final-verify.sh
#
# 前置：/tmp/ygg-p4-mount 由本脚本自建（pack.json mounts 声明的合成数据目录）。
# 执行序（共享挂载的写序敏感性：④ 读 A 局产物，必须在 B/C 重放局写同一文件之前）：
#   ① 开一局 → ② headless 任务流 → ④ 挂载点读写 → ②' 重放对账 → ③ agent 全链 + serve
# 输出：逐条 PASS/FAIL；全绿 EXIT=0，任一红 EXIT=1。
set -eu

REPO=$(git rev-parse --show-toplevel)
EGG="$REPO/.ygg/bin/egg"
[ -x "$EGG" ] || EGG="egg"
RUNS_A=/tmp/ygg-p4-final-a
RUNS_B=/tmp/ygg-p4-final-b
RUNS_C=/tmp/ygg-p4-final-c
MOUNT=/tmp/ygg-p4-mount

fails=0
note() { printf '%s\n' "$*"; }
pass() { printf 'PASS %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1"; fails=$((fails + 1)); }

# ── 前置（终验目录归零：脚本自建，不继承旧局）─────────────────────────────────
rm -rf "$RUNS_A" "$RUNS_B" "$RUNS_C"
mkdir -p "$MOUNT" "$RUNS_A" "$RUNS_B" "$RUNS_C"
export YGG_PACKAGES_ROOT=packages/libs
cd "$REPO"

# ── ① 开一局 ────────────────────────────────────────────────────────────────
out=$("$EGG" new --mod packages/mods/superagent --in "$RUNS_A" --event event.start 2>&1) && fv_rc=0 || fv_rc=$?
ra=$(find "$RUNS_A" -maxdepth 1 -type d -name "r-*" | sort | head -1)
if [ "$fv_rc" -eq 0 ] && printf '%s' "$out" | grep -q "run-root: ${ra}"; then
    pass "① 开一局 EXIT=0 + run-root 行（${ra}）"
else
    fail "① 开一局（rc=${fv_rc}；ra=${ra}；out 尾行：$(printf '%s' "$out" | tail -1)）"
fi

# ── ② 完整任务流（headless 事件序：委托→执行→交付）──────────────────────────
"$EGG" open "$ra" --event commission.take --payload '"c1|clerk:clerk-mo"' > /dev/null 2>&1 || true
"$EGG" open "$ra" --event commission.work > /dev/null 2>&1 || true
"$EGG" open "$ra" --event commission.deliver --payload '"c1|按新格式归档完毕"' > /dev/null 2>&1 || true
sum_a=$("$EGG" open "$ra" --event agent.summary --raw 2>/dev/null)
done_a=$(printf '%s' "$sum_a" | python3 -c "import json,sys;d=json.load(sys.stdin);print(d['meta']['summary']['quests']['done'])")
if [ "$done_a" = "1" ]; then
    pass "② 完整任务流：委托→执行→交付，quest.completed 证据链 done=1"
else
    fail "② 完整任务流 done=${done_a}"
fi

# ── ④ 挂载点读写（② 的 work 已写穿并在 handler 内回读校验；此处独立复核）────
# 共享挂载：本段必须在 B/C 重放局写同名文件之前执行（ harness 交付会覆写 c1 报告）。
report="$MOUNT/biz-report-c1.txt"
if [ -f "$report" ] && [ -e "$ra/mounts/biz" ] && [ -f "$ra/fs.jsonl" ]; then
    # 干活账在推进前记账：day 取「本动作开始时」的业务日（entry+take 之后 = 2）。
    expect="bureau-report|c1|day=2|taker=墨尘"
    actual=$(cat "$report")
    if [ "$actual" = "$expect" ]; then
        pass "④ 挂载点读写：<run>/mounts/biz 落点（软链）+ fs.jsonl 审计 + 字节逐等"
    else
        fail "④ 挂载文件字节不符：实得「${actual}」"
    fi
else
    fail "④ 挂载落点缺失（report=${report} mounts=${ra}/mounts/biz fs.jsonl）"
fi

# ── ②' 重放对账（同信封序列 sim.hash 逐字节相等；异序列对照）────────────────
run_seq() {
    # POSIX sh 无 local：跑在子 shell 里，变量名 fv_ 前缀避免污染调用方。
    fv_runs="$1"; fv_mode="$2"
    rm -rf "$fv_runs" && "$EGG" new --mod packages/mods/superagent --in "$fv_runs" --event event.start > /dev/null 2>&1
    fv_r=$(find "$fv_runs" -maxdepth 1 -type d -name "r-*" | sort | head -1)
    if [ "$fv_mode" = "harness" ]; then
        "$EGG" open "$fv_r" --event commission.take --payload '"c1|ext:agent-7"' > /dev/null 2>&1
        "$EGG" open "$fv_r" --requirement 'agent-7|c1 干得怎么样了' > /dev/null 2>&1
        "$EGG" open "$fv_r" --wait-tools > /dev/null 2>&1
    else
        "$EGG" open "$fv_r" --event commission.take --payload '"c1|clerk:clerk-mo"' > /dev/null 2>&1
    fi
    "$EGG" open "$fv_r" --event agent.summary --raw 2>/dev/null | python3 -c "
import json,sys
d=json.load(sys.stdin)
s=d['meta']['summary']
print(s['lastSimHash'], s['quests']['done'])
"
}
h_a=$(run_seq "$RUNS_B" harness)
h_b=$(run_seq "$RUNS_B" harness)
h_c=$(run_seq "$RUNS_C" clerk)
ah=$(printf '%s' "$h_a" | cut -d' ' -f1)
bh=$(printf '%s' "$h_b" | cut -d' ' -f1)
ch=$(printf '%s' "$h_c" | cut -d' ' -f1)
if [ "$ah" = "$bh" ] && [ "$ah" != "$ch" ] && [ "$ah" != "unset" ]; then
    pass "②' 重放对账：同信封序列 sim.hash 逐字节相等（${ah}）∧ 异序列不同值（${ch}）——断言非常数"
else
    fail "②' 重放对账 A=${ah} B=${bh} C=${ch}"
fi

# ── ③ agent 对话参与（inbox → 回执 → 交付；serve 只读面，观察 B 局）──────────
rb=$(find "$RUNS_B" -maxdepth 1 -type d -name "r-*" | sort | head -1)
sum_b=$("$EGG" open "$rb" --event agent.summary --raw 2>/dev/null)
done_b=$(printf '%s' "$sum_b" | python3 -c "import json,sys;d=json.load(sys.stdin);print(d['meta']['summary']['quests']['done'])")
lines_b=$(printf '%s' "$sum_b" | python3 -c "import json,sys;d=json.load(sys.stdin);print(d['meta']['dlg']['lines'])")
if [ "$done_b" = "1" ] && [ "$lines_b" -ge 1 ]; then
    pass "③ inbox 注入→agent 回执→交付：done=1，对话线程 ${lines_b} 行"
else
    fail "③ 全链路 done=${done_b} lines=${lines_b}"
fi

# serve 只读面（127.0.0.1 固定高位端口；观察根注册 → 面板 → events API）
PORT=17890
"$EGG" serve --port $PORT > /tmp/ygg-p4-serve.log 2>&1 &
serve_pid=$!
serve_ok=0
for _ in 1 2 3 4 5 6 7 8 9 10; do
    sleep 1
    if curl -s "http://127.0.0.1:$PORT/api/roots" | grep -q '"roots"'; then
        serve_ok=1
        break
    fi
done
if [ "$serve_ok" = "1" ]; then
    curl -s -X POST "http://127.0.0.1:$PORT/api/roots" -H 'Content-Type: application/json' -d "{\"root\": \"$RUNS_B\"}" > /dev/null
    panel=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/panel/$(basename "$rb")/index.html")
    ev=$(curl -s "http://127.0.0.1:$PORT/api/runs/$(basename "$rb")/events?limit=50")
    if [ "$panel" = "200" ] && printf '%s' "$ev" | grep -q 'agent.chat'; then
        pass "③ serve 只读面：/panel 200 + events API JSON 原文含 agent.chat 对话行"
    else
        fail "③ serve 面板=${panel} events 含 agent.chat=$(printf '%s' "$ev" | grep -c agent.chat || true)"
    fi
    curl -s -X DELETE "http://127.0.0.1:$PORT/api/roots?root=$RUNS_B" > /dev/null
else
    fail "③ serve 未就绪（/tmp/ygg-p4-serve.log）"
fi
kill "$serve_pid" 2>/dev/null || true
wait "$serve_pid" 2>/dev/null || true

# ── 收口 ────────────────────────────────────────────────────────────────────
if [ "$fails" -eq 0 ]; then
    note "FINAL-VERIFY: 4/4 PASS"
    exit 0
fi
note "FINAL-VERIFY: ${fails} 项 FAIL"
exit 1
