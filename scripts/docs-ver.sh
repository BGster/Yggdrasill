#!/bin/sh
# docs-ver.sh —— docs/context 召回树的 ver 生成 / 验真 / 链接闭包（只读或就地刷新）。
#
# 纯命令行契约（同 .pi/tools 风格）：不读 config、不读局 KV、不依赖 egg。
#
# ver 定义（ver 与兼容层）：把固定头里 `> **ver** <6 位十六进制>` 的 token 换成定宽占位 `000000`
# （字节数不变），其余字节原样，再 `git hash-object` 取前 6 位。
# 直接对文件本体哈希会自引用循环（ver 变 → 哈希变 → ver 变），故必须先归一化。
#
# 用法：
#   scripts/docs-ver.sh [--check] [--links] [--json] [--root <dir>] [--help]
#
# 参数：
#   --check   只报告不写：逐文件比对声明 ver 与计算 ver，并做链接闭包 + scenes.json 一致性；
#             只报告不写盘（可重复执行，第二次必须 changed=0）。
#   --links   只做链接闭包（--check 已隐含链接闭包）。
#   --json    输出单行 JSON（与 k=v 同信息集）。
#   --root    仓库根；缺省 = 脚本所在目录的 git toplevel。
#
# 输出（exit 0，k=v 缺省）：
#   files=<n>;changed=<k>;missing_header=<m>;broken_links=<b>;first=<relpath|->;ok=<0|1>
#   files           docs/context/**/*.md 的真文件数（软链不计数：兼容层不是权威内容）
#   changed         声明 ver 与计算 ver 不一致的文件数（--check 时即「需要刷新」）
#   missing_header  找不到 `> **ver** <hex6>` 固定头行的文件数
#   broken_links    树内相对 markdown 链接指向不存在目标的总数
#   first           第一个问题文件的相对路径（优先级 missing_header > changed > broken_links；无 = -）
#   ok              1 当且仅当 changed=missing_header=broken_links=0
#
# 退出码：0 正常执行（结论在 ok 键）；2 用法错；3 环境缺失（git/python3）；4 目录不可读或无 md。
# 「检测发现问题」永远 exit 0 + ok=0（与 .pi/tools/* 同口径）。

set -eu

usage() {
  cat <<'EOF'
scripts/docs-ver.sh [--check] [--links] [--json] [--root <dir>]
  --check   report only (ver drift + link closure + scenes.json consistency)
  --links   link closure only
  --json    emit single-line JSON
  --root    repo root (default: git toplevel of the script location)
  --help    this text
output (k=v): files=..;changed=..;missing_header=..;broken_links=..;first=..;ok=0|1
exit: 0 ok (see ok=), 2 usage, 3 env missing, 4 nothing to scan
EOF
}

CHECK=0
LINKS=0
AS_JSON=0
ROOT=""

while [ $# -gt 0 ]; do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --check) CHECK=1; shift ;;
    --links) LINKS=1; shift ;;
    --json) AS_JSON=1; shift ;;
    --root) ROOT="${2-}"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; usage >&2; exit 2 ;;
  esac
done

command -v python3 >/dev/null 2>&1 || { echo "python3 required" >&2; exit 3; }

if [ -z "$ROOT" ]; then
  command -v git >/dev/null 2>&1 || { echo "git required (or pass --root)" >&2; exit 3; }
  ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || { echo "not in a git repo; pass --root" >&2; exit 3; }
fi

[ -d "$ROOT/docs/context" ] || { echo "docs/context not found under $ROOT" >&2; exit 4; }

