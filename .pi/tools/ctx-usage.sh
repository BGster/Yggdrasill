#!/usr/bin/env bash
# ctx-usage.sh — read-only monitor for a pi session's context-window occupancy.
#
# Usage:
#   ctx-usage.sh [session-id] [--json] [--watch [sec]]
#
#   <session-id>          full UUID or unambiguous prefix, resolved under
#                         ~/.pi/agent/sessions. Omit to use the current session
#                         ($PI_SESSION_FILE). File paths are rejected on purpose:
#                         the parameter is a session id, not a file.
#   --json                machine-readable one-line JSON output
#   --watch [sec]         re-render every <sec> seconds (default 2), Ctrl-C stops
#
# Read-only: never writes to the session file, models.json, or anywhere else.
# Token counts come from the last assistant message's `usage` (provider-reported,
# same source /session renders); the window size is looked up from models.json.

set -eu

SESSIONS_ROOT="${PI_SESSIONS_ROOT:-$HOME/.pi/agent/sessions}"
MODELS_JSON="${PI_MODELS_JSON:-$HOME/.pi/agent/models.json}"
BAR_WIDTH=40

usage() {
  echo "usage: ctx-usage.sh [session-file | session-id] [--json] [--watch [sec]]" >&2
  exit "${1:-0}"
}

json=0; watch=0; interval=2; target=""
while [ $# -gt 0 ]; do
  case "$1" in
    -h|--help) usage 0 ;;
    --json) json=1 ;;
    --watch)
      watch=1
      if [ $# -ge 2 ] && printf '%s' "$2" | grep -Eq '^[0-9]+([.][0-9]+)?$'; then
        interval="$2"; shift
      fi
      ;;
    --*) echo "ctx-usage: unknown option: $1" >&2; usage 1 ;;
    *) target="$1" ;;
  esac
  shift
done

# --- resolve a session id (full UUID or prefix) to its JSONL file
resolve_session_id() {
  t="$1"
  if [ -z "$t" ]; then
    printf '%s' "${PI_SESSION_FILE:-}"
    return 0
  fi
  case "$t" in */*|*.jsonl) return 2 ;; esac   # path-like input: wrong parameter kind
  hits=$(find "$SESSIONS_ROOT" -type f -name "*${t}*.jsonl" 2>/dev/null | sort)
  n=$(printf '%s' "$hits" | grep -c . || true)
  if [ "$n" -eq 0 ]; then return 1; fi
  if [ "$n" -gt 1 ]; then
    echo "ctx-usage: $n sessions match id '$t', using the most recent" >&2
  fi
  printf '%s' "$hits" | tail -n 1
}

# --- last assistant usage + aggregate stats, straight from the session JSONL
stats_json() {
  f="$1"
  jq -s --arg fallback "${PI_MODEL:-}" '
    [ .[] | select(.type == "message") ] as $msgs
    | [ $msgs[] | select(.message.role == "assistant" and .message.usage) ] as $as
    | ($as | last) as $last
    | if $last == null then {"ok": false}
      else
        ($last.message.usage) as $u
        | {"ok": true,
           "model": ($last.message.model // $fallback // "?"),
           "used": ($u.totalTokens
                    // (($u.input // 0) + ($u.cacheRead // 0)
                        + ($u.cacheWrite // 0) + ($u.output // 0))),
           "assistant_msgs": ($as | length),
           "entries": length,
           "cost": ([ $as[] | .message.usage.cost.total // 0 ] | add)}
      end' "$f"
}

# --- context window for a model id, from models.json (0 = unknown)
window_for() {
  m="$1"
  [ -f "$MODELS_JSON" ] || { printf '0'; return 0; }
  jq -r --arg m "$m" '
    [ .. | objects | select((.id? // "") == $m) | (.contextWindow? // empty) ]
    | (first // 0)' "$MODELS_JSON" 2>/dev/null || printf '0'
}

run_once() {
  sess="$1"
  if [ -z "$sess" ] || [ ! -f "$sess" ]; then
    echo "ctx-usage: no session matches id '${target:-<current>}'" >&2
    return 1
  fi
  st=$(stats_json "$sess")
  [ -n "$st" ] || { echo "ctx-usage: failed to parse $sess" >&2; return 1; }

  ok=$(printf '%s' "$st" | jq -r '.ok')
  if [ "$ok" != "true" ]; then
    echo "ctx-usage: no assistant usage recorded yet in: $sess"
    return 0
  fi

  model=$(printf '%s' "$st" | jq -r '.model')
  used=$(printf '%s' "$st" | jq -r '.used')
  window=$(window_for "$model")
  case "$window" in ''|*[!0-9]*) window=0 ;; esac

  if [ "$json" -eq 1 ]; then
    sid=$(head -n 1 "$sess" | jq -r '.id // empty')
    printf '%s' "$st" | jq -c --arg session "$sess" --arg sid "$sid" --argjson window "$window" '
      . + {sessionId: $sid, session: $session, window: $window,
           pct: (if $window > 0
                 then ((.used / $window * 1000 | round) / 10)
                 else null end)}'
    return 0
  fi

  if [ "$window" -gt 0 ]; then
    pct=$(awk -v u="$used" -v w="$window" 'BEGIN{printf "%.1f", u/w*100}')
    filled=$(awk -v u="$used" -v w="$window" -v bw="$BAR_WIDTH" \
             'BEGIN{n=int(u/w*bw+0.5); if(n>bw)n=bw; if(n<0)n=0; printf "%d", n}')
    empty=$((BAR_WIDTH - filled))
    bar_line=$(awk -v f="$filled" -v e="$empty" 'BEGIN{
      full="\342\226\210"; empty="\342\226\260"; s="";
      for(i=0;i<f;i++) s=s full;
      for(i=0;i<e;i++) s=s empty;
      printf "%s", s}')
    scale="$used / $window tokens ($pct%)"
  else
    bar_line=$(awk -v e="$BAR_WIDTH" 'BEGIN{empty="\342\226\260"; s="";
      for(i=0;i<e;i++) s=s empty; printf "%s", s}')
    scale="$used tokens (window unknown: $model not in models.json)"
  fi

  cost=$(printf '%s' "$st" | jq -r '.cost')
  am=$(printf '%s' "$st" | jq -r '.assistant_msgs')
  en=$(printf '%s' "$st" | jq -r '.entries')
  cost_fmt=$(awk -v c="$cost" 'BEGIN{printf "%.4f", c}')

  sid=$(head -n 1 "$sess" | jq -r '.id // "?"')
  echo "id       $sid"
  echo "session  $sess"
  echo "model    $model"
  echo "context  $scale"
  echo "         |$bar_line|"
  echo "msgs     $am assistant / $en entries"
  echo "cost     \$$cost_fmt (session total)"
}

sess=""; rc=0
sess=$(resolve_session_id "$target") || rc=$?
case "$rc" in
  0) : ;;
  2) echo "ctx-usage: '$target' looks like a file path — pass a session id instead" >&2; exit 2 ;;
  *) echo "ctx-usage: no session matches id '${target:-<current>}'" >&2; exit 1 ;;
esac

if [ "$watch" -eq 1 ]; then
  trap 'exit 0' INT
  while :; do
    printf '\033[2J\033[H'
    run_once "$sess"
    sleep "$interval"
  done
else
  run_once "$sess"
fi
