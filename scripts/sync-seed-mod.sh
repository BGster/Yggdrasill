#!/bin/sh
# 单一事实源 Mod 校验与 manifest 重算脚本。
#
# 目录关系（2026 单源化）：
#   单一事实源  .ygg/mods/agent-seed
#   兼容入口    packages/mods/agent-seed -> ../../.ygg/mods/agent-seed（相对软链）
#
# 用法：
#   scripts/sync-seed-mod.sh                  # 重算 .ygg/mods/agent-seed/manifest.json 并校验软链
#   scripts/sync-seed-mod.sh --check          # 只读校验（软链目标 + manifest.json 与内容哈希一致）
#   scripts/sync-seed-mod.sh --from project   # 兼容旧参数
#   scripts/sync-seed-mod.sh --no-pack        # 只校验软链/目录，不重算 manifest

set -eu

usage() {
    sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'
}

from="project"
run_pack="yes"
check_only="no"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --check)
            check_only="yes"
            shift
            ;;
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

case "$from" in
    seed|project) ;;
    *)
        echo "用法错误：--from 只接受 seed | project（收到 ${from}）" >&2
        exit 2
        ;;
esac

unset CDPATH
script_dir=$(dirname "$0")
root=$(cd "$script_dir/.." && pwd)
seed_dir="packages/mods/agent-seed"
project_dir=".ygg/mods/agent-seed"
expected_target="../../.ygg/mods/agent-seed"

cd "$root"

if [ ! -d "$project_dir" ]; then
    echo "错误：找不到单一事实源 Mod 目录 $project_dir" >&2
    exit 1
fi

for req in "$project_dir/mod.lua" "$project_dir/pack.json"; do
    if [ ! -f "$req" ]; then
        echo "错误：Mod 缺少必需文件 $req" >&2
        exit 1
    fi
done

if [ ! -d "$project_dir/src" ] || [ ! -d "$project_dir/schemas" ]; then
    echo "错误：Mod 缺少 src/ 或 schemas/ 目录（$project_dir）" >&2
    exit 1
fi

# 校验 packages/mods/agent-seed 软链
if [ -L "$seed_dir" ]; then
    actual_target=$(readlink "$seed_dir")
    if [ "$actual_target" != "$expected_target" ]; then
        echo "错误：$seed_dir 软链目标不符（期望 $expected_target，实际 $actual_target）" >&2
        exit 1
    fi
else
    echo "错误：$seed_dir 应当是指向 $expected_target 的相对软链" >&2
    exit 1
fi

# 校验 schemas 与 packages/schema/schemas 单一事实源一致
for f in "$project_dir"/schemas/*.json; do
    [ -e "$f" ] || continue
    name=$(basename "$f")
    if [ -f "packages/schema/schemas/$name" ] && ! cmp -s "packages/schema/schemas/$name" "$f"; then
        echo "错误：$f 与 packages/schema/schemas/$name 不一致" >&2
        exit 1
    fi
done

if [ "$run_pack" = "no" ]; then
    echo "完成：单源软链 $seed_dir -> $expected_target 已核验（--no-pack 跳过 manifest）"
    exit 0
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "错误：找不到 cargo，无法计算或校验 manifest" >&2
    exit 1
fi

if [ "$check_only" = "yes" ]; then
    tmp_manifest=$(mktemp "${TMPDIR:-/tmp}/ygg-manifest.XXXXXX")
    trap 'rm -f "$tmp_manifest"' EXIT INT TERM
    cp "$project_dir/manifest.json" "$tmp_manifest"
    cargo run --quiet --example pack -- "$project_dir" >/dev/null
    if ! cmp -s "$tmp_manifest" "$project_dir/manifest.json"; then
        cp "$tmp_manifest" "$project_dir/manifest.json"
        echo "错误：$project_dir/manifest.json 与当前文件内容不一致——请运行 ./scripts/sync-seed-mod.sh" >&2
        exit 1
    fi
    rm -f "$tmp_manifest"
    trap - EXIT INT TERM
    echo "完成：单源软链 $seed_dir -> $expected_target 与 $project_dir/manifest.json 校验通过"
    exit 0
fi

echo "pack  $project_dir"
cargo run --quiet --example pack -- "$project_dir" >/dev/null
echo "完成：单源软链 $seed_dir -> $expected_target 已核验，$project_dir/manifest.json 已重算"
