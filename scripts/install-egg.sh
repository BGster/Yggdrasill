#!/usr/bin/env bash
# 一键构建 + 安装副本 + 全局软链（egg 全局化的更新路径载体；r9 / D21）。
#
# 事实源链条：<repo>/target/release/examples/egg →（cp）→ <repo>/.ygg/bin/egg
#           →（链式软链）→ ~/.local/bin/egg。
# 幂等：重建后重跑即同步最新构建产物；全局软链一次性设置、永不重指。
# 回滚 = `rm ~/.local/bin/egg`（零系统残留；安装副本归 .ygg/bin，随仓库约定）。
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

cargo build --release --example egg

mkdir -p .ygg/bin
cp -f target/release/examples/egg .ygg/bin/egg
echo "installed: $root/.ygg/bin/egg"

mkdir -p "$HOME/.local/bin"
ln -sfn "$root/.ygg/bin/egg" "$HOME/.local/bin/egg"
echo "global:    $HOME/.local/bin/egg -> $root/.ygg/bin/egg"

case ":$PATH:" in
  *":$HOME/.local/bin:"*) ;;
  *)
    echo "warn: $HOME/.local/bin 不在 PATH 中——请把它加入 shell 配置后，egg 才能在任意目录可用" >&2
    ;;
esac
