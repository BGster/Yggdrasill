#!/bin/sh
# todo-check.sh —— 主会话 Todo 盘点（只读）。
#
# 纯命令行契约（rev3 / D-MA26）：不读 config、不读局 KV、不依赖 egg。
#
# 用法：
#   todo-check.sh [--session <id>] [--sessions-root <dir>] [--json]
#   todo-check.sh --help
#
# 参数：
#   --session <id>       会话 id（jsonl 文件名去掉时间戳前缀与 .jsonl 后缀的 uuid 段）。
#                        缺省 = $YGG_SESSION_ID，或 $PI_SESSION_ID。两者皆无 → exit 2。
#   --sessions-root <dir> 缺省 = $YGG_SESSIONS_ROOT，或 $HOME/.pi/agent/sessions/--<cwd-slug>--。
#                        slug = cwd 去前导 / 后把 / 换成 -（与宿主 cwd_slug 同口径）。
#   --json               输出单行 JSON（与 k=v 同信息集）；缺省输出 k=v;k=v。
#
# 语义：定位该会话的 .jsonl 转录档，取最后一条 todo 工具状态，统计「全状态条目」
#   （纪律 = 连已完成历史项一并清，remaining 计未完成 pending/in_progress 之外的口径由
#    转录内 todo 记录的 status 字段决定：pending/in_progress = 未完成）。
#   remaining = 未完成任务数；active = 1 当且仅当存在任何 todo 记录（用于区分「无 todo」与「已清零」）。
#
# 输出（exit 0，k=v 缺省）：
#   found=<0|1>;total=<n>;remaining=<r>;active=<0|1>;ok=<0|1>
#   found    = 1 当且仅当定位到会话转录档。
#   total    = 最后一条 todo 状态里的条目总数（无 todo 记录 = 0）。
#   remaining= 未完成任务数（无 todo 记录 = 0）。
#   active   = 1 当且仅当存在任何 todo 记录。
#   ok       = 1 当且仅当 remaining=0。
#
# exit 码：0 正常执行（结论在 ok 键）；2 用法错（含 session 缺失）；3 环境缺失；4 转录档不可读。
# 「检测发现问题」（remaining>0）永远 exit 0 + ok=0，与 malfunction 严格区分。
#
# 红线：不读 *.xlsx/*.csv 等业务数据；只按扩展名 .jsonl 入选；todo 条目内容（任务文本）不回显，
#       只输出计数。转录解析用 python3（若可用）提取最后 todo 工具调用的结构化状态；
#       python3 缺席时退化为「找到最后 todo 工具行行号 + 行内 pending/in_progress 计数」的近似，
#       并在输出加 approx=1 标记。

set -eu

usage() {
  cat <<'EOF'
todo-check.sh [--session <id>] [--sessions-root <dir>] [--json]
  --session <id>        default: $YGG_SESSION_ID or $PI_SESSION_ID
  --sessions-root <dir> default: $YGG_SESSIONS_ROOT or $HOME/.pi/agent/sessions/--<cwd-slug>--
  --json                emit single-line JSON instead of k=v;k=v
  --help                this text
exit: 0 ok (see ok=), 2 usage/session missing, 3 env missing, 4 transcript unreadable
EOF
}

SESSION=""
SESSIONS_ROOT=""
AS_JSON=0

while [ $# -gt 0 ]; do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --session) SESSION="${2-}"; shift 2 ;;
    --sessions-root) SESSIONS_ROOT="${2-}"; shift 2 ;;
    --json) AS_JSON=1; shift ;;
    *) echo "unknown arg: $1" >&2; usage >&2; exit 2 ;;
  esac
done

# session 缺省链
if [ -z "$SESSION" ]; then
  if [ -n "${YGG_SESSION_ID-}" ]; then
    SESSION="$YGG_SESSION_ID"
  elif [ -n "${PI_SESSION_ID-}" ]; then
    SESSION="$PI_SESSION_ID"
  fi
fi
[ -n "$SESSION" ] || { echo "no --session and neither YGG_SESSION_ID nor PI_SESSION_ID set" >&2; exit 2; }

cwd_slug() { pwd | sed -e 's#^/##' -e 's#/#-#g'; }

if [ -z "$SESSIONS_ROOT" ]; then
  if [ -n "${YGG_SESSIONS_ROOT-}" ]; then
    SESSIONS_ROOT="$YGG_SESSIONS_ROOT"
  else
    [ -n "${HOME-}" ] || { echo "HOME not set and --sessions-root not given" >&2; exit 3; }
    SESSIONS_ROOT="$HOME/.pi/agent/sessions/--$(cwd_slug)--"
  fi
fi

[ -d "$SESSIONS_ROOT" ] || { echo "sessions-root not a directory: $SESSIONS_ROOT" >&2; exit 4; }

# 定位会话转录档：文件名含 session id 的 .jsonl（时间戳前缀_uuid.jsonl）
TRANSCRIPT=$(find "$SESSIONS_ROOT" -maxdepth 1 -type f -name "*${SESSION}*.jsonl" 2>/dev/null | head -1)
[ -n "$TRANSCRIPT" ] || { echo "no transcript matching session: $SESSION" >&2; exit 4; }
[ -r "$TRANSCRIPT" ] || { echo "transcript not readable: $TRANSCRIPT" >&2; exit 4; }

