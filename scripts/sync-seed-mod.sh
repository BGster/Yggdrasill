#!/bin/sh
# 同步 seed Mod 与项目 Mod 的**骨架文件**（`mod.lua` + `schemas/`）。
#
# 分层约定（两个 Mod 同处一个仓库，但不是同一份东西）：
#
#   骨架面（机制，必须逐字节一致）   mod.lua / schemas/*  —— 本脚本同步的对象
#   实例面（各项目自己的数据）       data/*              —— 本脚本**不碰**
#
# - `data/playbook.json` 是「本项目的现场规则」，种子版是「通用元工作流纪律」，
#   两者内容必然不同，任何同步都会毁掉其中一侧。
# - `data/config.json` / `data/scenes.json` 同理按项目实例各自维护。
# - `pack.json` 声明的是同构结构（entries / schemaFiles / agent / inboxRoutes），
#   目前两侧一致但语义上属于「各项目自己的声明」，本脚本不覆盖它。
#
# 同步后自动重算**两侧**的 `manifest.json`（内容寻址：sha256 + byteLength，
# 少算一侧就会以「内容哈希不符」拒绝装载）。
#
# 用法：
#   scripts/sync-seed-mod.sh                  # 默认 seed → project（模板推送到项目）
#   scripts/sync-seed-mod.sh --from project   # 反向（项目回灌到模板，改完机制要回流）
#   scripts/sync-seed-mod.sh --no-pack        # 只同步文件，不重算 manifest
#
# 幂等：内容已一致时不写文件；重复跑不产生新的 diff。
#
# 依赖：POSIX sh（无第三方包、无绝对路径假设；重算 manifest 需要 cargo）。
# 一致性由 CI 兜底（`.github/workflows/check.yml` 的骨架校验步），不一致即失败。

set -eu

usage() {
    sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'
}

from="seed"
run_pack="yes"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --from)
            if [ "$#" -lt 2 ]; then
                echo "用法错误：--from 需要参数（seed | project）" >&2
                exit 2
            fi
            from="$2"
            shift 2
            ;;
        --from=*)
            from="${1#--from=}"
            shift
            ;;
        --no-pack)
            run_pack="no"
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "用法错误：未知参数 $1" >&2
            exit 2
            ;;
    esac
done

unset CDPATH
script_dir=$(dirname "$0")
root=$(cd "$script_dir/.." && pwd)
seed_dir="packages/mods/agent-seed"
project_dir=".ygg/mods/agent-seed"

case "$from" in
    seed)
        src="$seed_dir"
        dst="$project_dir"
        ;;
    project)
        src="$project_dir"
        dst="$seed_dir"
        ;;
    *)
        echo "用法错误：--from 只接受 seed | project（收到 ${from}）" >&2
        exit 2
        ;;
esac

cd "$root"

for dir in "$src" "$dst"; do
    if [ ! -d "$dir" ]; then
        echo "错误：找不到 Mod 目录 $dir" >&2
        exit 1
    fi
done

if [ ! -f "$src/mod.lua" ]; then
    echo "错误：源 Mod 缺少 mod.lua（${src}）" >&2
    exit 1
fi
if [ ! -d "$src/schemas" ]; then
    echo "错误：源 Mod 缺少 schemas/（${src}）" >&2
    exit 1
fi

changed=0

copy_file() {
    # copy_file <源> <目标> <展示用路径>
    if [ -f "$2" ] && cmp -s "$1" "$2"; then
        return 0
    fi
    cp "$1" "$2"
    echo "更新  $3"
    changed=1
}

# ---- 骨架：mod.lua ---------------------------------------------------------
copy_file "$src/mod.lua" "$dst/mod.lua" "$dst/mod.lua"

# ---- 骨架：schemas/（镜像，双向都按源的集合收敛）--------------------------
mkdir -p "$dst/schemas"

for f in "$src"/schemas/*.json; do
    [ -e "$f" ] || continue
    name=$(basename "$f")
    copy_file "$f" "$dst/schemas/$name" "$dst/schemas/$name"
done

for f in "$dst"/schemas/*.json; do
    [ -e "$f" ] || continue
    name=$(basename "$f")
    if [ ! -e "$src/schemas/$name" ]; then
        rm -f "$f"
        echo "删除  ${dst}/schemas/${name}（源侧已不存在）"
        changed=1
    fi
done

if [ "$changed" -eq 0 ]; then
    echo "骨架已一致：${src} → ${dst}（mod.lua + schemas/），未写任何文件"
fi

# ---- 重算两侧 manifest ----------------------------------------------------
if [ "$run_pack" = "no" ]; then
    echo "跳过 manifest 重算（--no-pack）；改过骨架文件时请自行重算两侧"
    exit 0
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "错误：找不到 cargo，无法重算 manifest" >&2
    echo "     装 Rust 工具链后重跑，或本轮用 --no-pack 只同步文件" >&2
    exit 1
fi

for dir in "$seed_dir" "$project_dir"; do
    echo "pack  $dir"
    cargo run --quiet --example pack -- "$dir" >/dev/null
done

echo "完成：骨架同步 ${src} → ${dst}，两侧 manifest 已重算"
