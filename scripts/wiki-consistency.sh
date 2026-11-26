#!/bin/sh
# wiki-consistency.sh —— openwiki 闭包核验（只读）+ 场景签名验真（--attest）。
#
# 纯命令行契约（rev3 / harness 机证）：不读 config、不读局 KV、不依赖 egg。
# 默认模式：校验 openwiki/overview.md 索引内的相对链接逐一存在 + 必备文件清单存在 + AGENTS.md 行数预算。
# --attest：核验某局事件日志里的 `read.<doc-id>=<ver>` 签名声明与 `openwiki/ver.json`
#           当前的 ver 以及它的历史版本是否一致（`--run <dir>` 必填 —— 脚本不猜「最新局」）。
#
# 用法：
#   wiki-consistency.sh [--repo <dir>] [--index <file>] [--json]
#   wiki-consistency.sh --attest --run <run-dir> [--repo <dir>] [--json] [--max-history <n>]
#   wiki-consistency.sh --help
#
# 参数：
#   --repo <dir>       缺省 = 脚本所在 git toplevel。
#   --index <file>     缺省 = <repo>/openwiki/overview.md。
#   --json             输出单行 JSON（与 k=v 同信息集）；缺省输出 k=v;k=v。
#   --attest           进入签名验真模式（读事件日志，不解索引链接）。
#   --run <run-dir>    --attest 必填：run 目录（含 events.jsonl 或 run.sqlite）。
#   --max-history <n>  每个文件回溯的历史版本上限，缺省 50。
#
# 输出 · 默认（exit 0，k=v）：
#   links=<n>;broken=<k>;first=<relpath|->;ok=<0|1>
#   links  = 索引内相对 markdown 链接总数（(http/https/# 开头的外链与锚点不计）。
#   broken = 链接目标文件不存在 / 必备文件缺失的总数。
#   first  = 第一个 broken 的相对路径（无 = -）。仓库内文档路径，非业务数据。
#   ok     = 1 当且仅当 broken=0。
#
# 输出 · --attest（exit 0，k=v）：
#   claims=<n>;forged=<k>;stale=<s>;first=<doc-id|->;ok=<0|1>
#   claims = 事件日志里出现过的**去重** doc-id 数（同一 id 多次签名取最后一条）。
#   forged = 声明值与磁盘当前 ver 不符，且该值不是任何历史版本的 ver。
#   stale  = 声明值曾是某历史版本的 ver，但当前已变（签名过期）。
#   first  = 第一个出问题的 doc-id（先 forged 后 stale；无 = -）。
#   ok     = 1 当且仅当 claims>0 且 forged=0 且 stale=0（0 声明 = 无证据 → fail-closed）。
#
# 事件源（先后）：<run>/events.jsonl（JSON 后端）；否则 <run>/run.sqlite 的 `step` 表
#   （SQLite 后端，r9 缺省；以 `immutable=1` 只读打开，不建 WAL、不写盘）。
#
# exit 码：0 正常执行（结论在 ok 键）；2 用法错；3 环境缺失（无 python3/git/sqlite3）；
#          4 索引不可读 / run 不可读；5 索引读取异常。
# 「检测发现问题」（broken>0 / forged>0 / stale>0）永远 exit 0 + ok=0。

set -eu

usage() {
  cat <<'EOF'
docs-consistency.sh [--repo <dir>] [--index <file>] [--json]
docs-consistency.sh --attest --run <run-dir> [--repo <dir>] [--json] [--max-history <n>]
  --help             this text
exit: 0 ok (see ok=), 2 usage, 3 env missing, 4 unreadable, 5 index read error
EOF
}

REPO=""
INDEX=""
AS_JSON=0
ATTEST=0
RUN=""
MAXHIST=50

while [ $# -gt 0 ]; do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --repo) REPO="${2-}"; shift 2 ;;
    --index) INDEX="${2-}"; shift 2 ;;
    --json) AS_JSON=1; shift ;;
    --attest) ATTEST=1; shift ;;
    --run) RUN="${2-}"; shift 2 ;;
    --max-history) MAXHIST="${2-}"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; usage >&2; exit 2 ;;
  esac
done

command -v python3 >/dev/null 2>&1 || { echo "python3 required" >&2; exit 3; }