# 用 python3 重放 todo 工具调用序列（状态机），算最终 total/remaining（内容零回显：只回计数）。
# 真实转录结构（pi）：content[] 里 type=toolCall、name=todo、arguments={action,...}。
# 契约：create 建清单（tasks[]，id 从 1 起）→ update 按 id 改 status → delete 按 id 删 → clear 清空。
# 兼容旧形（type=tool_use、input/args）与批量 tasks[] 替换。
if command -v python3 >/dev/null 2>&1; then
  RESULT=$(python3 - "$TRANSCRIPT" <<'PYEOF'
import sys, json
path = sys.argv[1]

OPEN = ('pending','in_progress','in-progress','todo','open','blocked')

# 任务表：id -> status（dict 保持插入序，id 用 int/str 原样）
tasks = {}
order = []
saw_todo = False

def norm_status(s):
    return str(s or '').lower()

def apply_call(args):
    global saw_todo
    if not isinstance(args, dict):
        return
    saw_todo = True
    action = str(args.get('action','') or '').lower()
    if action == 'create' or (not action and isinstance(args.get('tasks'), list)):
        # 建/替换整表：tasks[] 每项可带 id/status，缺 id 按位置编号
        new = args.get('tasks')
        if isinstance(new, list):
            for i, t in enumerate(new, start=1):
                if isinstance(t, dict):
                    tid = t.get('id', i)
                    st = norm_status(t.get('status','pending'))
                else:
                    tid = i; st = 'pending'
                if tid not in tasks:
                    order.append(tid)
                tasks[tid] = st
    elif action == 'update':
        tid = args.get('id')
        st = norm_status(args.get('status',''))
        if tid is not None:
            if tid not in tasks:
                order.append(tid)
            if st:
                tasks[tid] = st
    elif action == 'delete':
        tid = args.get('id')
        if tid in tasks:
            del tasks[tid]
            if tid in order:
                order.remove(tid)
    elif action == 'clear':
        tasks.clear(); order.clear()
    elif action == 'list':
        pass  # 查询不改状态
    else:
        # 未知 action：若带 tasks[] 按替换处理
        new = args.get('tasks')
        if isinstance(new, list):
            for i, t in enumerate(new, start=1):
                if isinstance(t, dict):
                    tid = t.get('id', i); st = norm_status(t.get('status','pending'))
                else:
                    tid = i; st = 'pending'
                if tid not in tasks:
                    order.append(tid)
                tasks[tid] = st

try:
    with open(path, 'r', encoding='utf-8', errors='replace') as fh:
        for line in fh:
            if 'todo' not in line:
                continue
            try:
                obj = json.loads(line)
            except Exception:
                continue
            msg = obj.get('message', obj)
            content = msg.get('content') if isinstance(msg, dict) else None
            items = []
            if isinstance(content, list):
                items = content
            elif isinstance(content, dict):
                items = [content]
            for it in items:
                if not isinstance(it, dict):
                    continue
                name = str(it.get('name','') or it.get('tool','') or '')
                if 'todo' not in name.lower():
                    continue
                # 参数面：arguments（pi toolCall）/ input / args（其它 harness）
                args = it.get('arguments', it.get('input', it.get('args', {})))
                if isinstance(args, str):
                    try:
                        args = json.loads(args)
                    except Exception:
                        continue
                apply_call(args)
except Exception as e:
    print("ERR:%s" % e)
    sys.exit(0)

if not saw_todo:
    print("0;0;0")  # total remaining active（无 todo 记录）
else:
    total = len(tasks)
    remaining = sum(1 for tid in tasks if tasks[tid] in OPEN)
    active = 1 if (total > 0 or saw_todo) else 0
    print("%d;%d;%d" % (total, remaining, active))
PYEOF
)
  if [ "${RESULT#ERR:}" != "$RESULT" ]; then
    echo "python3 parse error: $RESULT" >&2; exit 5
  fi
  TOTAL=$(printf '%s' "$RESULT" | cut -d';' -f1)
  REMAINING=$(printf '%s' "$RESULT" | cut -d';' -f2)
  ACTIVE=$(printf '%s' "$RESULT" | cut -d';' -f3)
  FOUND=1
  APPROX=0
else
  # 无 python3：近似——统计转录里 todo 相关行数，remaining 取最后一次出现的 pending/in_progress 计数
  FOUND=1
  TOTAL=$(grep -c 'todo' "$TRANSCRIPT" 2>/dev/null || echo 0)
  REMAINING=$(grep -oE '"(pending|in_progress)"' "$TRANSCRIPT" 2>/dev/null | wc -l | tr -d ' ')
  ACTIVE=$([ "$TOTAL" -gt 0 ] && echo 1 || echo 0)
  APPROX=1
fi

if [ "$REMAINING" -eq 0 ]; then OK=1; else OK=0; fi

if [ "$AS_JSON" -eq 1 ]; then
  printf '{"found":%s,"total":%s,"remaining":%s,"active":%s,"ok":%s,"approx":%s}\n' \
    "$FOUND" "$TOTAL" "$REMAINING" "$ACTIVE" "$OK" "$APPROX"
else
  printf 'found=%s;total=%s;remaining=%s;active=%s;ok=%s;approx=%s\n' \
    "$FOUND" "$TOTAL" "$REMAINING" "$ACTIVE" "$OK" "$APPROX"
fi
exit 0
