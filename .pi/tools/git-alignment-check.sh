#!/bin/sh
# git-alignment-check.sh — develop/master 与当前 remote-tracking 快照对齐检测（只读）。
# 不 fetch，不改 refs/index/worktree；默认输出单个 JSON 文档。

set -eu

usage() {
  cat <<'EOF'
git-alignment-check.sh [--repo <dir>] [--develop <branch>] [--master <branch>] [--origin <name>] [--text] [--json]
  --repo <dir>       default: git toplevel of the caller cwd
  --develop <branch> argv > $YGG_DEVELOP_BRANCH > <repo>/.env.local > <repo>/.env
  --master <branch>  argv > $YGG_MASTER_BRANCH > <repo>/.env.local > <repo>/.env
  --origin <name>    argv > $YGG_ORIGIN > <repo>/.env.local > <repo>/.env
  --text             human-readable output (default: JSON)
  --json             compatibility no-op; JSON is already the default
  --help             this text
exit: 0 detection completed (see ok), 2 usage, 3 env missing, 4 repo/ref error, 5 git error
EOF
}

AS_TEXT=0
for arg in "$@"; do
  case "$arg" in
    --help|-h) usage; exit 0 ;;
  esac
done

if ! command -v python3 >/dev/null 2>&1; then
  printf '%s\n' '{"ok":false,"data":null,"error":{"code":"env_missing","message":"python3 is required"}}'
  exit 3
fi
if ! command -v git >/dev/null 2>&1; then
  printf '%s\n' '{"ok":false,"data":null,"error":{"code":"env_missing","message":"git is required"}}'
  exit 3
fi

json_escape() {
  python3 -c 'import json,sys; print(json.dumps(sys.argv[1], ensure_ascii=True).replace("|", "\\u007c"))' "$1"
}

fail() {
  code=$1
  status=$2
  message=$3
  if [ "$AS_TEXT" -eq 1 ]; then
    printf 'error.%s: %s\n' "$code" "$message"
  else
    printf '{"ok":false,"data":null,"error":{"code":%s,"message":%s}}\n' \
      "$(json_escape "$code")" "$(json_escape "$message")"
  fi
  exit "$status"
}

REPO=""
ARG_DEVELOP=""
ARG_MASTER=""
ARG_ORIGIN=""
while [ $# -gt 0 ]; do
  case "$1" in
    --repo) [ $# -ge 2 ] || fail usage 2 "--repo requires a value"; REPO=$2; shift 2 ;;
    --develop) [ $# -ge 2 ] || fail usage 2 "--develop requires a value"; ARG_DEVELOP=$2; shift 2 ;;
    --master) [ $# -ge 2 ] || fail usage 2 "--master requires a value"; ARG_MASTER=$2; shift 2 ;;
    --origin) [ $# -ge 2 ] || fail usage 2 "--origin requires a value"; ARG_ORIGIN=$2; shift 2 ;;
    --text) AS_TEXT=1; shift ;;
    --json) shift ;;
    *) fail usage 2 "unknown argument: $1" ;;
  esac
done

if [ -z "$REPO" ]; then
  if ! REPO=$(git rev-parse --show-toplevel 2>/dev/null); then
    fail repo_invalid 4 "not in a git worktree; pass --repo"
  fi
else
  if ! REPO=$(git -C "$REPO" rev-parse --show-toplevel 2>/dev/null); then
    fail repo_invalid 4 "--repo is not a git worktree"
  fi
fi

PROCESS_DEVELOP=${YGG_DEVELOP_BRANCH-}
PROCESS_MASTER=${YGG_MASTER_BRANCH-}
PROCESS_ORIGIN=${YGG_ORIGIN-}
ENV_STATUS=0
set +u
set +e
if [ -f "$REPO/.env" ]; then
  . "$REPO/.env" || ENV_STATUS=$?
fi
if [ "$ENV_STATUS" -eq 0 ] && [ -f "$REPO/.env.local" ]; then
  . "$REPO/.env.local" || ENV_STATUS=$?
fi
set -e
set -u
[ "$ENV_STATUS" -eq 0 ] || fail env_invalid 3 "failed to load repository .env/.env.local"

if [ -n "$PROCESS_DEVELOP" ]; then DEVELOP=$PROCESS_DEVELOP; else DEVELOP=${YGG_DEVELOP_BRANCH-}; fi
if [ -n "$PROCESS_MASTER" ]; then MASTER=$PROCESS_MASTER; else MASTER=${YGG_MASTER_BRANCH-}; fi
if [ -n "$PROCESS_ORIGIN" ]; then ORIGIN=$PROCESS_ORIGIN; else ORIGIN=${YGG_ORIGIN-}; fi
[ -z "$ARG_DEVELOP" ] || DEVELOP=$ARG_DEVELOP
[ -z "$ARG_MASTER" ] || MASTER=$ARG_MASTER
[ -z "$ARG_ORIGIN" ] || ORIGIN=$ARG_ORIGIN

