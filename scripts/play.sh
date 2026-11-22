#!/bin/sh
# scripts/play.sh —— 四复现仓游戏一键启动器（菜单选局 → 自动起 run → 直接进交互游玩）
# 用法：sh ~/WorkHub/Yggdrasill/scripts/play.sh    （或加入 alias：alias yggplay='sh ~/WorkHub/Yggdrasill/scripts/play.sh'）
# 依赖：engine release 构建（target/release/examples/egg）；缺则自动跑 install-egg.sh。

YGG="${YGG_ROOT:-$HOME/WorkHub/Yggdrasill}"
PKG="$YGG/packages/libs"
EGG="$YGG/target/release/examples/egg"
PR="$HOME/WorkHub/PokeRogue-YGG"
SL="$HOME/WorkHub/Saltline-YGG"
DC="$HOME/WorkHub/DCSS-YGG"
CD="$HOME/WorkHub/CDDA-YGG"

# ---------- 前置自检 ----------
if [ ! -x "$EGG" ]; then
  echo "engine release 产物缺失，先构建（几分钟）……"
  sh "$YGG/scripts/install-egg.sh" >/dev/null 2>&1 || { echo "构建失败，请手动跑: sh $YGG/scripts/install-egg.sh"; exit 1; }
  EGG="$YGG/.ygg/bin/egg"
fi
export YGG_PACKAGES_ROOT="$PKG"

# serve（观察面 127.0.0.1:7800）
if ! curl -s -o /dev/null http://127.0.0.1:7800/api/roots 2>/dev/null; then
  nohup "$EGG" serve >/tmp/ygg-serve.log 2>&1 &
  sleep 1
  echo "serve 已起: http://127.0.0.1:7800"
fi

# PokeRogue viewer 静态服务（8765）
if ! curl -s -o /dev/null http://localhost:8765/viewer/index.html 2>/dev/null; then
  (cd "$PR" && nohup python3 -m http.server 8765 >/tmp/pr-http.log 2>&1 &)
  sleep 1
  echo "viewer 已起: http://localhost:8765/viewer/index.html"
fi

# ---------- 工具 ----------
new_run() {   # $1=mod 路径  $2=runs 目录  $3=开局事件
  mkdir -p "$2"
  ROOT=$("$EGG" new --mod "$1" --in "$2" --event "$3" 2>/dev/null | tail -1 | sed 's/^run-root: //')
  [ -n "$ROOT" ] && echo "$ROOT"
}
attach() {    # $1=run-root  （stdin 交给你；q 或 Ctrl-D 安全暂停）
  "$EGG" open "$1" --drive stdin
  echo "（局已安全暂停，可随时回来续玩：$EGG open $1 --drive stdin）"
}

# ---------- 菜单 ----------
while :; do
  printf '\n\033[1m==== Yggdrasill 游戏启动器 ====\033[0m\n'
  printf '  1) PokeRogue   宝可梦式回合战斗（带浏览器图形面 viewer）\n'
  printf '  2) Saltline    3D FPS 复现 · 字符渲染（动作 1..12：1 前进 7 开火 9 交互）\n'
  printf '  3) DCSS        地牢爬行（开局选 1=人类战士，之后方向/动作选项）\n'
  printf '  4) CDDA        末日生存回合制\n'
  printf '  5) 退出\n'
  printf '选择: '
  read -r pick || break
  case "$pick" in
    1)
      ROOT=$(new_run "$PR/mod" "$PR/.ygg/runs" game.boot)
      [ -z "$ROOT" ] && { echo "开局失败"; continue; }
      ln -sfn "$ROOT" "$PR/.ygg/latest"
      echo "新局: $ROOT"
      echo "浏览器开: http://localhost:8765/viewer/index.html  （随回合刷新页面）"
      attach "$ROOT" ;;
    2) ROOT=$(new_run "$SL" /tmp/sl-play game.boot);  [ -n "$ROOT" ] && attach "$ROOT" || echo "开局失败" ;;
    3) ROOT=$(new_run "$DC/mod" /tmp/dcss-play event.start); [ -n "$ROOT" ] && attach "$ROOT" || echo "开局失败" ;;
    4) ROOT=$(new_run "$CD" /tmp/cdda-play event.start); [ -n "$ROOT" ] && attach "$ROOT" || echo "开局失败" ;;
    5) break ;;
    *) echo '请输 1-5' ;;
  esac
done
echo "bye"
