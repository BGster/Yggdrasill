#!/usr/bin/env bash
# 长跑双端一致性矩阵（ADR-S3-6）：同一份包镜像 + 同一个种子，原生（`--example golden`）
# 与 wasm（`scripts/wasm-lane.mjs --ticks`）跑同一串 tick 数，逐档比最终 state hash。
#
# 用法：bash scripts/persist-matrix.sh [--ticks "1000 10000 100000"] [--image <路径>] [--pack <目录>]
#
# 只读、可反复跑（不写任何产物）。开跑前先把「产物是新的」验一遍：wasm 二进制过期时
# 「双端不等」会变成一句查不出原因的话（实证：预算按入口重置的修复落地后没重建 wasm，
# 参考包跑到第 1586 tick 才炸 E_BUDGET）。
#
# 退出码：0 全档一致；1 有档不一致或有档跑挂；2 产物缺失/过期（附修复命令）。
# 首档的 wall 会带上一次性编译时间；比速度看每行的 native=/wasm=（两侧都只圈步进循环）。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TICKS="1000 10000 100000"
IMAGE="hosts/h5/pack.image.js"
PACK="packs/example"
WASM_GLUE="crates/ygg-wasm/pkg/ygg_wasm.js"
WASM_BIN="crates/ygg-wasm/pkg/ygg_wasm_bg.wasm"

usage() {
  echo "用法: bash scripts/persist-matrix.sh [--ticks \"1000 10000 100000\"] [--image <pack.image.js|json>] [--pack <目录>]"
  echo "  逐档跑原生与 wasm 两侧，比最终 state hash；不等即非零退出并打印两端数字"
}

fail() {
  echo "persist-matrix: $*" >&2
  exit 2
}

die_run() {
  echo "persist-matrix: $*" >&2
  exit 1
}

