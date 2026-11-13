#!/bin/sh
# git-release.sh — merge --no-ff develop into master，再 reset develop 到 master。
# 缺省 dry-run；只有显式 --push 才执行本地变更和远端 push。

set -eu

usage() {
  cat <<'EOF'
git-release.sh [--repo <dir>] [--develop <branch>] [--master <branch>] [--origin <name>] [--push] [--dry-run] [--text] [--json]
  --repo <dir>       default: git toplevel of the caller cwd
  --develop <branch> argv > $YGG_DEVELOP_BRANCH > <repo>/.env.local > <repo>/.env
  --master <branch>  argv > $YGG_MASTER_BRANCH > <repo>/.env.local > <repo>/.env
  --origin <name>    argv > $YGG_ORIGIN > <repo>/.env.local > <repo>/.env
  --push             execute merge/reset and push master/develop (default: dry-run)
  --dry-run          explicit dry-run; mutually exclusive with --push
  --text             human-readable output (default: JSON)
  --json             compatibility no-op; JSON is already the default
  --help             this text
develop is merged into master with --no-ff; develop is pushed with --force-with-lease.
exit: 0 success/plan, 2 usage, 3 env missing, 4 repo/ref error, >=5 preflight/write failure
EOF
}

AS_TEXT=0
for arg in "$@"; do
  case "$arg" in
    --help|-h) usage; exit 0 ;;
  esac
done
if ! command -v python3 >/dev/null 2>&1; then
  printf '%s\n' '{"ok":false,"data":{"steps":[],"partial":false,"failed_step":null},"error":{"code":"env_missing","message":"python3 is required"}}'
  exit 3
fi
if ! command -v git >/dev/null 2>&1; then
  printf '%s\n' '{"ok":false,"data":{"steps":[],"partial":false,"failed_step":null},"error":{"code":"env_missing","message":"git is required"}}'
  exit 3
fi

json_escape() {
  python3 -c 'import json,sys; print(json.dumps(sys.argv[1], ensure_ascii=True).replace("|", "\\u007c"))' "$1"
}

MODE=dry-run
MODE_REQUEST=""
REPO=""
ARG_DEVELOP=""
ARG_MASTER=""
ARG_ORIGIN=""
STEPS_JSON=""
PARTIAL=false
MUTATED=false
FAILED_STEP=""
MERGE_SHA=""
PUSHED=false
RESET_DEVELOP=false
ERROR_CODE=""
ERROR_MESSAGE=""

append_step() {
  AP_NAME=$(json_escape "$1")
  AP_STATUS=$(json_escape "$2")
  AP_COMMAND=$(json_escape "$3")
  AP_STEP="{\"name\":$AP_NAME,\"status\":$AP_STATUS,\"command\":$AP_COMMAND}"
  if [ -z "$STEPS_JSON" ]; then STEPS_JSON=$AP_STEP; else STEPS_JSON="$STEPS_JSON,$AP_STEP"; fi
}

