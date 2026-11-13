#!/bin/sh
# git-conflict-check.sh — rebase/merge 冲突只读预检。
# merge-tree 只写不可达 tree object；不 fetch，不改 refs/index/worktree。

set -eu

usage() {
  cat <<'EOF'
git-conflict-check.sh --check all|rebase|merge [--repo <dir>] [--feature <branch>] [--develop <branch>] [--master <branch>] [--text] [--json]
  --check all|rebase|merge  required; merge: ours=master,theirs=develop; rebase: ours=develop,theirs=feature
  --repo <dir>       default: git toplevel of the caller cwd
  --feature <branch> required for all/rebase; argv-only integration context
  --develop <branch> argv > $YGG_DEVELOP_BRANCH > <repo>/.env.local > <repo>/.env
  --master <branch>  required for all/merge; argv > $YGG_MASTER_BRANCH > <repo>/.env.local > <repo>/.env
  --text             human-readable output (default: JSON)
  --json             compatibility no-op; JSON is already the default
  --help             this text
exit: 0 detection completed (see ok), 2 usage, 3 env missing, 4 repo/ref/git unsupported, 5 git error
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

CHECK=""
REPO=""
FEATURE=""
ARG_DEVELOP=""
ARG_MASTER=""
while [ $# -gt 0 ]; do
  case "$1" in
    --check) [ $# -ge 2 ] || fail usage 2 "--check requires a value"; CHECK=$2; shift 2 ;;
    --repo) [ $# -ge 2 ] || fail usage 2 "--repo requires a value"; REPO=$2; shift 2 ;;
    --feature) [ $# -ge 2 ] || fail usage 2 "--feature requires a value"; FEATURE=$2; shift 2 ;;
    --develop) [ $# -ge 2 ] || fail usage 2 "--develop requires a value"; ARG_DEVELOP=$2; shift 2 ;;
    --master) [ $# -ge 2 ] || fail usage 2 "--master requires a value"; ARG_MASTER=$2; shift 2 ;;
    --text) AS_TEXT=1; shift ;;
    --json) shift ;;
    *) fail usage 2 "unknown argument: $1" ;;
  esac
done
case "$CHECK" in
  all|rebase|merge) ;;
  "") fail usage 2 "missing required --check all|rebase|merge" ;;
  *) fail usage 2 "--check must be all, rebase, or merge" ;;
esac
if [ "$CHECK" != merge ] && [ -z "$FEATURE" ]; then
  fail usage 2 "--feature is required for rebase/all"
fi

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
[ -z "$ARG_DEVELOP" ] || DEVELOP=$ARG_DEVELOP
[ -z "$ARG_MASTER" ] || MASTER=$ARG_MASTER
[ -n "$DEVELOP" ] || fail env_missing 3 "YGG_DEVELOP_BRANCH is required (or pass --develop)"
if [ "$CHECK" != rebase ] || [ -n "$MASTER" ]; then
  [ -n "$MASTER" ] || fail env_missing 3 "YGG_MASTER_BRANCH is required (or pass --master)"
  [ "$DEVELOP" != "$MASTER" ] || fail branch_collision 2 "develop and master must differ"
  git check-ref-format --branch "$MASTER" >/dev/null 2>&1 || fail branch_invalid 2 "invalid master branch name"
fi
if [ -n "$FEATURE" ]; then
  [ "$FEATURE" != "$DEVELOP" ] || fail branch_collision 2 "feature and develop must differ"
  if [ -n "$MASTER" ]; then
    [ "$FEATURE" != "$MASTER" ] || fail branch_forbidden 2 "feature must not integrate directly into master"
  fi
fi
git check-ref-format --branch "$DEVELOP" >/dev/null 2>&1 || fail branch_invalid 2 "invalid develop branch name"
if [ -n "$FEATURE" ]; then
  git check-ref-format --branch "$FEATURE" >/dev/null 2>&1 || fail branch_invalid 2 "invalid feature branch name"
