#!/bin/sh
# manifest-verify.sh —— 单源 Mod 完整性核验（只读）。
#
# 纯命令行契约（rev3 / harness 机证）：不读 config、不读局 KV、不依赖 egg。
# 自实现只读比对：逐条读 manifest.json，对声明的每个文件做 sha256 + 字节数比对。
# 不包装 sync-seed-mod.sh --check（那会临时改写 manifest 且需 cargo）。
#
# 用法：
#   manifest-verify.sh [--repo <dir>] [--mod <dir>] [--packages <dir>] [--json]
#   manifest-verify.sh --help
#
# 参数：
#   --repo <dir>     缺省 = 脚本所在 git toplevel（git rev-parse --show-toplevel）。
#   --mod <dir>      缺省 = <repo>/.ygg/mods/agent-seed。
#   --packages <dir> 包根；缺省 = <repo>/packages/libs（manifest `packages` 段的 path
#                    相对包根且以包 id 打头）。
#   --json           输出单行 JSON（与 k=v 同信息集）；缺省输出 k=v;k=v。
#
# 输出（exit 0，k=v 缺省）：
#   files=<n>;drift=<k>;first=<relpath|->;pkg_files=<n>;pkg_drift=<k>;pkg_first=<relpath|->;ok=<0|1>
#   files / drift / first = manifest `code` 数组面（模块源码）。
#   pkg_files / pkg_drift / pkg_first = manifest `packages` 段面（包文件，路径相对包根）。
#   ok    = 1 当且仅当 drift=0 且 pkg_drift=0。
#
# 本脚本**只做只读比对，不携带任何规则**（AGENTS.md §1.7）：哪些文件该入册、
# 包怎么解析，全在 `cargo run --example pack` 的单一实现里；这里只回答
# 「manifest 说的字节还在不在」。
#
# exit 码：0 正常执行（结论在 ok 键）；2 用法错；3 环境缺失（无 python3/git）；4 manifest 不可读。
# 「检测发现问题」（drift>0）永远 exit 0 + ok=0。

set -eu

usage() {
  cat <<'EOF'
manifest-verify.sh [--repo <dir>] [--mod <dir>] [--packages <dir>] [--json]
  --repo <dir>     default: git toplevel of the script location
  --mod <dir>      default: <repo>/.ygg/mods/agent-seed
  --packages <dir> default: <repo>/packages/libs
  --json           emit single-line JSON instead of k=v;k=v
  --help           this text
exit: 0 ok (see ok=), 2 usage, 3 env missing (python3/git), 4 manifest unreadable
EOF
}

REPO=""
MOD=""
PACKAGES=""
AS_JSON=0

while [ $# -gt 0 ]; do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --repo) REPO="${2-}"; shift 2 ;;
    --mod) MOD="${2-}"; shift 2 ;;
    --packages) PACKAGES="${2-}"; shift 2 ;;
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
[ -n "$PACKAGES" ] || PACKAGES="$REPO/packages/libs"

MANIFEST="$MOD/manifest.json"
[ -r "$MANIFEST" ] || { echo "manifest not readable: $MANIFEST" >&2; exit 4; }

RESULT=$(python3 - "$MANIFEST" "$MOD" "$PACKAGES" <<'PYEOF'
import sys, json, hashlib, os
manifest_path, mod_dir, pkg_root = sys.argv[1], sys.argv[2], sys.argv[3]
try:
    data = json.load(open(manifest_path, 'r', encoding='utf-8'))
except Exception as e:
    print("ERR:manifest-parse:%s" % e); sys.exit(0)


def drift_of(items, base_dir, path_key):
    """逐条比对 (path, sha256, byteLength)；返回 (总数, drift 数, 首个 drift 路径)。"""
    total = len(items)
    drift = 0
    first = "-"
    for e in items:
        rel = e.get(path_key, '')
        want_sha = e.get('sha256', '')
        want_len = e.get('byteLength', -1)
        fp = os.path.join(base_dir, rel)
        bad = False
        if not os.path.isfile(fp):
            bad = True
        else:
            try:
                raw = open(fp, 'rb').read()
                if hashlib.sha256(raw).hexdigest() != want_sha or len(raw) != want_len:
                    bad = True
            except Exception:
                bad = True
        if bad:
            drift += 1
            if first == "-":
                first = rel
    return total, drift, first


files, drift, first = drift_of(data.get('code', []), mod_dir, 'path')

# packages 段：path 相对**包根**且以包 id 打头，所以拍平成 [(id/path)] 一次比完。
pkg_files = []
for pkg in data.get('packages', []):
    pkg_files.extend(pkg.get('files', []))
total_pkg, pkg_drift, pkg_first = drift_of(pkg_files, pkg_root, 'path')

print("%d;%d;%s;%d;%d;%s" % (files, drift, first, total_pkg, pkg_drift, pkg_first))
PYEOF
)

if [ "${RESULT#ERR:}" != "$RESULT" ]; then
  echo "$RESULT" >&2; exit 5
fi

FILES=$(printf '%s' "$RESULT" | cut -d';' -f1)
DRIFT=$(printf '%s' "$RESULT" | cut -d';' -f2)
FIRST=$(printf '%s' "$RESULT" | cut -d';' -f3)
PKG_FILES=$(printf '%s' "$RESULT" | cut -d';' -f4)
PKG_DRIFT=$(printf '%s' "$RESULT" | cut -d';' -f5)
PKG_FIRST=$(printf '%s' "$RESULT" | cut -d';' -f6-)

if [ "$DRIFT" -eq 0 ] && [ "$PKG_DRIFT" -eq 0 ]; then OK=1; else OK=0; fi

if [ "$AS_JSON" -eq 1 ]; then
  printf '{"files":%s,"drift":%s,"first":"%s","pkg_files":%s,"pkg_drift":%s,"pkg_first":"%s","ok":%s}\n' \
    "$FILES" "$DRIFT" "$FIRST" "$PKG_FILES" "$PKG_DRIFT" "$PKG_FIRST" "$OK"
else
  printf 'files=%s;drift=%s;first=%s;pkg_files=%s;pkg_drift=%s;pkg_first=%s;ok=%s\n' \
    "$FILES" "$DRIFT" "$FIRST" "$PKG_FILES" "$PKG_DRIFT" "$PKG_FIRST" "$OK"
fi
exit 0
