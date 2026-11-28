#!/bin/sh
# wiki-ver.sh —— openwiki 页面的 ver 账本生成 / 验真 / 链接闭包（只读或就地刷新）。
#
# 纯命令行契约（同仓 scripts/* 风格）：不读局 KV、不依赖运行时状态。
#
# ver 定义：页面正文（整文件字节）的 git blob 哈希取前 6 位十六进制。页面本身**不写 ver**
# ——一个 ver 只有一个落点，写进页面就会出现两个。账本 = openwiki/ver.json。
#
# 用法：
#   scripts/wiki-ver.sh [--check] [--links] [--json] [--root <dir>] [--help]
#
# 参数：
#   --check   只报告不写：比对账本与计算 ver，并做链接闭包 + id 闭包；只报告不写盘
#             （可重复执行，第二次必须 changed=0）。
#   --links   只做链接闭包（--check 已隐含）。
#   --json    输出单行 JSON（与 k=v 同信息集）。
#   --root    仓库根；缺省 = 脚本所在目录的 git toplevel。
#
# 输出（exit 0，k=v 缺省）：
#   files=<n>;changed=<k>;missing_id=<m>;dup_id=<d>;missing_ver=<v>;broken_links=<b>;first=<relpath|->;ok=<0|1>
#   files          openwiki/**/*.md 的真文件数
#   changed        账本与计算 ver 不一致（--check 即「需要刷新」）
#   missing_id     页头缺 `> **id** <id>` 的文件数
#   dup_id         两个页面声明了同一个 id 的次数（id 必须唯一，否则账本互相覆盖）
#   missing_ver    页面 id 在账本里没有条目的文件数
#   broken_links   树内相对 markdown 链接指向不存在目标的总数
#   first          第一个问题文件（优先级 missing_id > missing_ver > changed > broken_links；无 = -）
#   ok             1 当且仅当 changed=missing_id=missing_ver=broken_links=0
#
# 退出码：0 正常执行（结论在 ok 键）；2 用法错；3 环境缺失（git/python3）；4 目录不可读或无 md。
# 「检测发现问题」永远 exit 0 + ok=0（与同仓 scripts/* 同口径）。

set -eu

usage() {
  cat <<'EOF'
scripts/wiki-ver.sh [--check] [--links] [--json] [--root <dir>]
  --check   report only (ver drift + link closure + id closure)
  --links   link closure only
  --json    emit single-line JSON
  --root    repo root (default: git toplevel of the script location)
  --help    this text
output (k=v): files=..;changed=..;missing_id=..;missing_ver=..;broken_links=..;first=..;ok=0|1
exit: 0 ok (see ok=), 2 usage, 3 env missing, 4 nothing to scan
EOF
}

CHECK=0
LINKS_ONLY=0
AS_JSON=0
ROOT=""

while [ $# -gt 0 ]; do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --check) CHECK=1; shift ;;
    --links) LINKS_ONLY=1; CHECK=1; shift ;;
    --json) AS_JSON=1; shift ;;
    --root) ROOT="${2-}"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; usage >&2; exit 2 ;;
  esac
done

command -v git >/dev/null 2>&1 || { echo "env: git not found" >&2; exit 3; }
command -v python3 >/dev/null 2>&1 || { echo "env: python3 not found" >&2; exit 3; }

if [ -z "$ROOT" ]; then
  ROOT=$(cd "$(dirname "$0")/.." && pwd)
  if git -C "$ROOT" rev-parse --show-toplevel >/dev/null 2>&1; then
    ROOT=$(git -C "$ROOT" rev-parse --show-toplevel)
  fi
fi
[ -d "$ROOT/openwiki" ] || { echo "env: no openwiki/ under $ROOT" >&2; exit 4; }

CHECK="$CHECK" LINKS_ONLY="$LINKS_ONLY" AS_JSON="$AS_JSON" ROOT="$ROOT" python3 - <<'PY'
import json, os, re, subprocess, sys
from collections import OrderedDict