emit_result() {
  ER_STATUS=$1
  if [ "$ER_STATUS" -eq 0 ]; then ER_OK=true; else ER_OK=false; fi
  if [ -n "$FAILED_STEP" ]; then ER_FAILED=$(json_escape "$FAILED_STEP"); else ER_FAILED=null; fi
  if [ -n "$MERGE_SHA" ]; then ER_MERGE=$(json_escape "$MERGE_SHA"); else ER_MERGE=null; fi
  if [ -n "$ERROR_CODE" ]; then
    ER_ERROR="{\"code\":$(json_escape "$ERROR_CODE"),\"message\":$(json_escape "$ERROR_MESSAGE")}"
  else
    ER_ERROR=null
  fi
  if [ "$AS_TEXT" -eq 1 ]; then
    printf 'mode=%s repo=%s ok=%s partial=%s failed_step=%s merge_sha=%s pushed=%s reset_develop=%s\n' \
      "$MODE" "$REPO" "$ER_OK" "$PARTIAL" "${FAILED_STEP:-null}" "${MERGE_SHA:-null}" "$PUSHED" "$RESET_DEVELOP"
    if [ -n "$STEPS_JSON" ]; then printf 'steps=[%s]\n' "$STEPS_JSON"; else printf 'steps=[]\n'; fi
    if [ -n "$ERROR_CODE" ]; then printf 'error.%s: %s\n' "$ERROR_CODE" "$ERROR_MESSAGE"; fi
  else
    printf '{"ok":%s,"data":{"mode":%s,"repo":%s,"steps":[%s],"merge_sha":%s,"pushed":%s,"reset_develop":%s,"partial":%s,"failed_step":%s},"error":%s}\n' \
      "$ER_OK" "$(json_escape "$MODE")" "$(json_escape "$REPO")" "$STEPS_JSON" "$ER_MERGE" "$PUSHED" "$RESET_DEVELOP" "$PARTIAL" "$ER_FAILED" "$ER_ERROR"
  fi
  exit "$ER_STATUS"
}

fail_result() {
  FR_CODE=$1
  FR_STATUS=$2
  FR_MESSAGE=$3
  FR_STEP=${4-}
  ERROR_CODE=$FR_CODE
  ERROR_MESSAGE=$FR_MESSAGE
  FAILED_STEP=$FR_STEP
  if [ "$MUTATED" = true ]; then PARTIAL=true; fi
  emit_result "$FR_STATUS"
}
usage_fail() { fail_result usage 2 "$1" ""; }

while [ $# -gt 0 ]; do
  case "$1" in
    --repo) [ $# -ge 2 ] || usage_fail "--repo requires a value"; REPO=$2; shift 2 ;;
    --develop) [ $# -ge 2 ] || usage_fail "--develop requires a value"; ARG_DEVELOP=$2; shift 2 ;;
    --master) [ $# -ge 2 ] || usage_fail "--master requires a value"; ARG_MASTER=$2; shift 2 ;;
    --origin) [ $# -ge 2 ] || usage_fail "--origin requires a value"; ARG_ORIGIN=$2; shift 2 ;;
    --push)
      [ "$MODE_REQUEST" != dry-run ] || usage_fail "--push and --dry-run are mutually exclusive"
      MODE=execute; MODE_REQUEST=execute; shift ;;
    --dry-run)
      [ "$MODE_REQUEST" != execute ] || usage_fail "--push and --dry-run are mutually exclusive"
      MODE=dry-run; MODE_REQUEST=dry-run; shift ;;
    --text) AS_TEXT=1; shift ;;
    --json) shift ;;
    *) usage_fail "unknown argument: $1" ;;
  esac
done

if [ -z "$REPO" ]; then
  if ! REPO=$(git rev-parse --show-toplevel 2>/dev/null); then
    REPO=${REPO:-.}
    fail_result repo_invalid 4 "not in a git worktree; pass --repo" ""
  fi
