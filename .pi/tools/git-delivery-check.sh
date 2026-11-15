#!/bin/sh
# git-delivery-check.sh — 交付洁净度只读检查（五项：dirty / unpushed / stash / conflict_debris / fmt）。
#
# 纯命令行契约（与 .pi/tools/git-* 亚家族同方言）：不读 config、不读局 KV、不依赖 egg。
# 只读：不改 refs / index / worktree，不 push、不 reset、不 stash、不写盘。
# 默认输出单个 JSON 文档（`ok` / `data` / `error` 三段式）；`--text` 输出人读单行 + 明细行。
#
# 定位：既有四件 git 工具都不检查**执行者交付**——git-alignment-check 比对 develop/master 与
# remote-tracking，git-conflict-check 做 rebase/merge 冲突预检，git-integrate 的 status 检查是
# **协调者合并前**的 preflight，git-release 只做 develop→master。本脚本补的正是这一格：
# 交付物本身能不能被当成「干净交付」受理（改动已提交、已推送、无残留、格式干净）。
#
# 用法：
#   git-delivery-check.sh [--repo <dir>] [--upstream <ref>] [--max-paths <n>]
#                         [--no-fmt] [--no-stash] [--fmt-cmd <cmd>]
#                         [--text] [--json] [--help]
#
# 输出（exit 0，JSON 缺省）：
#   {"ok":<bool>,"data":{"repo":…,"branch":…,"head":…,"detached":…,
#     "checks":{"dirty":{…},"unpushed":{…},"stash":{…},"conflict_debris":{…},"fmt":{…}},
#     "failed":[…],"waived":[…],"truncated":<bool>},"error":null}
#
# 五项（每项都有 `count`，`ok` = 五项 count 全 0）：
#   dirty            `git status --porcelain=v1 -uall` 行数；拆 tracked / untracked / unmerged。
#                    `-uall` 是必须的：缺省 `-unormal` 把整个未跟踪目录折叠成一行，回执数字会骗人。
#   unpushed         `git rev-list --count <upstream>..HEAD`。**无 upstream 记 -1 = 失败**：
#                    0 表示「比过了、一致」，-1 表示「没法比」，两者在回执里必须可分辨。
#                    0 的哨兵同时解释为什么 r3/r4/r5 那种「本地干净、远端没有」能被抓到。
#   stash            `git stash list` 行数。注意 stash 栈是**仓库级共享**的（泳道 worktree 池与主检出
#                    共用），所以 `--no-stash` 豁免必须进 `waived` 并写明理由。
#   conflict_debris  basename 含 `~`、`*.orig`、`*.rej` 的已跟踪/未跟踪文件数。用 `*~*`（含波浪号）
#                    而不是 `*~`（结尾波浪号）：r1 的真实残留是 `copy-contract-core.md~develop`，
#                    波浪号在中间，`*~` 抓不到。`--exclude-standard` 让 `target/` 下的 .orig 不成噪音。
#   fmt              有 `Cargo.toml` 时跑 `cargo fmt --check`（clean→0 / dirty→1 / 命令失败→1+error）；
#                    无 `Cargo.toml` → `n/a`（count 0，不计）；`--no-fmt` → `skipped`（进 `waived`）。
#                    语言无关是本脚本的刻意选择：git-* 亚家族其余四件只认 git，把 cargo 焊死会让
#                    脚本离开本仓就废；但「不存在即静默通过」会造假绿，所以 n/a / skipped 在输出里
#                    显式可见，`--fmt-cmd` 给非 Rust 仓留出口。
#
# 豁免：`--no-fmt` / `--no-stash` 不是静默跳过，两项都写进 `data.waived`——豁免必须留痕。
#
# argv > 环境：`--fmt-cmd` > `$YGG_FMT_CMD` > `cargo fmt --check`；`--repo` > 调用者 cwd 的 git toplevel。
# 本脚本**不 source .env / .env.local**（与 alignment/conflict/integrate/release 的 --develop/--master/
# --origin 三键不同）：它没有任何「仓库级可变配置」，多一层 env 加载只多一个失败面。
#
# exit 码（与 .pi/tools 家族同表）：
#   0 正常执行（结论在 ok 键）；2 用法错；3 环境缺失（无 python3 / 无 git）；
#   4 目标资源不可用（--repo 非 git worktree / --upstream 解析不能）；5 git 命令意外失败。
# 「检测发现问题」永远 exit 0 + ok=false（与 git-alignment-check.sh 同口径），与 malfunction 严格区分。