fi

require_branch() {
  branch=$1
  label=$2
  git -C "$REPO" show-ref --verify --quiet "refs/heads/$branch" || fail ref_missing 4 "$label local branch does not exist"
}
require_branch "$DEVELOP" "develop"
if [ "$CHECK" = all ] || [ "$CHECK" = merge ]; then
  require_branch "$MASTER" "master"
fi
[ -z "$FEATURE" ] || require_branch "$FEATURE" "feature"

TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/ygg-git-conflict.XXXXXX") || fail env_missing 3 "mktemp failed"
trap 'rm -rf "$TMP_DIR"' 0 1 2 3 15

MT_STATUS=0
MT_TREE=""
capture_conflicts() {
  CAP_FILES_JSON=""
  CAP_FIRST_JSON=null
  CAP_TOTAL=0
  CAP_TRUNCATED=false
  while IFS= read -r file || [ -n "$file" ]; do
    CAP_TOTAL=$((CAP_TOTAL + 1))
    if [ "$CAP_TOTAL" -eq 1 ]; then CAP_FIRST_JSON=$(json_escape "$file"); fi
    if [ "$CAP_TOTAL" -le 64 ]; then
      escaped=$(json_escape "$file")
      if [ -z "$CAP_FILES_JSON" ]; then CAP_FILES_JSON=$escaped; else CAP_FILES_JSON="$CAP_FILES_JSON,$escaped"; fi
    else
      CAP_TRUNCATED=true
    fi
  done < "$TMP_DIR/files"
}

run_merge_tree() {
  if git -C "$REPO" merge-tree --write-tree --name-only "$@" >"$TMP_DIR/out" 2>"$TMP_DIR/err"; then
    MT_STATUS=0
  else
    MT_STATUS=$?
  fi
  if [ "$MT_STATUS" -gt 1 ]; then
    cat "$TMP_DIR/err" >&2
    if [ "$MT_STATUS" -eq 129 ]; then
      fail git_unsupported 4 "git merge-tree --write-tree/--merge-base is unsupported"
    fi
    fail git_error 5 "git merge-tree failed with exit $MT_STATUS"
  fi
  MT_TREE=$(awk 'NR == 1 { print; exit }' "$TMP_DIR/out")
  [ -n "$MT_TREE" ] || fail git_error 5 "git merge-tree returned no tree OID"
  awk 'NR == 1 { next } NF == 0 { exit } { print }' "$TMP_DIR/out" >"$TMP_DIR/files"
}

RE_APPLICABLE=false
RE_CONFLICT=false
RE_COMMITS=0
RE_FILES_JSON=""
RE_FIRST_JSON=null
RE_NOOP=true
RE_TRUNCATED=false
RE_FILE_TOTAL=0
ME_APPLICABLE=false
ME_CONFLICT=false
ME_COMMITS=0
ME_FILES_JSON=""
ME_FIRST_JSON=null
ME_NOOP=true
ME_TRUNCATED=false
ME_FILE_TOTAL=0

