#!/bin/sh
# sessions-count.sh —— 子会话盘点（只读）。
#
# 纯命令行契约（rev3 / D-MA26）：不读 config、不读局 KV、不依赖 egg。
# 参数全部走 argv；缺省值取进程环境或固定约定路径。
#
# 用法：
#   sessions-count.sh --parent <id> [--sessions-root <dir>] [--trash-root <dir>] [--json]
#   sessions-count.sh --help
#
# 参数：
#   --parent <id>        必填。父（主）会话标识；对会话库内每个 .jsonl 做布尔 contains 匹配
#                        （与宿主 archive-sessions 选集规则同口径），内容零回显。
#                        形状：非空、不含 | 换行 / 控制字符。
#   --sessions-root <dir> 缺省 = $YGG_SESSIONS_ROOT，或 $HOME/.pi/agent/sessions/--<cwd-slug>--。
#                        slug = cwd 去前导 / 后把 / 换成 -（与宿主 cwd_slug 同口径）。
#   --trash-root <dir>   缺省 = $YGG_SESSIONS_TRASH，或 $HOME/.pi/agent/sessions-trash。
#   --json               输出单行 JSON（与 k=v 同信息集）；缺省输出 k=v;k=v。
#
# 输出（exit 0，k=v 缺省）：
#   total=<t>;matching=<m>;trash=<x>;ok=<0|1>
#   total    = 会话库内 .jsonl 总数（库不存在 = 0）。
#   matching = 含 parent 的 .jsonl 数（结算不变式：matching=0 即全部归档）。
#   trash    = trash 根下 .jsonl 数（trash 根不存在 = 0）。
#   ok       = 1 当且仅当 matching=0。
#
# exit 码：0 正常执行（结论在 ok 键）；2 用法错；3 环境缺失（$HOME 缺席且未给根）；4 目标资源不可读。
# 「检测发现问题」（matching>0）永远 exit 0 + ok=0，与 malfunction 严格区分。
#
# 红线：不读 *.xlsx/*.csv 等业务数据；只按扩展名 .jsonl 入选；文件内容从不回显（grep -q 布尔）。

set -eu

usage() {
  cat <<'EOF'
sessions-count.sh --parent <id> [--sessions-root <dir>] [--trash-root <dir>] [--json]
  --parent <id>         required: parent session id (boolean match, no content echo)
  --sessions-root <dir> default: $YGG_SESSIONS_ROOT or $HOME/.pi/agent/sessions/--<cwd-slug>--
  --trash-root <dir>    default: $YGG_SESSIONS_TRASH or $HOME/.pi/agent/sessions-trash
  --json                emit single-line JSON instead of k=v;k=v
  --help                this text
exit: 0 ok (see ok=), 2 usage, 3 env missing, 4 unreadable
EOF
}

PARENT=""
SESSIONS_ROOT=""
TRASH_ROOT=""
AS_JSON=0

while [ $# -gt 0 ]; do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --parent) PARENT="${2-}"; shift 2 ;;
    --sessions-root) SESSIONS_ROOT="${2-}"; shift 2 ;;
    --trash-root) TRASH_ROOT="${2-}"; shift 2 ;;
    --json) AS_JSON=1; shift ;;
    *) echo "unknown arg: $1" >&2; usage >&2; exit 2 ;;
  esac
done

# parent 必填 + 形状（禁 | 换行 控制字符）
case "$PARENT" in
  "") echo "missing required --parent <id>" >&2; exit 2 ;;
esac
if printf '%s' "$PARENT" | grep -q '[|]'; then
  echo "parent must not contain |" >&2; exit 2
fi
if printf '%s' "$PARENT" | grep -q "$(printf '\r')"; then
  echo "parent must not contain CR" >&2; exit 2
fi

# cwd slug（与宿主 cwd_slug 同口径）
cwd_slug() {
  pwd | sed -e 's#^/##' -e 's#/#-#g'
}

# 解析 sessions-root 缺省
if [ -z "$SESSIONS_ROOT" ]; then
  if [ -n "${YGG_SESSIONS_ROOT-}" ]; then
    SESSIONS_ROOT="$YGG_SESSIONS_ROOT"
  else
    [ -n "${HOME-}" ] || { echo "HOME not set and --sessions-root not given" >&2; exit 3; }
    SESSIONS_ROOT="$HOME/.pi/agent/sessions/--$(cwd_slug)--"
  fi
fi

# 解析 trash-root 缺省
if [ -z "$TRASH_ROOT" ]; then
  if [ -n "${YGG_SESSIONS_TRASH-}" ]; then
    TRASH_ROOT="$YGG_SESSIONS_TRASH"
  else
    [ -n "${HOME-}" ] || { echo "HOME not set and --trash-root not given" >&2; exit 3; }
    TRASH_ROOT="$HOME/.pi/agent/sessions-trash"
  fi
fi

# 计数：库不存在 = 0（合法：无会话可清）
count_jsonl() {
  # $1 = dir；不存在或不可读返回 0
  dir="$1"
  if [ ! -d "$dir" ]; then
    echo 0; return
  fi
  # find 只按扩展名入选；不打开内容
  n=$(find "$dir" -maxdepth 1 -type f -name '*.jsonl' 2>/dev/null | wc -l | tr -d ' ')
  echo "$n"
}

# matching：对 sessions-root 内每个 .jsonl 做布尔 contains(parent) 匹配。
# find -print0 喂 xargs -0 grep -lF（NUL 分隔输入，含空格/换行文件名安全）：
# grep -l 只打印命中文件名（内容零回显），grep -F = 字面量（parent 不当正则），LC_ALL=C 防多字节误配。
# 命中文件每行一个 → wc -l 计数。不用 grep -Z（GNU 专有 NUL 输出；macOS BSD grep 的 -Z 语义不同）。
matching_count() {
  dir="$1"
  if [ ! -d "$dir" ]; then
    echo 0; return
  fi
  m=$(find "$dir" -maxdepth 1 -type f -name '*.jsonl' -print0 2>/dev/null \
       | LC_ALL=C xargs -0 grep -lF -- "$PARENT" 2>/dev/null \
       | wc -l | tr -d ' ')
  echo "${m:-0}"
}

TOTAL=$(count_jsonl "$SESSIONS_ROOT")
MATCHING=$(matching_count "$SESSIONS_ROOT")
TRASH=$(count_jsonl "$TRASH_ROOT")

if [ "$MATCHING" -eq 0 ]; then OK=1; else OK=0; fi

if [ "$AS_JSON" -eq 1 ]; then
  printf '{"total":%s,"matching":%s,"trash":%s,"ok":%s}\n' "$TOTAL" "$MATCHING" "$TRASH" "$OK"
else
  printf 'total=%s;matching=%s;trash=%s;ok=%s\n' "$TOTAL" "$MATCHING" "$TRASH" "$OK"
fi
exit 0
