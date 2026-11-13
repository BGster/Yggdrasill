#!/bin/sh
# manifest-verify.sh —— 单源 Mod 完整性核验（只读）。
#
# 纯命令行契约（rev3 / D-MA26）：不读 config、不读局 KV、不依赖 egg。
# 自实现只读比对：逐条读 manifest.json，对声明的每个文件做 sha256 + 字节数比对。
# 不包装 sync-seed-mod.sh --check（那会临时改写 manifest 且需 cargo）。
#
# 用法：
#   manifest-verify.sh [--repo <dir>] [--mod <dir>] [--json]
#   manifest-verify.sh --help
#
# 参数：
#   --repo <dir>   缺省 = 脚本所在 git toplevel（git rev-parse --show-toplevel）。
#   --mod <dir>    缺省 = <repo>/.ygg/mods/agent-seed。
#   --json         输出单行 JSON（与 k=v 同信息集）；缺省输出 k=v;k=v。
#
# 输出（exit 0，k=v 缺省）：
#   files=<n>;drift=<k>;first=<relpath|->;ok=<0|1>
#   files = manifest 声明的文件总数。
#   drift = sha256 或字节数不一致 / 文件缺失的条目数。
#   first = 第一个 drift 的相对路径（无 drift = -）。路径内容属仓库内源码路径，非业务数据。
#   ok    = 1 当且仅当 drift=0。
#
# exit 码：0 正常执行（结论在 ok 键）；2 用法错；3 环境缺失（无 python3/git）；4 manifest 不可读。
# 「检测发现问题」（drift>0）永远 exit 0 + ok=0。

set -eu

usage() {
  cat <<'EOF'
manifest-verify.sh [--repo <dir>] [--mod <dir>] [--json]
  --repo <dir>  default: git toplevel of the script location
  --mod <dir>   default: <repo>/.ygg/mods/agent-seed
  --json        emit single-line JSON instead of k=v;k=v
  --help        this text
exit: 0 ok (see ok=), 2 usage, 3 env missing (python3/git), 4 manifest unreadable
EOF
}

REPO=""
MOD=""
AS_JSON=0

while [ $# -gt 0 ]; do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --repo) REPO="${2-}"; shift 2 ;;
    --mod) MOD="${2-}"; shift 2 ;;
    --json) AS_JSON=1; shift ;;
    *) echo "unknown arg: $1" >&2; usage >&2; exit 2 ;;
  esac
done

command -v python3 >/dev/null 2>&1 || { echo "python3 required" >&2; exit 3; }

if [ -z "$REPO" ]; then
  command -v git >/dev/null 2>&1 || { echo "git required (or pass --repo)" >&2; exit 3; }
  REPO=$(git rev-parse --show-toplevel 2>/dev/null) || { echo "not in a git repo; pass --repo" >&2; exit 3; }
fi
[ -n "$MOD" ] || MOD="$REPO/.ygg/mods/agent-seed"

MANIFEST="$MOD/manifest.json"
[ -r "$MANIFEST" ] || { echo "manifest not readable: $MANIFEST" >&2; exit 4; }

RESULT=$(python3 - "$MANIFEST" "$MOD" <<'PYEOF'
import sys, json, hashlib, os
manifest_path, mod_dir = sys.argv[1], sys.argv[2]
try:
    data = json.load(open(manifest_path, 'r', encoding='utf-8'))
except Exception as e:
    print("ERR:manifest-parse:%s" % e); sys.exit(0)

entries = data.get('code', [])
total = len(entries)
drift = 0
first = "-"
for e in entries:
    rel = e.get('path', '')
    want_sha = e.get('sha256', '')
    want_len = e.get('byteLength', -1)
    fp = os.path.join(mod_dir, rel)
    bad = False
    if not os.path.isfile(fp):
        bad = True
    else:
        try:
            raw = open(fp, 'rb').read()
            got_sha = hashlib.sha256(raw).hexdigest()
            got_len = len(raw)
            if got_sha != want_sha or got_len != want_len:
                bad = True
        except Exception:
            bad = True
    if bad:
        drift += 1
        if first == "-":
            first = rel
print("%d;%d;%s" % (total, drift, first))
PYEOF
)

if [ "${RESULT#ERR:}" != "$RESULT" ]; then
  echo "$RESULT" >&2; exit 5
fi

FILES=$(printf '%s' "$RESULT" | cut -d';' -f1)
DRIFT=$(printf '%s' "$RESULT" | cut -d';' -f2)
FIRST=$(printf '%s' "$RESULT" | cut -d';' -f3-)

if [ "$DRIFT" -eq 0 ]; then OK=1; else OK=0; fi

if [ "$AS_JSON" -eq 1 ]; then
  printf '{"files":%s,"drift":%s,"first":"%s","ok":%s}\n' "$FILES" "$DRIFT" "$FIRST" "$OK"
else
  printf 'files=%s;drift=%s;first=%s;ok=%s\n' "$FILES" "$DRIFT" "$FIRST" "$OK"
fi
exit 0