[ -n "$DEVELOP" ] || fail env_missing 3 "YGG_DEVELOP_BRANCH is required (or pass --develop)"
[ -n "$MASTER" ] || fail env_missing 3 "YGG_MASTER_BRANCH is required (or pass --master)"
[ -n "$ORIGIN" ] || fail env_missing 3 "YGG_ORIGIN is required (or pass --origin)"
[ "$DEVELOP" != "$MASTER" ] || fail branch_collision 2 "develop and master must differ"
git check-ref-format --branch "$DEVELOP" >/dev/null 2>&1 || fail branch_invalid 2 "invalid develop branch name"
git check-ref-format --branch "$MASTER" >/dev/null 2>&1 || fail branch_invalid 2 "invalid master branch name"
git -C "$REPO" remote get-url "$ORIGIN" >/dev/null 2>&1 || fail origin_missing 4 "origin remote does not exist"

resolve_ref() {
  ref=$1
  label=$2
  if ! oid=$(git -C "$REPO" rev-parse --verify "$ref^{commit}" 2>/dev/null); then
    fail ref_missing 4 "$label ref does not exist"
  fi
  printf '%s\n' "$oid"
}

D_LOCAL=$(resolve_ref "refs/heads/$DEVELOP" "local develop")
D_REMOTE=$(resolve_ref "refs/remotes/$ORIGIN/$DEVELOP" "remote-tracking develop")
M_LOCAL=$(resolve_ref "refs/heads/$MASTER" "local master")
M_REMOTE=$(resolve_ref "refs/remotes/$ORIGIN/$MASTER" "remote-tracking master")

branch_counts() {
  branch=$1
  if ! counts=$(git -C "$REPO" rev-list --left-right --count "refs/heads/$branch...refs/remotes/$ORIGIN/$branch" 2>/dev/null); then
    fail git_error 5 "git rev-list failed for $branch"
  fi
  set -- $counts
  [ $# -eq 2 ] || fail git_error 5 "unexpected git rev-list output for $branch"
  printf '%s %s\n' "$1" "$2"
}

D_COUNTS=$(branch_counts "$DEVELOP")
D_AHEAD=${D_COUNTS%% *}
D_BEHIND=${D_COUNTS##* }
M_COUNTS=$(branch_counts "$MASTER")
M_AHEAD=${M_COUNTS%% *}
M_BEHIND=${M_COUNTS##* }

branch_status() {
  ahead=$1
  behind=$2
  if [ "$ahead" -eq 0 ] && [ "$behind" -eq 0 ]; then
    printf 'aligned\n'
  elif [ "$ahead" -gt 0 ] && [ "$behind" -gt 0 ]; then
    printf 'diverged\n'
  elif [ "$ahead" -gt 0 ]; then
    printf 'ahead\n'
  else
    printf 'behind\n'
  fi
}
D_STATUS=$(branch_status "$D_AHEAD" "$D_BEHIND")
M_STATUS=$(branch_status "$M_AHEAD" "$M_BEHIND")
if [ "$D_STATUS" = aligned ] && [ "$M_STATUS" = aligned ]; then OK=true; else OK=false; fi

if [ "$AS_TEXT" -eq 1 ]; then
  printf 'develop: name=%s local=%s remote=%s ahead=%s behind=%s status=%s\n' \
    "$DEVELOP" "$D_LOCAL" "$D_REMOTE" "$D_AHEAD" "$D_BEHIND" "$D_STATUS"
  printf 'master: name=%s local=%s remote=%s ahead=%s behind=%s status=%s\n' \
    "$MASTER" "$M_LOCAL" "$M_REMOTE" "$M_AHEAD" "$M_BEHIND" "$M_STATUS"
  printf 'ok=%s\n' "$OK"
else
  printf '{"ok":%s,"data":{"branches":{"develop":{"name":%s,"local":%s,"remote":%s,"ahead":%s,"behind":%s,"status":%s},"master":{"name":%s,"local":%s,"remote":%s,"ahead":%s,"behind":%s,"status":%s}}},"error":null}\n' \
    "$OK" \
    "$(json_escape "$DEVELOP")" "$(json_escape "$D_LOCAL")" "$(json_escape "$D_REMOTE")" "$D_AHEAD" "$D_BEHIND" "$(json_escape "$D_STATUS")" \
    "$(json_escape "$MASTER")" "$(json_escape "$M_LOCAL")" "$(json_escape "$M_REMOTE")" "$M_AHEAD" "$M_BEHIND" "$(json_escape "$M_STATUS")"
fi
exit 0