RESULT=$(python3 - "$ROOT" "$CHECK" "$LINKS" "$AS_JSON" <<'PYEOF'
import json
import os
import re
import subprocess
import sys

root = os.path.abspath(sys.argv[1])
check_only = sys.argv[2] == "1"
links_only = sys.argv[3] == "1"
as_json = sys.argv[4] == "1"

ctx = os.path.join(root, "docs", "context")
VER_LINE = re.compile(r'^> \*\*ver\*\* ([0-9a-f]{6})$', re.M)
ID_LINE = re.compile(r'^> \*\*id\*\* ([^ ·]+)', re.M)
PLACEHOLDER = "000000"


def git_blob_hash(data):
    out = subprocess.run(["git", "hash-object", "--stdin"], input=data,
                         stdout=subprocess.PIPE, check=True)
    return out.stdout.decode().strip()


def real_files():
    found = []
    for dirpath, dirnames, filenames in os.walk(ctx):
        dirnames.sort()
        for name in sorted(filenames):
            if not name.endswith(".md"):
                continue
            p = os.path.join(dirpath, name)
            if os.path.islink(p) or not os.path.isfile(p):
                continue  # 兼容层（软链）不是权威内容，不参与 ver 账
            found.append(p)
    return sorted(found)


def normalize(raw):
    """ver token → 定宽占位（字节数不变），其余原样。"""
    text = raw.decode("utf-8", "surrogateescape")
    norm = VER_LINE.sub(lambda m: "> **ver** " + PLACEHOLDER, text)
    return norm.encode("utf-8", "surrogateescape")


def computed_ver(path):
    with open(path, "rb") as f:
        raw = f.read()
    return git_blob_hash(normalize(raw))[:6]


def declared_ver(path):
    with open(path, "r", encoding="utf-8", errors="replace") as f:
        m = VER_LINE.search(f.read())
    return m.group(1) if m else None


def refresh(path, new):
    with open(path, "rb") as f:
        text = f.read().decode("utf-8", "surrogateescape")
    text = VER_LINE.sub("> **ver** " + new, text, count=1)
    with open(path, "wb") as f:
        f.write(text.encode("utf-8", "surrogateescape"))


files = real_files()
if not files:
    print("ERR:no-md-under:%s" % os.path.join("docs", "context"))
    sys.exit(0)

changed = 0
missing = 0
first = "-"
detail = []

for p in files:
    rel = os.path.relpath(p, ctx)
    dec = declared_ver(p)
    if dec is None:
        missing += 1
        detail.append("missing_header:%s" % rel)
        if first == "-":
            first = rel
        continue
    cur = computed_ver(p)
    if cur == dec:
        continue
    if not check_only:
        refresh(p, cur)
    changed += 1
    detail.append("%s:%s->%s" % ("drift" if check_only else "refreshed", rel, cur))
    if first == "-" and missing == 0:
        first = rel

# 链接闭包：树内相对 markdown 链接逐条核存在性（软链穿透）
broken = 0
link_re = re.compile(r'\[[^\]]*\]\(([^)]+)\)')
if check_only or links_only:
    for p in files:
        rel = os.path.relpath(p, ctx)
        with open(p, "r", encoding="utf-8", errors="replace") as f:
            text = f.read()
        for m in link_re.finditer(text):
            target = m.group(1).strip().split()[0] if m.group(1).strip() else ""
            if not target or target.startswith(("http://", "https://", "mailto:", "#")):
                continue
            target = target.split("#")[0]
            if not target:
                continue
            fp = os.path.normpath(os.path.join(os.path.dirname(p), target))
            if not os.path.exists(fp):
                broken += 1
                detail.append("broken_link:%s->%s" % (rel, target))
                if first == "-":
                    first = rel

# scenes.json 一致性：表里每个 id 必须能解析到文件，且文件头 id 与表中值逐字相等
scenes_path = os.path.join(ctx, "scenes.json")
if os.path.exists(scenes_path):
    with open(scenes_path, "r", encoding="utf-8") as f:
        try:
            scenes = json.load(f)
        except Exception as exc:  # 表坏掉本身就是缺陷
            broken += 1
            detail.append("bad_scenes_json:%s" % exc)
            if first == "-":
                first = "scenes.json"
            scenes = {}

    def resolve(doc_id):
        p = os.path.join(ctx, doc_id + ".md")
        if not os.path.isfile(p):
            return None, "missing"
        with open(p, "r", encoding="utf-8", errors="replace") as f:
            m = ID_LINE.search(f.read())
        if not m:
            return p, "no_id_header"
        if m.group(1) != doc_id:
            return p, "id_mismatch:%s" % m.group(1)
        return p, None

    ids = []
    for lst in (scenes.get("scene") or {}).values():
        ids.extend(lst if isinstance(lst, list) else [lst])
    for v in (scenes.get("reqStage") or {}).values():
        ids.extend(v if isinstance(v, list) else [v])
    for doc_id in ids:
        _, problem = resolve(doc_id)
        if problem:
            broken += 1
            detail.append("scenes:%s:%s" % (doc_id, problem))
            if first == "-":
                first = "scenes.json"

ok = 1 if (changed == 0 and missing == 0 and broken == 0) else 0

if as_json:
    print(json.dumps({"files": len(files), "changed": changed,
                      "missing_header": missing, "broken_links": broken,
                      "first": first, "ok": ok}, ensure_ascii=False))
else:
    print("files=%d;changed=%d;missing_header=%d;broken_links=%d;first=%s;ok=%d"
          % (len(files), changed, missing, broken, first, ok))
PYEOF
)

if [ "${RESULT#ERR:}" != "$RESULT" ]; then
  echo "$RESULT" >&2
  exit 4
fi

printf '%s\n' "$RESULT"
exit 0
