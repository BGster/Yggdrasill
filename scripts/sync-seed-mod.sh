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
#
# 除 manifest 外，本脚本还刷新一处**投影**：core.json 的 scenes.<node> 必读块
# （openwiki/operating/scenes.json + 各文件固定头 → 幕转场台词；由 scripts/gen-scene-readfirst.py 生成）。
# 该块不在任何文案键账里（键账只覆盖 beats + core 的 options/tuning/endings），
# 所以 --check 里的块比对是它唯一的漂移防线：不一致即 fail-closed。

set -eu

usage() {
    sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'
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

# 幕必读块投影：docs/context 固定头 → core.json 的 scenes.<node>（零-src，幂等）。
# check 模式只比对不写盘，漂移/缺件即 fail-closed；--no-pack 已在上方提前退出（本模式不重算 manifest，
# 若在此写盘会让 manifest 失真）。
gen_script="$root/scripts/gen-scene-readfirst.py"
if [ -f "$gen_script" ]; then
    if ! command -v python3 >/dev/null 2>&1; then
        echo "错误：找不到 python3，无法校验或生成幕必读块（$gen_script）" >&2
        exit 1
    fi
    if [ "$check_only" = "yes" ]; then
        if ! python3 "$gen_script" --check --root "$root"; then
            echo "错误：core.json 的 [READ FIRST] 块与 docs/context 固定头漂移——请运行 ./scripts/sync-seed-mod.sh" >&2
            exit 1
        fi
    else
        python3 "$gen_script" --root "$root"
    fi
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
    # S38：盘上→声明完备性（S32 事故根因——新拆分文件未入 pack.json code.modules，
    # 声明→盘上方向校验恒绿，egg run 装载才白名单拒）。src/ 全域 lua 与 modules 差集非空即红。
    undeclared=$(python3 - "$project_dir" <<'PYCHK'
import json,os,sys
root=sys.argv[1]
pack=json.load(open(os.path.join(root,"pack.json")))
declared=set(pack.get("code",{}).get("modules",[]))
for dirpath,_,files in os.walk(os.path.join(root,"src")):
    for f in files:
        if f.endswith(".lua"):
            rel=os.path.relpath(os.path.join(dirpath,f),root)
            if rel not in declared:
                print(rel)
PYCHK
)
    if [ -n "$undeclared" ]; then
        echo "错误：src/ 存在未声明进 pack.json code.modules 的模块（盘上→声明完备性失败）：" >&2
        printf '%s\n' "$undeclared" | sed 's/^/  - /' >&2
        echo "  请补声明后运行 ./scripts/sync-seed-mod.sh 重算" >&2
        exit 1
    fi
    echo "完成：单源软链 $seed_dir -> $expected_target 与 $project_dir/manifest.json 校验通过（含盘上→声明完备性）"
    exit 0
fi

echo "pack  $project_dir"
cargo run --quiet --example pack -- "$project_dir" >/dev/null
echo "完成：单源软链 $seed_dir -> $expected_target 已核验，$project_dir/manifest.json 已重算"