set -eu

usage() {
  cat <<'EOF'
git-delivery-check.sh [--repo <dir>] [--upstream <ref>] [--max-paths <n>]
                      [--no-fmt] [--no-stash] [--fmt-cmd <cmd>] [--text] [--json]
  --repo <dir>       default: git toplevel of the caller cwd
  --upstream <ref>   default: the branch upstream (@{u}); give it explicitly when the
                     branch has no upstream yet but you want to compare against develop
  --max-paths <n>    cap on stash / conflict_debris detail rows (default: 20)
  --no-fmt           waive the fmt check (recorded in data.waived, never silent)
  --no-stash         waive the stash check (recorded in data.waived, never silent)
  --fmt-cmd <cmd>    default: $YGG_FMT_CMD, else `cargo fmt --check`; run via sh -c in the repo
  --text             human-readable output (default: JSON)
  --json             compatibility no-op; JSON is already the default
  --help             this text
checks (every one carries count; ok = all five counts are 0):
  dirty            git status --porcelain=v1 -uall (tracked / untracked / unmerged)
  unpushed         git rev-list --count <upstream>..HEAD; NO UPSTREAM COUNTS AS -1 = FAILURE
  stash            git stash list (repo-wide: the lane worktree pool shares this stack)
  conflict_debris  basename containing `~`, or *.orig, or *.rej
  fmt              cargo fmt --check when Cargo.toml exists; n/a otherwise; --no-fmt to waive
exit: 0 detection completed (see ok), 2 usage, 3 env missing (python3/git), 4 repo/ref error, 5 git error
EOF
}

AS_TEXT=0
for arg in "$@"; do
  case "$arg" in
    --help|-h) usage; exit 0 ;;
  esac
done

fail() {
  code=$1
  status=$2
  message=$3
  if [ "$AS_TEXT" -eq 1 ]; then
    printf 'error.%s: %s\n' "$code" "$message"
  else
    printf '{"ok":false,"data":null,"error":{"code":"%s","message":"%s"}}\n' "$code" "$message"
  fi
  exit "$status"
}

command -v python3 >/dev/null 2>&1 || fail env_missing 3 "python3 is required"
command -v git >/dev/null 2>&1 || fail env_missing 3 "git is required"

REPO=""
UPSTREAM=""
MAX_PATHS=20
NO_FMT=0
NO_STASH=0
FMT_CMD=${YGG_FMT_CMD:-cargo fmt --check}

while [ $# -gt 0 ]; do
  case "$1" in
    --repo)
      [ $# -ge 2 ] || fail usage 2 "--repo requires a value"
      REPO=$2; shift 2 ;;
    --upstream)
      [ $# -ge 2 ] || fail usage 2 "--upstream requires a value"
      UPSTREAM=$2; shift 2 ;;
    --max-paths)
      [ $# -ge 2 ] || fail usage 2 "--max-paths requires a value"
      MAX_PATHS=$2; shift 2 ;;
    --no-fmt) NO_FMT=1; shift ;;
    --no-stash) NO_STASH=1; shift ;;
    --fmt-cmd)
      [ $# -ge 2 ] || fail usage 2 "--fmt-cmd requires a value"
      FMT_CMD=$2; shift 2 ;;
    --text) AS_TEXT=1; shift ;;
    --json) shift ;;
    *) fail usage 2 "unknown argument: $1" ;;
  esac
done

case "$MAX_PATHS" in
  ''|*[!0-9]*) fail usage 2 "--max-paths must be a positive integer" ;;
esac
[ "$MAX_PATHS" -ge 1 ] || fail usage 2 "--max-paths must be a positive integer"

python3 - "$REPO" "$UPSTREAM" "$MAX_PATHS" "$NO_FMT" "$NO_STASH" "$FMT_CMD" "$AS_TEXT" <<'PYEOF'
import json
import os
import subprocess
import sys