if [ -z "$REPO" ]; then
  command -v git >/dev/null 2>&1 || { echo "git required (or pass --repo)" >&2; exit 3; }
  REPO=$(git rev-parse --show-toplevel 2>/dev/null) || { echo "not in a git repo; pass --repo" >&2; exit 3; }
fi

# ---------------------------------------------------------------- --attest 模式
if [ "$ATTEST" -eq 1 ]; then
  if [ -z "$RUN" ]; then
    echo "--attest requires --run <run-dir>" >&2
    usage >&2
    exit 2
  fi
  [ -d "$RUN" ] || { echo "run dir not readable: $RUN" >&2; exit 4; }

  RESULT=$(python3 - "$REPO" "$RUN" "$MAXHIST" <<'PYEOF'
import json
import os
import re
import shutil
import subprocess
import sys

repo = os.path.abspath(sys.argv[1])
run_dir = os.path.abspath(sys.argv[2])
maxhist = int(sys.argv[3])

ctx = os.path.join(repo, "openwiki", "operating")
LEDGER = os.path.join(repo, "openwiki", "ver.json")
CLAIM = re.compile(r'read\.([A-Za-z0-9._/-]+)=([0-9a-f]{6})')


def fail(code, msg):
    print("ERR%d:%s" % (code, msg))
    sys.exit(0)


def load_events():
    jsonl = os.path.join(run_dir, "events.jsonl")
    if os.path.exists(jsonl):
        chunks = []
        with open(jsonl, "r", encoding="utf-8", errors="replace") as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    chunks.append(json.dumps(json.loads(line), ensure_ascii=False))
                except Exception:
                    chunks.append(line)  # 坏行也扫：宁可多扫，不可漏签
        return chunks, "jsonl"

    db = os.path.join(run_dir, "run.sqlite")
    if os.path.exists(db):
        if shutil.which("sqlite3") is None:
            fail(3, "sqlite3-missing")
        uri = "file:%s?immutable=1" % db  # 只读：不建 WAL、不写盘
        res = subprocess.run(
            ["sqlite3", "-json", uri, "select event, payload from step order by seq"],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if res.returncode != 0:
            fail(4, "sqlite-read:%s" % res.stderr.decode(errors="replace").strip()[:160])
        try:
            rows = json.loads(res.stdout.decode("utf-8") or "[]")
        except Exception as exc:
            fail(4, "sqlite-json:%s" % exc)
        chunks = [(str(r.get("event") or "") + "\n" + str(r.get("payload") or "")) for r in rows]
        return chunks, "sqlite"

    fail(4, "run-unreadable:%s" % run_dir)


def ledger_pages(text):
    """ver.json → {doc-id: ver}；坏 JSON 当作空账本，结论落 forged。"""
    try:
        data = json.loads(text)
    except Exception:
        return {}
    pages = data.get("pages") if isinstance(data, dict) else None
    return {str(k): str(v) for k, v in (pages or {}).items()}


def current_ver(doc_id):
    with open(LEDGER, "rb") as f:
        raw = f.read().decode("utf-8", "surrogateescape")
    return ledger_pages(raw).get(doc_id)


def historical_vers(doc_id):
    """ver.json 的历史版本里该 id 出现过的 ver（签名过期 = 曾是历史值）。"""
    rel = "openwiki/ver.json"
    revs = subprocess.run(["git", "-C", repo, "log", "--format=%H", "--", rel],
                          stdout=subprocess.PIPE, stderr=subprocess.DEVNULL).stdout.decode().split()
    out = set()
    for rev in revs[:maxhist]:
        blob = subprocess.run(["git", "-C", repo, "show", "%s:%s" % (rev, rel)],
                              stdout=subprocess.PIPE, stderr=subprocess.DEVNULL).stdout
        if not blob:
            continue
        value = ledger_pages(blob.decode("utf-8", "surrogateescape")).get(doc_id)
        if value:
            out.add(value)
    return out


def normalize_id(doc_id):
    if doc_id.endswith(".md"):
        doc_id = doc_id[:-3]
    if "/" in doc_id:
        return doc_id
    return doc_id.replace(".", "/")  # read.acts.coordinate → acts/coordinate


chunks, source = load_events()

claims = {}
for chunk in chunks:  # 事件顺序：后出现的覆盖先出现的（同一 id 取最新签名）
    for m in CLAIM.finditer(chunk):
        claims[normalize_id(m.group(1))] = m.group(2)

forged = 0
stale = 0
first = "-"
detail = []

for doc_id, declared in claims.items():
    path = os.path.join(ctx, doc_id + ".md")
    cur = current_ver(doc_id)
    if not os.path.isfile(path) or cur is None:
        forged += 1
        detail.append("forged:%s:missing-file-or-ledger" % doc_id)
        if first == "-":
            first = doc_id
        continue
    if declared == cur:
        continue
    if declared in historical_vers(path):
        stale += 1
        detail.append("stale:%s:%s->%s" % (doc_id, declared, cur))
        if first == "-":
            first = doc_id
    else:
        forged += 1
        detail.append("forged:%s:%s!=%s" % (doc_id, declared, cur))
        if first == "-":
            first = doc_id

ok = 1 if (claims and forged == 0 and stale == 0) else 0
print("%d;%d;%d;%s;%d" % (len(claims), forged, stale, first, ok))
PYEOF
)

  case "$RESULT" in
    ERR3:*) echo "${RESULT#ERR3:}" >&2; exit 3 ;;
    ERR4:*) echo "${RESULT#ERR4:}" >&2; exit 4 ;;
  esac

  CLAIMS=$(printf '%s' "$RESULT" | cut -d';' -f1)
  FORGED=$(printf '%s' "$RESULT" | cut -d';' -f2)
  STALE=$(printf '%s' "$RESULT" | cut -d';' -f3)
  FIRST=$(printf '%s' "$RESULT" | cut -d';' -f4)
  OK=$(printf '%s' "$RESULT" | cut -d';' -f5)

  if [ "$AS_JSON" -eq 1 ]; then
    printf '{"claims":%s,"forged":%s,"stale":%s,"first":"%s","ok":%s}\n' "$CLAIMS" "$FORGED" "$STALE" "$FIRST" "$OK"
  else
    printf 'claims=%s;forged=%s;stale=%s;first=%s;ok=%s\n' "$CLAIMS" "$FORGED" "$STALE" "$FIRST" "$OK"
  fi
  exit 0
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
    'openwiki/operating/scenes.json',
    'openwiki/operating/cold-start/vocabulary.md',
    'openwiki/operating/cold-start/principles.md',
    # r12：决策台账已解散（逐条裁决落进它治理的文档的「设计依据」段），archive/ 不再存在
    # r6：设计契约树的树入口（三模块的枚举由 agent_seed_semantics 的索引集合相等用例负责，
    # 那处才是权威；这里只登记入口，避免两处清单互相追平）
    'openwiki/architecture/README.md',
    # r6：行为叙事归位后的代表文件（理解性，不拦门禁；其余两份由 scenes.json 的 reference 段机检）
    'openwiki/operating/story/requirement-story.md',
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
print("%d;%d;%s;%d" % (checked, broken, first, agents_lines))
PYEOF
)

if [ "${RESULT#ERR:}" != "$RESULT" ]; then
  echo "$RESULT" >&2; exit 5
fi

LINKS=$(printf '%s' "$RESULT" | cut -d';' -f1)
BROKEN=$(printf '%s' "$RESULT" | cut -d';' -f2)
FIRST=$(printf '%s' "$RESULT" | cut -d';' -f3)
AGENTS=$(printf '%s' "$RESULT" | cut -d';' -f4)
AGENTS_BUDGET=160

if [ "$BROKEN" -eq 0 ] && [ "${AGENTS:-0}" -le "$AGENTS_BUDGET" ]; then OK=1; else OK=0; fi

if [ "$AS_JSON" -eq 1 ]; then
  printf '{"links":%s,"broken":%s,"first":"%s","agents":%s,"agents_budget":%s,"ok":%s}\n' \
    "$LINKS" "$BROKEN" "$FIRST" "${AGENTS:-0}" "$AGENTS_BUDGET" "$OK"
else
  printf 'links=%s;broken=%s;first=%s;agents=%s/%s;ok=%s\n' \
    "$LINKS" "$BROKEN" "$FIRST" "${AGENTS:-0}" "$AGENTS_BUDGET" "$OK"
fi
exit 0