else
  if ! REPO=$(git -C "$REPO" rev-parse --show-toplevel 2>/dev/null); then
    fail_result repo_invalid 4 "--repo is not a git worktree" ""
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
[ "$ENV_STATUS" -eq 0 ] || fail_result env_invalid 3 "failed to load repository .env/.env.local" ""
if [ -n "$PROCESS_DEVELOP" ]; then DEVELOP=$PROCESS_DEVELOP; else DEVELOP=${YGG_DEVELOP_BRANCH-}; fi
if [ -n "$PROCESS_MASTER" ]; then MASTER=$PROCESS_MASTER; else MASTER=${YGG_MASTER_BRANCH-}; fi
if [ -n "$PROCESS_ORIGIN" ]; then ORIGIN=$PROCESS_ORIGIN; else ORIGIN=${YGG_ORIGIN-}; fi
[ -z "$ARG_DEVELOP" ] || DEVELOP=$ARG_DEVELOP
[ -z "$ARG_MASTER" ] || MASTER=$ARG_MASTER
[ -z "$ARG_ORIGIN" ] || ORIGIN=$ARG_ORIGIN
[ -n "$DEVELOP" ] || fail_result env_missing 3 "YGG_DEVELOP_BRANCH is required (or pass --develop)" ""
[ -n "$MASTER" ] || fail_result env_missing 3 "YGG_MASTER_BRANCH is required (or pass --master)" ""
[ -n "$ORIGIN" ] || fail_result env_missing 3 "YGG_ORIGIN is required (or pass --origin)" ""
[ "$DEVELOP" != "$MASTER" ] || fail_result branch_collision 2 "develop and master must differ" ""
git check-ref-format --branch "$DEVELOP" >/dev/null 2>&1 || fail_result branch_invalid 2 "invalid develop branch name" ""
git check-ref-format --branch "$MASTER" >/dev/null 2>&1 || fail_result branch_invalid 2 "invalid master branch name" ""
git -C "$REPO" remote get-url "$ORIGIN" >/dev/null 2>&1 || fail_result origin_missing 4 "origin remote does not exist" ""
for branch in "$DEVELOP" "$MASTER"; do
  git -C "$REPO" show-ref --verify --quiet "refs/heads/$branch" || fail_result ref_missing 4 "local branch does not exist: $branch" ""
  git -C "$REPO" show-ref --verify --quiet "refs/remotes/$ORIGIN/$branch" || fail_result ref_missing 4 "remote-tracking branch does not exist: $ORIGIN/$branch" ""
done

if [ "$MODE" = execute ]; then
  if [ -n "$(git -C "$REPO" status --porcelain)" ]; then
    fail_result worktree_dirty 5 "worktree must be clean before --push" preflight
  fi
  for operation in MERGE_HEAD rebase-merge rebase-apply CHERRY_PICK_HEAD; do
    OPERATION_PATH=$(git -C "$REPO" rev-parse --git-path "$operation")
    [ ! -e "$OPERATION_PATH" ] || fail_result operation_in_progress 5 "another git operation is in progress: $operation" preflight
  done
fi

run_step() {
  RS_NAME=$1
  RS_COMMAND=$2
  shift 2
  if git -C "$REPO" "$@" >&2; then
    append_step "$RS_NAME" ok "$RS_COMMAND"
    MUTATED=true
  else
    RS_STATUS=$?
    append_step "$RS_NAME" failed "$RS_COMMAND"
    MUTATED=true
    fail_result git_failed 6 "git command failed with exit $RS_STATUS: $RS_COMMAND" "$RS_NAME"
  fi
}
plan_step() { append_step "$1" planned "$2"; }

get_alignment() {
  GA_BRANCH=$1
  if ! GA_COUNTS=$(git -C "$REPO" rev-list --left-right --count "refs/heads/$GA_BRANCH...refs/remotes/$ORIGIN/$GA_BRANCH" 2>/dev/null); then
    fail_result git_failed 5 "cannot compute alignment for $GA_BRANCH" "alignment_$GA_BRANCH"
  fi
  set -- $GA_COUNTS
  GA_AHEAD=$1
  GA_BEHIND=$2
}
check_or_align() {
  CA_BRANCH=$1
  get_alignment "$CA_BRANCH"
  if [ "$GA_AHEAD" -gt 0 ]; then
    if [ "$GA_BEHIND" -gt 0 ]; then CA_SHAPE=diverged; else CA_SHAPE=ahead; fi
    append_step "alignment_$CA_BRANCH" failed "git rev-list --left-right --count $CA_BRANCH...$ORIGIN/$CA_BRANCH"
    fail_result unsafe_alignment 5 "$CA_BRANCH is $CA_SHAPE; refusing release" "alignment_$CA_BRANCH"
  fi
  if [ "$GA_BEHIND" -gt 0 ]; then
    if [ "$MODE" = dry-run ]; then
      plan_step "checkout_$CA_BRANCH" "git checkout $CA_BRANCH"
      plan_step "alignment_$CA_BRANCH" "git merge --ff-only $ORIGIN/$CA_BRANCH"
    else
      run_step "checkout_$CA_BRANCH" "git checkout $CA_BRANCH" checkout "$CA_BRANCH"
      run_step "alignment_$CA_BRANCH" "git merge --ff-only $ORIGIN/$CA_BRANCH" merge --ff-only "$ORIGIN/$CA_BRANCH"
    fi
  fi
}