repo_arg, upstream_arg, max_paths, no_fmt, no_stash, fmt_cmd, as_text = sys.argv[1:8]
max_paths = int(max_paths)
no_fmt = no_fmt == "1"
no_stash = no_stash == "1"
as_text = as_text == "1"


def esc(value):
    """Family json_escape parity: ASCII-only, and `|` escaped so pipe-delimited payloads stay safe."""
    return json.dumps(value, ensure_ascii=True).replace("|", "\\u007c")


def emit_error(code, message, status):
    if as_text:
        sys.stdout.write("error.%s: %s\n" % (code, message))
    else:
        sys.stdout.write(
            '{"ok":false,"data":null,"error":{"code":%s,"message":%s}}\n'
            % (esc(code), esc(message))
        )
    sys.exit(status)


# ── repo resolution ───────────────────────────────────────────────────────────
start = repo_arg if repo_arg else os.getcwd()
try:
    resolved = subprocess.run(
        ["git", "-C", start, "rev-parse", "--show-toplevel"],
        capture_output=True,
        text=True,
    )
except OSError as err:  # pragma: no cover - git presence is checked by the caller
    emit_error("env_missing", "cannot exec git: %s" % err, 3)
if resolved.returncode != 0:
    emit_error("repo_invalid", "not in a git worktree; pass --repo", 4)
repo = resolved.stdout.strip()


def git(*args, raw=False):
    return subprocess.run(
        ["git", "-C", repo] + list(args),
        capture_output=True,
        text=not raw,
    )


# ── branch / head ────────────────────────────────────────────────────────────
branch_probe = git("symbolic-ref", "--short", "-q", "HEAD")
branch = branch_probe.stdout.strip() if branch_probe.returncode == 0 else ""
detached = branch == ""
head_probe = git("rev-parse", "HEAD")
head = head_probe.stdout.strip() if head_probe.returncode == 0 else ""

# ── ① dirty ──────────────────────────────────────────────────────────────────
status = git("status", "--porcelain=v1", "-uall")
if status.returncode != 0:
    emit_error("git_error", "git status failed: %s" % status.stderr.strip(), 5)
dirty_lines = [line for line in status.stdout.splitlines() if line]
tracked = sum(1 for line in dirty_lines if not line.startswith("??"))
untracked = sum(1 for line in dirty_lines if line.startswith("??"))
unmerged = 0
for line in dirty_lines:
    xy = line[:2]
    if "U" in xy or xy in ("AA", "DD"):
        unmerged += 1

# ── ② unpushed ──────────────────────────────────────────────────────────────
if upstream_arg:
    upstream = upstream_arg
    if git("rev-parse", "--verify", upstream + "^{commit}").returncode != 0:
        emit_error("upstream_invalid", "cannot resolve --upstream %s" % upstream, 4)
else:
    upstream_probe = git("rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}")
    upstream = upstream_probe.stdout.strip() if upstream_probe.returncode == 0 else ""

if head == "":
    ahead = -1
    unpushed = -1
    hint = "nothing committed yet"
elif upstream == "":
    ahead = -1
    unpushed = -1
    # Detached HEAD cannot have an upstream at all: advice has to differ from a fresh branch.
    hint = "detached HEAD — check out a branch first" if detached else "git push -u origin HEAD"
else:
    counts = git("rev-list", "--count", "%s..HEAD" % upstream)
    if counts.returncode != 0:
        emit_error(
            "git_error",
            "git rev-list failed for %s: %s" % (upstream, counts.stderr.strip()),
            5,
        )
    ahead = int(counts.stdout.strip() or 0)
    unpushed = ahead
    hint = None

# ── ③ stash ─────────────────────────────────────────────────────────────────
if no_stash:
    stash_rows = []
    stash_count = 0
    stash_skipped = True
else:
    stash_probe = git("stash", "list", "--format=%gd %s")
    if stash_probe.returncode != 0:
        emit_error("git_error", "git stash list failed: %s" % stash_probe.stderr.strip(), 5)
    stash_rows = [line for line in stash_probe.stdout.splitlines() if line]
    stash_count = len(stash_rows)
    stash_skipped = False