while [ $# -gt 0 ]; do
  case "$1" in
    --ticks)
      [ $# -ge 2 ] || fail "--ticks 需要一个空格分隔的 tick 列表，如 \"1000 10000\""
      TICKS="$2"
      shift 2
      ;;
    --image)
      [ $# -ge 2 ] || fail "--image 需要指定文件路径"
      IMAGE="$2"
      shift 2
      ;;
    --pack)
      [ $# -ge 2 ] || fail "--pack 需要指定包目录"
      PACK="$2"
      shift 2
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      fail "未知参数：$1（见 --help）"
      ;;
  esac
done

BUILD_WASM_CMD="bash scripts/build-wasm.sh"
BUILD_IMAGE_CMD="cargo run -q --example image -p ygg-host -- $PACK --out $IMAGE"

# ---------------------------------------------------------------- 前置：产物
command -v node >/dev/null 2>&1 || fail "缺 node（wasm 车道要 node ≥ 21：JSON.parse reviver 的 context.source）"
command -v cargo >/dev/null 2>&1 || fail "缺 cargo（原生侧由 --example golden 提供）"
[ -d "$PACK" ] || fail "包目录不存在：$PACK"
[ -f "$WASM_GLUE" ] && [ -f "$WASM_BIN" ] || fail "缺 wasm 产物：$WASM_GLUE / ${WASM_BIN}（先跑：${BUILD_WASM_CMD}）"
[ -f "$IMAGE" ] || fail "缺包镜像：${IMAGE}（先跑：${BUILD_IMAGE_CMD}）"

# 镜像与包源逐文件比 sha256（镜像里嵌着 manifest，是唯一能判定「镜像过期」的事实）。
if ! YGG_IMAGE="$IMAGE" YGG_PACK="$PACK" node --input-type=module -e '
import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import path from "node:path";
import { pathToFileURL } from "node:url";

const packDir = path.resolve(process.env.YGG_PACK);
const { IMAGE } = await import(pathToFileURL(path.resolve(process.env.YGG_IMAGE)).href);
const manifest = JSON.parse(IMAGE.manifest);
const listed = [...(manifest.code ?? []), ...(manifest.entries ?? [])];
const stale = [];
for (const item of listed) {
  let bytes;
  try {
    bytes = await readFile(path.join(packDir, item.path));
  } catch {
    stale.push(`${item.path}（缺失）`);
    continue;
  }
  if (createHash("sha256").update(bytes).digest("hex") !== item.sha256) {
    stale.push(item.path);
  }
}
if (stale.length > 0) {
  console.error(stale.join("、"));
  process.exit(1);
}
'; then
  fail "包镜像与 $PACK 的源文件不一致（上面列出的文件已改）→ 重跑：$BUILD_IMAGE_CMD"
fi

# wasm 产物是否早于它的源码：按 ygg-wasm 的 normal 依赖链现算（链变了自动跟上）。
# 这是粗判（只比 mtime，不比内容），方向是「宁可多报一次重建」。
# 路径取 `cargo tree --prefix none` 第三列（`(/abs/path)`），剥掉括号——BSD sed 的 BRE
# 分组在这里不可靠，awk 一行更直白。
stale_src=""
for crate in $(cargo tree -p ygg-wasm -e normal --prefix none 2>/dev/null \
  | awk '/^ygg/ { p = $3; gsub(/[()]/, "", p); if (p ~ /^\//) print p }' | sort -u); do
  # 只比会进二进制的源（tests/benches/examples 不进 lib，改一条用例不该要求重建 wasm）
  hit="$(find "$crate" \
    \( -name tests -o -name benches -o -name examples \) -prune -o \
    \( -name '*.rs' -o -name 'Cargo.toml' \) -newer "$WASM_BIN" -print -quit 2>/dev/null || true)"
  if [ -n "$hit" ]; then
    stale_src="$hit"
    break
  fi
done
if [ -n "$stale_src" ]; then
  fail "wasm 产物早于源码（${stale_src}）→ 重跑：$BUILD_WASM_CMD"
fi

# ---------------------------------------------------------------- 取数
now_ms() {
  if [ -n "${EPOCHREALTIME:-}" ]; then
    # bash ≥ 5：微秒时间戳，砍掉后三位取毫秒
    printf '%s' "$((${EPOCHREALTIME//[!0-9]/} / 1000))"
  elif command -v perl >/dev/null 2>&1; then
    # macOS 自带 bash 3.2 没有 EPOCHREALTIME；秒级 date 会把「wall 3000ms」抹成 0/1000
    perl -MTime::HiRes=time -e 'printf "%.0f", time * 1000'
  else
    printf '%s' "$(($(date +%s) * 1000))"
  fi
}

# `final_hash=<十进制> tick=<N> elapsed_ms=<x>` 一行的字段取值
field() {
  printf '%s\n' "$2" | sed -n "s/.*$1=\([0-9][0-9]*\).*/\1/p" | head -1
}

ok_count=0
bad_count=0
detail=""

for ticks in $TICKS; do
  case "$ticks" in
    '' | *[!0-9]*) fail "--ticks 只接受十进制正整数，收到：$ticks" ;;
  esac

  native_wall=$(now_ms)
  if ! native_out=$(cargo run --release -q --example golden -p ygg-sandbox -- "$PACK" --ticks "$ticks" --final-only); then
    die_run "原生侧 $ticks tick 跑挂（见上方 golden 输出）"
  fi
  native_wall=$(($(now_ms) - native_wall))

  wasm_wall=$(now_ms)
  if ! wasm_out=$(node scripts/wasm-lane.mjs --ticks "$ticks" --image "$IMAGE"); then
    die_run "wasm 侧 $ticks tick 跑挂（见上方车道输出）"
  fi
  wasm_wall=$(($(now_ms) - wasm_wall))

  native_hash=$(field final_hash "$native_out")
  wasm_hash=$(field final_hash "$wasm_out")
  native_ms=$(field elapsed_ms "$native_out")
  wasm_ms=$(field elapsed_ms "$wasm_out")
  if [ -z "$native_hash" ] || [ -z "$wasm_hash" ]; then
    die_run "取不到最终 hash（原生=${native_out:-空} wasm=${wasm_out:-空}）"
  fi

  if [ "$native_hash" = "$wasm_hash" ]; then
    ok_count=$((ok_count + 1))
    printf '[%s] ok  hash=%s  native=%sms wasm=%sms（wall %sms/%sms）\n' \
      "$ticks" "$native_hash" "$native_ms" "$wasm_ms" "$native_wall" "$wasm_wall"
  else
    bad_count=$((bad_count + 1))
    printf '[%s] MISMATCH native=%s wasm=%s\n' "$ticks" "$native_hash" "$wasm_hash"
    detail="${detail}  ${ticks} tick: native=${native_hash} wasm=${wasm_hash}\n"
  fi
done

if [ "$bad_count" -gt 0 ]; then
  echo "persist-matrix: ${bad_count} 档不一致（一致的 ${ok_count} 档）" >&2
  printf "%b" "$detail" >&2
  echo "persist-matrix: 先排除产物过期——wasm: ${BUILD_WASM_CMD}；镜像: $BUILD_IMAGE_CMD" >&2
  exit 1
fi

echo "persist-matrix: ${ok_count} 档全等（原生与 wasm 最终 state hash 逐档相同）"