ROOT = os.environ["ROOT"]
CHECK = os.environ["CHECK"] == "1"
LINKS_ONLY = os.environ["LINKS_ONLY"] == "1"
AS_JSON = os.environ["AS_JSON"] == "1"
WIKI = os.path.join(ROOT, "openwiki")
LEDGER = os.path.join(WIKI, "ver.json")

ID_LINE = re.compile(r"^> \*\*id\*\*\s+(\S+)", re.M)
LINK = re.compile(r"\[[^\]]*\]\(([^)\s]+)\)")
SCHEMES = ("http://", "https://", "mailto:", "#")


def fail(code, msg):
    sys.stderr.write(msg + "\n")
    sys.exit(code)


def pages():
    out = []
    for base, dirs, files in os.walk(WIKI):
        dirs[:] = [d for d in dirs if not d.startswith(".")]
        for name in sorted(files):
            if not name.endswith(".md") or name == "index.md":
                continue  # index.md 是 OpenWiki 生成的目录页，不是作者页
            path = os.path.join(base, name)
            if os.path.islink(path):
                continue
            out.append((os.path.relpath(path, ROOT), path))
    return sorted(out)


def header_id(text):
    m = ID_LINE.search(text)
    return m.group(1) if m else None


def git_ver(path):
    out = subprocess.run(["git", "-C", ROOT, "hash-object", path],
                         capture_output=True, text=True)
    if out.returncode != 0:
        fail(3, "git hash-object failed: " + out.stderr.strip())
    return out.stdout.strip()[:6]


def load_ledger():
    if not os.path.exists(LEDGER):
        return {}
    with open(LEDGER, encoding="utf-8") as f:
        data = json.load(f)
    return data.get("pages", {})


def link_closure(page_list):
    broken = 0
    first = None
    for rel, path in page_list:
        with open(path, encoding="utf-8") as f:
            text = f.read()
        for target in LINK.findall(text):
            if target.startswith(SCHEMES):
                continue
            target = target.split("#", 1)[0]
            if not target:
                continue
            dest = os.path.normpath(os.path.join(os.path.dirname(path), target))
            if not os.path.exists(dest):
                broken += 1
                first = first or rel
    return broken, first


page_list = pages()
if not page_list:
    fail(4, "no markdown pages under openwiki/")

ledger = load_ledger()
computed = OrderedDict()
missing_id = 0
dup_id = 0
missing_ver = 0
changed = 0
first_id = first_ver = first_changed = None

for rel, path in page_list:
    with open(path, encoding="utf-8") as f:
        text = f.read()
    doc_id = header_id(text)
    if not doc_id:
        missing_id += 1
        first_id = first_id or rel
        continue
    if doc_id in computed:
        dup_id += 1
        first_id = first_id or rel
        continue
    ver = git_ver(path)
    computed[doc_id] = ver
    if doc_id not in ledger:
        missing_ver += 1
        first_ver = first_ver or rel
    elif ledger[doc_id] != ver:
        changed += 1
        first_changed = first_changed or rel

broken, first_broken = link_closure(page_list)

ok = (changed == 0 and missing_id == 0 and dup_id == 0 and missing_ver == 0 and broken == 0)
first = first_id or first_ver or first_changed or first_broken or "-"

if CHECK and not LINKS_ONLY:
    pass
elif not CHECK:
    payload = OrderedDict((
        ("generated_by", "scripts/wiki-ver.sh"),
        ("note", "ver = first 6 hex of the page blob hash; pages carry no ver line"),
        ("pages", computed),
    ))
    tmp = LEDGER + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(payload, f, ensure_ascii=False, indent=2, sort_keys=True)
        f.write("\n")
    os.replace(tmp, LEDGER)

result = OrderedDict((
    ("files", len(page_list)),
    ("changed", changed),
    ("missing_id", missing_id),
    ("dup_id", dup_id),
    ("missing_ver", missing_ver),
    ("broken_links", broken),
    ("first", first),
    ("ok", 1 if ok else 0),
))

if AS_JSON:
    print(json.dumps(result, ensure_ascii=False))
else:
    print(";".join(f"{k}={v}" for k, v in result.items()))

sys.exit(0)
PY