# ── ④ conflict_debris ───────────────────────────────────────────────────────
tracked_files = git("ls-files", "-z", "--cached", "--others", "--exclude-standard", raw=True)
if tracked_files.returncode != 0:
    emit_error(
        "git_error",
        "git ls-files failed: %s" % tracked_files.stderr.decode("utf-8", "replace").strip(),
        5,
    )
names = [
    os.fsdecode(chunk)
    for chunk in tracked_files.stdout.split(b"\0")
    if chunk
]
debris = [
    name
    for name in names
    if "~" in name or name.endswith(".orig") or name.endswith(".rej")
]

# ── ⑤ fmt ───────────────────────────────────────────────────────────────────
fmt_detail = {"status": None, "count": 0, "command": None}
if no_fmt:
    fmt_detail["status"] = "skipped"
elif os.path.isfile(os.path.join(repo, "Cargo.toml")):
    fmt_detail["command"] = fmt_cmd
    fmt_run = subprocess.run(["sh", "-c", fmt_cmd], cwd=repo, capture_output=True, text=True)
    if fmt_run.returncode == 0:
        fmt_detail["status"] = "clean"
    elif fmt_run.returncode == 1:
        fmt_detail["status"] = "dirty"
        fmt_detail["count"] = 1
    else:
        fmt_detail["status"] = "error"
        fmt_detail["count"] = 1
        fmt_detail["message"] = (fmt_run.stderr or "").strip()[:200]
else:
    fmt_detail["status"] = "n/a"

checks = {
    "dirty": {
        "count": len(dirty_lines),
        "tracked": tracked,
        "untracked": untracked,
        "unmerged": unmerged,
    },
    "unpushed": {
        "count": unpushed,
        "ahead": ahead,
        "upstream": upstream or None,
        "hint": hint,
    },
    "stash": {
        "count": stash_count,
        "list": stash_rows[:max_paths],
        "skipped": stash_skipped,
    },
    "conflict_debris": {
        "count": len(debris),
        "paths": debris[:max_paths],
        "truncated": len(debris) > max_paths,
    },
    "fmt": fmt_detail,
}

# ── verdict ─────────────────────────────────────────────────────────────────
order = ["dirty", "unpushed", "stash", "conflict_debris", "fmt"]
failed = [name for name in order if checks[name]["count"] != 0]
waived = []
if no_fmt:
    waived.append("fmt")
if no_stash:
    waived.append("stash")
truncated = checks["conflict_debris"]["truncated"] or stash_count > max_paths
ok = not failed

data = {
    "repo": repo,
    "branch": branch or None,
    "head": head or None,
    "detached": detached,
    "checks": checks,
    "failed": failed,
    "waived": waived,
    "truncated": truncated,
}

if as_text:
    out = [
        "repo=%s branch=%s head=%s upstream=%s"
        % (repo, branch or "null", (head[:7] if head else "null"), upstream or "null")
    ]
    out.append(
        "dirty=%d unpushed=%d stash=%d debris=%d fmt=%s ok=%d failed=[%s]"
        % (
            checks["dirty"]["count"],
            checks["unpushed"]["count"],
            checks["stash"]["count"],
            checks["conflict_debris"]["count"],
            checks["fmt"]["status"],
            1 if ok else 0,
            ",".join(failed),
        )
    )
    if waived:
        out.append("waived=[%s]" % ",".join(waived))
    if hint:
        out.append("hint=%s" % hint)
    if fmt_detail.get("message"):
        out.append("fmt.message=%s" % fmt_detail["message"])
    for name in checks["conflict_debris"]["paths"]:
        out.append("debris=%s" % name)
    for name in checks["stash"]["list"]:
        out.append("stash=%s" % name)
    sys.stdout.write("\n".join(out) + "\n")
else:
    sys.stdout.write(
        '{"ok":%s,"data":%s,"error":null}\n'
        % (
            esc(ok),
            json.dumps(
                data, ensure_ascii=True, sort_keys=False, separators=(",", ":")
            ).replace("|", "\\u007c"),
        )
    )

sys.exit(0)
PYEOF
