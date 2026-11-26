#!/bin/sh
# wiki-consistency.sh —— openwiki 闭包核验（只读）。
#
# 三件事，都是「提交前必须为零」的：索引内相对链接逐一存在、必备文件清单存在、
# claims 证据闭包（仓内文件存在且 sha256 一致）。外加 AGENTS.md 行数预算——
# 硬约束面每会话自动注入，膨胀会直接摊薄所有约束的注意力。
#
# 用法：
#   wiki-consistency.sh [--repo <dir>] [--index <file>] [--json]
#   wiki-consistency.sh --help
#
# 参数：
#   --repo <dir>    缺省 = 脚本所在 git toplevel。
#   --index <file>  缺省 = <repo>/openwiki/overview.md。
#   --json          输出单行 JSON（与 k=v 同信息集）；缺省输出 k=v;k=v。
#
# 输出（exit 0，k=v）：
#   links=<n>;broken=<k>;first=<relpath|->;agents=<n>/<budget>;claims=<n>;claims_bad=<k>;ok=<0|1>
#   links  = 索引内相对 markdown 链接总数（http/https/# 开头的外链与锚点不计）。
#   broken = 链接目标不存在 / 必备文件缺失 / claims 证据坏的总数。
#   first  = 第一个 broken 的相对路径（无 = -）。仓库内文档路径，非业务数据。
#   claims = openwiki/.claims/** 内 claim 总数；claims_bad = 证据不解析或版本哈希不符，
#            证据指向 openwiki/ 自身亦计坏（生成页不能自证）。
#   ok     = 1 当且仅当 broken=0、agents≤预算、claims_bad=0、claims>0。
#
# exit 码：0 正常执行（结论在 ok 键）；2 用法错；3 环境缺失（无 python3/git）；
#          4 索引不可读；5 索引读取异常。「检测到问题」永远 exit 0 + ok=0。

set -eu

usage() {
  cat <<'EOF'
wiki-consistency.sh [--repo <dir>] [--index <file>] [--json]
  --help             this text
exit: 0 ok (see ok=), 2 usage, 3 env missing, 4 unreadable, 5 index read error
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

# ---------------------------------------------------------------- 默认模式（索引闭包）
[ -n "$INDEX" ] || INDEX="$REPO/openwiki/overview.md"

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
    'openwiki/overview.md',
    'openwiki/governance.md',
    'openwiki/quickstart.md',
    'openwiki/architecture/README.md',
    'openwiki/operating/vocabulary.md',
    'openwiki/operating/build.md',
    'openwiki/operating/dev-workflow.md',
    'openwiki/operating/example-pack.md',
    'openwiki/operating/h5-host.md',
]
for rel in required:
    fp = os.path.join(repo, rel)
    if not os.path.exists(fp):
        broken += 1
        if first == "-":
            first = rel

# AGENTS 行数预算：硬约束面是每会话自动注入的，膨胀直接摊薄所有约束的注意力。
# r12 前它在 277 行（超预算 27），是「操作程序混进硬约束」的可见信号——
# 分类判断无法机械化，但「什么时候该重新审视分类」可以。
agents_lines = 0
agents_path = os.path.join(repo, "AGENTS.md")
if os.path.exists(agents_path):
    agents_lines = sum(1 for _ in open(agents_path, encoding="utf-8"))
# claims 证据闭包：openwiki/.claims/**/*.json 里每条 evidence 必须解析到仓内真实文件，
# 且 `version` 的 sha256 与当前字节一致——页面被引用内容改动后证据会失真。
# 2026-11-26 事故：docs/ 整树删除后 6 条 evidence 仍指 `repo://docs/context/…`，
# 提交侧无校验、留到下一次 update run 才暴露；这里把它变成默认模式的 fail。
import glob, hashlib
try:
    import json
except Exception:
    json = None
claims_total = 0
claims_bad = 0
if json is not None:
    for cf in sorted(glob.glob(os.path.join(repo, 'openwiki/.claims/**/*.json'), recursive=True)):
        try:
            data = json.load(open(cf, 'r', encoding='utf-8'))
        except Exception:
            claims_bad += 1
            continue
        for claim in data.get('claims', []):
            claims_total += 1
            for ev in claim.get('evidence', []):
                res = str(ev.get('resource', ''))
                if not res.startswith('repo://'):
                    claims_bad += 1
                    continue
                rel = res[len('repo://'):]
                rel = rel.split('#')[0] or rel
                fp = os.path.join(repo, rel)
                if rel.startswith('openwiki/') or not os.path.isfile(fp):
                    claims_bad += 1
                    continue
                want = 'repo-file-v1:sha256:' + hashlib.sha256(open(fp, 'rb').read()).hexdigest()
                if str(ev.get('version', '')) != want:
                    claims_bad += 1
print("%d;%d;%s;%d;%d;%d" % (checked, broken, first, agents_lines, claims_total, claims_bad))
PYEOF
)

if [ "${RESULT#ERR:}" != "$RESULT" ]; then
  echo "$RESULT" >&2; exit 5
fi

LINKS=$(printf '%s' "$RESULT" | cut -d';' -f1)
BROKEN=$(printf '%s' "$RESULT" | cut -d';' -f2)
FIRST=$(printf '%s' "$RESULT" | cut -d';' -f3)
AGENTS=$(printf '%s' "$RESULT" | cut -d';' -f4)
CLAIMS=$(printf '%s' "$RESULT" | cut -d';' -f5)
CBAD=$(printf '%s' "$RESULT" | cut -d';' -f6)
AGENTS_BUDGET=160

if [ "$BROKEN" -eq 0 ] && [ "${AGENTS:-0}" -le "$AGENTS_BUDGET" ] && [ "${CBAD:-1}" -eq 0 ] && [ "${CLAIMS:-0}" -gt 0 ]; then OK=1; else OK=0; fi

if [ "$AS_JSON" -eq 1 ]; then
  printf '{"links":%s,"broken":%s,"first":"%s","agents":%s,"agents_budget":%s,"claims":%s,"claims_bad":%s,"ok":%s}\n' \
    "$LINKS" "$BROKEN" "$FIRST" "${AGENTS:-0}" "$AGENTS_BUDGET" "${CLAIMS:-0}" "${CBAD:-0}" "$OK"
else
  printf 'links=%s;broken=%s;first=%s;agents=%s/%s;claims=%s;claims_bad=%s;ok=%s\n' \
    "$LINKS" "$BROKEN" "$FIRST" "${AGENTS:-0}" "$AGENTS_BUDGET" "${CLAIMS:-0}" "${CBAD:-0}" "$OK"
fi
exit 0
