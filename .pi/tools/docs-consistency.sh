#!/bin/sh
# docs-consistency.sh —— docs wiki 闭包核验（只读）。
#
# 纯命令行契约（rev3 / D-MA26）：不读 config、不读局 KV、不依赖 egg。
# 校验 docs/README.md 索引内的相对链接逐一存在 + 必备文件清单存在。
#
# 用法：
#   docs-consistency.sh [--repo <dir>] [--index <file>] [--json]
#   docs-consistency.sh --help
#
# 参数：
#   --repo <dir>   缺省 = 脚本所在 git toplevel。
#   --index <file> 缺省 = <repo>/docs/README.md。
#   --json         输出单行 JSON（与 k=v 同信息集）；缺省输出 k=v;k=v。
#
# 输出（exit 0，k=v 缺省）：
#   links=<n>;broken=<k>;first=<relpath|->;ok=<0|1>
#   links  = 索引内相对 markdown 链接总数（(http/https/# 开头的外链与锚点不计）。
#   broken = 链接目标文件不存在 / 必备文件缺失的总数。
#   first  = 第一个 broken 的相对路径（无 = -）。仓库内文档路径，非业务数据。
#   ok     = 1 当且仅当 broken=0。
#
# exit 码：0 正常执行（结论在 ok 键）；2 用法错；3 环境缺失（无 python3/git）；4 索引不可读。
# 「检测发现问题」（broken>0）永远 exit 0 + ok=0。

set -eu

usage() {
  cat <<'EOF'
docs-consistency.sh [--repo <dir>] [--index <file>] [--json]
  --repo <dir>   default: git toplevel of the script location
  --index <file> default: <repo>/docs/README.md
  --json         emit single-line JSON instead of k=v;k=v
  --help         this text
exit: 0 ok (see ok=), 2 usage, 3 env missing (python3/git), 4 index unreadable
EOF
}

REPO=""
INDEX=""
AS_JSON=0

while [ $# -gt 0 ]; do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --repo) REPO="${2-}"; shift 2 ;;
    --index) INDEX="${2-}"; shift 2 ;;
    --json) AS_JSON=1; shift ;;
    *) echo "unknown arg: $1" >&2; usage >&2; exit 2 ;;
  esac
done

command -v python3 >/dev/null 2>&1 || { echo "python3 required" >&2; exit 3; }

if [ -z "$REPO" ]; then
  command -v git >/dev/null 2>&1 || { echo "git required (or pass --repo)" >&2; exit 3; }
  REPO=$(git rev-parse --show-toplevel 2>/dev/null) || { echo "not in a git repo; pass --repo" >&2; exit 3; }
fi
[ -n "$INDEX" ] || INDEX="$REPO/docs/README.md"

[ -r "$INDEX" ] || { echo "index not readable: $INDEX" >&2; exit 4; }

RESULT=$(python3 - "$INDEX" "$REPO" <<'PYEOF'
import sys, re, os
index_path, repo = sys.argv[1], sys.argv[2]
try:
    text = open(index_path, 'r', encoding='utf-8', errors='replace').read()
except Exception as e:
    print("ERR:index-read:%s" % e); sys.exit(0)

base = os.path.dirname(index_path)
# markdown 相对链接 [text](path)：排除 http(s):、mailto:、# 锚点、绝对路径协议
link_re = re.compile(r'\[[^\]]*\]\(([^)]+)\)')
links = []
for m in link_re.finditer(text):
    target = m.group(1).strip()
    # 去掉可选标题 "path \"title\""
    target = target.split()[0] if target else target
    if not target:
        continue
    if target.startswith(('http://', 'https://', 'mailto:', '#')):
        continue
    # 去掉锚点
    target = target.split('#')[0]
    if not target:
        continue
    links.append(target)

broken = 0
first = "-"
checked = 0
for rel in links:
    checked += 1
    fp = os.path.normpath(os.path.join(base, rel))
    if not os.path.exists(fp):
        broken += 1
        if first == "-":
            first = rel

# 必备文件清单（wiki 骨架）
required = [
    'docs/README.md',
    'docs/wiki-governance.md',
    'docs/context/vocabulary.md',
    'docs/context/principles.md',
    'docs/context/decisions-agent.md',
    'docs/context/decisions-engine.md',
]
for rel in required:
    fp = os.path.join(repo, rel)
    if not os.path.exists(fp):
        broken += 1
        if first == "-":
            first = rel

print("%d;%d;%s" % (checked, broken, first))
PYEOF
)

if [ "${RESULT#ERR:}" != "$RESULT" ]; then
  echo "$RESULT" >&2; exit 5
fi

LINKS=$(printf '%s' "$RESULT" | cut -d';' -f1)
BROKEN=$(printf '%s' "$RESULT" | cut -d';' -f2)
FIRST=$(printf '%s' "$RESULT" | cut -d';' -f3-)

if [ "$BROKEN" -eq 0 ]; then OK=1; else OK=0; fi

if [ "$AS_JSON" -eq 1 ]; then
  printf '{"links":%s,"broken":%s,"first":"%s","ok":%s}\n' "$LINKS" "$BROKEN" "$FIRST" "$OK"
else
  printf 'links=%s;broken=%s;first=%s;ok=%s\n' "$LINKS" "$BROKEN" "$FIRST" "$OK"
fi
exit 0