run_merge_precheck() {
  SCRIPT_DIR=$(CDPATH= cd "$(dirname "$0")" && pwd)
  if [ "$MODE" = dry-run ]; then
    plan_step merge_precheck "git-conflict-check.sh --check merge --develop $DEVELOP --master $MASTER"
    return
  fi
  if ! CHECK_JSON=$("$SCRIPT_DIR/git-conflict-check.sh" --check merge --repo "$REPO" --develop "$DEVELOP" --master "$MASTER"); then
    printf '%s\n' "$CHECK_JSON" >&2
    append_step merge_precheck failed "git-conflict-check.sh --check merge"
    fail_result conflict_precheck_failed 6 "merge conflict precheck malfunctioned" merge_precheck
  fi
  if ! CHECK_CONFLICT=$(printf '%s' "$CHECK_JSON" | python3 -c 'import json,sys; o=json.load(sys.stdin); assert o.get("error") is None and isinstance(o.get("ok"),bool); c=o["data"]["merge"]["conflict"]; assert isinstance(c,bool); print("true" if c else "false")'); then
    append_step merge_precheck failed "git-conflict-check.sh --check merge"
    fail_result conflict_precheck_failed 6 "merge conflict precheck returned malformed JSON" merge_precheck
  fi
  if [ "$CHECK_CONFLICT" = true ]; then
    append_step merge_precheck failed "git-conflict-check.sh --check merge"
    fail_result merge_conflict 6 "merge conflict detected; release not started" merge_precheck
  fi
  append_step merge_precheck ok "git-conflict-check.sh --check merge"
}

if [ "$MODE" = dry-run ]; then plan_step fetch "git fetch $ORIGIN"; else run_step fetch "git fetch $ORIGIN" fetch "$ORIGIN"; fi
check_or_align "$MASTER"
check_or_align "$DEVELOP"
run_merge_precheck
if [ "$MODE" = dry-run ]; then
  plan_step checkout_master "git checkout $MASTER"
  plan_step merge_develop "git merge --no-ff $DEVELOP"
  plan_step push_master "git push $ORIGIN $MASTER"
  plan_step checkout_develop "git checkout $DEVELOP"
  plan_step reset_develop "git reset --hard $MASTER"
  plan_step push_develop "git push --force-with-lease $ORIGIN $DEVELOP"
  plan_step restore_master "git checkout $MASTER"
else
  run_step checkout_master "git checkout $MASTER" checkout "$MASTER"
  run_step merge_develop "git merge --no-ff $DEVELOP" merge --no-ff "$DEVELOP"
  MERGE_SHA=$(git -C "$REPO" rev-parse "refs/heads/$MASTER^{commit}") || fail_result git_failed 6 "cannot resolve release merge SHA" merge_develop
  run_step push_master "git push $ORIGIN $MASTER" push "$ORIGIN" "$MASTER"
  run_step checkout_develop "git checkout $DEVELOP" checkout "$DEVELOP"
  run_step reset_develop "git reset --hard $MASTER" reset --hard "$MASTER"
  RESET_DEVELOP=true
  run_step push_develop "git push --force-with-lease $ORIGIN $DEVELOP" push --force-with-lease "$ORIGIN" "$DEVELOP"
  run_step restore_master "git checkout $MASTER" checkout "$MASTER"
  PUSHED=true
  PARTIAL=false
fi
emit_result 0
