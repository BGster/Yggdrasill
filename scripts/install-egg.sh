#!/usr/bin/env bash
# 一键构建 + 安装副本 + 全局软链（egg 全局化的更新路径载体；r9 / D21）。
#
# 事实源链条：<repo>/target/release/examples/egg →（cp）→ <repo>/.ygg/bin/egg
#           →（链式软链）→ ~/.local/bin/egg。
# 幂等：重建后重跑即同步最新构建产物；全局软链一次性设置、永不重指。
# 回滚 = `rm ~/.local/bin/egg`（零系统残留；安装副本归 .ygg/bin，随仓库约定）。
# macOS：安装副本必须 ad-hoc 签名，否则首次 exec 被内核杀掉（exit 137，无错误文本）；
#       签名必须作用于**副本**——`cp` 不会带走 target/ 里那份的签名。
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

cargo build --release --example egg

mkdir -p .ygg/bin
cp -f target/release/examples/egg .ygg/bin/egg

# macOS 必要步骤：未签名二进制被内核 SIGKILL（exit 137，无错误文本）。
# 必须在 cp 之后签名（签名不随 cp 迁移）；无 codesign 的环境（Linux/CI）自动跳过。
if [ "$(uname -s)" = "Darwin" ] && command -v codesign >/dev/null 2>&1; then
  codesign --sign - --force .ygg/bin/egg
  echo "signed:    .ygg/bin/egg (ad-hoc)"
fi

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