if [ "$CHECK" = all ] || [ "$CHECK" = rebase ]; then
  RE_APPLICABLE=true
  D_REF="refs/heads/$DEVELOP"
  F_REF="refs/heads/$FEATURE"
  if ! BASE=$(git -C "$REPO" merge-base "$D_REF" "$F_REF" 2>/dev/null); then
    fail git_error 5 "git merge-base failed for develop/feature"
  fi
  git -C "$REPO" rev-list --reverse --no-merges "$BASE..$F_REF" >"$TMP_DIR/commits" || fail git_error 5 "git rev-list failed for rebase commits"
  git -C "$REPO" cherry "$D_REF" "$F_REF" "$BASE" >"$TMP_DIR/cherry" || fail git_error 5 "git cherry failed for rebase commits"
  CURRENT=$(git -C "$REPO" rev-parse "$D_REF^{commit}") || fail git_error 5 "cannot resolve develop HEAD"
  while IFS= read -r commit || [ -n "$commit" ]; do
    [ -n "$commit" ] || continue
    if grep -q "^- $commit\$" "$TMP_DIR/cherry"; then
      continue
    fi
    RE_COMMITS=$((RE_COMMITS + 1))
    PARENT=$(git -C "$REPO" rev-parse "$commit^1") || fail git_error 5 "cannot resolve rebase commit parent"
    run_merge_tree --merge-base "$PARENT" "$CURRENT" "$commit"
    if [ "$MT_STATUS" -eq 1 ]; then
      RE_CONFLICT=true
      capture_conflicts
      RE_FILES_JSON=$CAP_FILES_JSON
      RE_FIRST_JSON=$CAP_FIRST_JSON
      RE_FILE_TOTAL=$CAP_TOTAL
      RE_TRUNCATED=$CAP_TRUNCATED
      break
    fi
    CURRENT=$MT_TREE
  done < "$TMP_DIR/commits"
  if [ "$RE_COMMITS" -gt 0 ]; then RE_NOOP=false; fi
fi

if [ "$CHECK" = all ] || [ "$CHECK" = merge ]; then
  ME_APPLICABLE=true
  M_REF="refs/heads/$MASTER"
  D_REF="refs/heads/$DEVELOP"
  ME_COMMITS=$(git -C "$REPO" rev-list --count "$M_REF..$D_REF") || fail git_error 5 "git rev-list failed for merge commits"
  if git -C "$REPO" merge-base --is-ancestor "$D_REF" "$M_REF"; then
    ME_NOOP=true
  else
    ME_NOOP=false
  fi
  run_merge_tree "$M_REF" "$D_REF"
  if [ "$MT_STATUS" -eq 1 ]; then
    ME_CONFLICT=true
    capture_conflicts
    ME_FILES_JSON=$CAP_FILES_JSON
    ME_FIRST_JSON=$CAP_FIRST_JSON
    ME_FILE_TOTAL=$CAP_TOTAL
    ME_TRUNCATED=$CAP_TRUNCATED
  fi
fi

if [ "$RE_CONFLICT" = true ] || [ "$ME_CONFLICT" = true ]; then OK=false; else OK=true; fi
if [ "$AS_TEXT" -eq 1 ]; then
  printf 'rebase: applicable=%s conflict=%s commits=%s files=[%s] first=%s noop=%s truncated=%s\n' \
    "$RE_APPLICABLE" "$RE_CONFLICT" "$RE_COMMITS" "$RE_FILES_JSON" "$RE_FIRST_JSON" "$RE_NOOP" "$RE_TRUNCATED"
  printf 'merge: applicable=%s conflict=%s commits=%s files=[%s] first=%s noop=%s truncated=%s\n' \
    "$ME_APPLICABLE" "$ME_CONFLICT" "$ME_COMMITS" "$ME_FILES_JSON" "$ME_FIRST_JSON" "$ME_NOOP" "$ME_TRUNCATED"
  printf 'ok=%s\n' "$OK"
else
  printf '{"ok":%s,"data":{"rebase":{"applicable":%s,"conflict":%s,"commits":%s,"files":[%s],"first":%s,"noop":%s,"file_total":%s,"truncated":%s},"merge":{"applicable":%s,"conflict":%s,"commits":%s,"files":[%s],"first":%s,"noop":%s,"file_total":%s,"truncated":%s}},"error":null}\n' \
    "$OK" \
    "$RE_APPLICABLE" "$RE_CONFLICT" "$RE_COMMITS" "$RE_FILES_JSON" "$RE_FIRST_JSON" "$RE_NOOP" "$RE_FILE_TOTAL" "$RE_TRUNCATED" \
    "$ME_APPLICABLE" "$ME_CONFLICT" "$ME_COMMITS" "$ME_FILES_JSON" "$ME_FIRST_JSON" "$ME_NOOP" "$ME_FILE_TOTAL" "$ME_TRUNCATED"
fi
exit 0
