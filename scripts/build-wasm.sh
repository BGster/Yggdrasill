#!/usr/bin/env bash
# B1 wasm 产物构建链（单一入口）：cargo build → wasm-bindgen（--reference-types）。
#
# 为什么不是 `wasm-pack build`：mlua wasm32 链路带 wasm 异常（wasi-libc EH 变体
# setjmp/longjmp），wasm-bindgen 0.2.129 对含 EH 指令的模块生成 catch wrappers
# 时强制要求 externref 表，该表由其 CLI 旗 `--reference-types` 开启的 externref
# pass 创建；wasm-pack 0.15 不透传 bindgen 旗——故为两步链（cargo 出裸 wasm，
# 再调 wasm-bindgen CLI）。AGENTS.md §五 的 wasm-pack 门禁命令以本脚本替代
# （决策待回填 AGENTS.md）。
#
# 前置：
#   1. rustup target add wasm32-unknown-unknown
#   2. WASI SDK（mlua-rs/mlua docs/wasm32 上游安装指引，版本 34 实证）：
#      - 约定路径 /opt/wasi-sdk，或
#      - 任意路径 + 导出 WASI_SDK_PATH，例如 Apple Silicon：
#          WASI_SDK_VERSION=34
#          curl -fL "https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-${WASI_SDK_VERSION}/wasi-sdk-${WASI_SDK_VERSION}.0-arm64-macos.tar.gz" | tar -xz -C "$HOME/.local"
#          export WASI_SDK_PATH="$HOME/.local/wasi-sdk-${WASI_SDK_VERSION}.0-arm64-macos"
#   3. wasm-bindgen CLI（版本必须与 Cargo.lock 的 wasm-bindgen crate 一致）：
#      cargo install wasm-bindgen-cli@<版本>，或复用 wasm-pack 缓存
#      （$HOME/Library/Caches/.wasm-pack/wasm-bindgen-cargo-install-<版本>/）。
#
# 产物：crates/ygg-wasm/pkg/（ygg_wasm.js 胶水 + ygg_wasm_bg.wasm + .d.ts）。
# 可选后处理：PATH 上有 wasm-opt（binaryen ≥118）时自动 -Oz（EH 兼容实证与
#   环境变量开关见下方 wasm-opt 段注释）。
# 活链验证：
#   node crates/ygg-wasm/smoke.mjs                # B1 活链（资源包 + 叙事 + 存读档）
#   node crates/ygg-wasm/verify-determinism.mjs   # FR-9 wasm 侧双跑哈希等价（M1 C2）
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# ---- WASI SDK 与交叉编译环境 --------------------------------------------
# 缺省旗面已固化于 .cargo/config.toml（CFLAGS_wasm32_*：--target=wasm32-wasip1
# + luai_makeseed；[target.wasm32-unknown-unknown] rustflags：-Lnative 静态库
# 目录）。此处只覆盖**路径相关**项（CC/CXX/AR 与 -Lnative）——本机 SDK 不在
# /opt/wasi-sdk 时（config 缺省路径不存在），经 WASI_SDK_PATH 重导出即为终值
# （同名覆盖 config 的 force=false [env] 缺省；RUSTFLAGS 旗面与 config 取并集，
# 不存在的 /opt 路径无害）。
WASI_SDK_PATH="${WASI_SDK_PATH:-/opt/wasi-sdk}"
if [ ! -x "$WASI_SDK_PATH/bin/clang" ]; then
  echo "error: WASI SDK not found at $WASI_SDK_PATH" >&2
  echo "       install to /opt/wasi-sdk, or: export WASI_SDK_PATH=<path> (see header comment)" >&2
  exit 1
fi
export CC_wasm32_unknown_unknown="${CC_wasm32_unknown_unknown:-$WASI_SDK_PATH/bin/clang}"
export CXX_wasm32_unknown_unknown="${CXX_wasm32_unknown_unknown:-$WASI_SDK_PATH/bin/clang++}"
export AR_wasm32_unknown_unknown="${AR_wasm32_unknown_unknown:-$WASI_SDK_PATH/bin/llvm-ar}"
export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS="${CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS:--Ctarget-feature=+reference-types -Lnative=$WASI_SDK_PATH/share/wasi-sysroot/lib/wasm32-wasip1/eh -Lnative=$WASI_SDK_PATH/share/wasi-sysroot/lib/wasm32-wasip1 -lstatic=c++abi -lstatic=unwind -lstatic=wasi-emulated-process-clocks -lstatic=c}"


# ---- wasm-bindgen CLI（版本对齐 Cargo.lock 的 wasm-bindgen crate）---------
BINDGEN_CRATE_VERSION="$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | sed -n 's/^version = "\(.*\)"$/\1/p' | head -1)"
[ -n "$BINDGEN_CRATE_VERSION" ] || { echo "error: wasm-bindgen version not found in Cargo.lock" >&2; exit 1; }

BINDGEN_CLI=""
for candidate in \
  "$(command -v wasm-bindgen || true)" \
  "$HOME/Library/Caches/.wasm-pack/wasm-bindgen-cargo-install-$BINDGEN_CRATE_VERSION/wasm-bindgen" \
  "$HOME/.cache/.wasm-pack/wasm-bindgen-cargo-install-$BINDGEN_CRATE_VERSION/wasm-bindgen"; do
  if [ -n "$candidate" ] && [ -x "$candidate" ]; then BINDGEN_CLI="$candidate"; break; fi
done
if [ -z "$BINDGEN_CLI" ]; then
  echo "error: wasm-bindgen CLI $BINDGEN_CRATE_VERSION not found" >&2
  echo "       cargo install wasm-bindgen-cli@$BINDGEN_CRATE_VERSION" >&2
  exit 1
fi


echo "==> [1/2] cargo build --release --target wasm32-unknown-unknown -p ygg-wasm"
cargo build --release --target wasm32-unknown-unknown -p ygg-wasm
WASM="target/wasm32-unknown-unknown/release/ygg_wasm.wasm"
[ -f "$WASM" ] || { echo "error: $WASM not produced" >&2; exit 1; }

# ---- wasm-bindgen CLI（版本对齐 Cargo.lock 的 wasm-bindgen crate）---------
CLI_VERSION="$("$BINDGEN_CLI" --version | awk '{print $2}')"
if [ "$CLI_VERSION" != "$BINDGEN_CRATE_VERSION" ]; then
  echo "error: wasm-bindgen CLI ($CLI_VERSION) != crate version ($BINDGEN_CRATE_VERSION)" >&2
  echo "       crate 与 CLI 版本必须一致（wasm-bindgen schema version 门）" >&2
  exit 1
fi

echo "==> [2/2] wasm-bindgen $CLI_VERSION --target web --reference-types"
rm -rf crates/ygg-wasm/pkg
"$BINDGEN_CLI" "$WASM" \
  --out-dir crates/ygg-wasm/pkg \
  --typescript \
  --target web \
  --reference-types

# ---- 胶水 WASI 导入重写 ---------------------------------------------------
# --reference-types 的 externref pass 把全部 wasi_snapshot_preview1 导入升为
# 胶水级**静态 ESM 导入**（bare specifier，浏览器/node 均无法直接解析）。重写
# 为相对路径 ../wasi-shim.js（wasi-shim.js 以命名导出承载全部函数，namespace
# import 即可被 WebAssembly 按属性解析）——产物 pkg/ + wasi-shim.js 即自洽
# 消费集，浏览器/Vite/node 零 import map、零 node_modules 桩。重写模式由
# wasm-bindgen 输出格式决定（版本已锁 Cargo.lock），构建后校验兜底。
perl -pi -e 's{from "wasi_snapshot_preview1"}{from "../wasi-shim.js"}g' crates/ygg-wasm/pkg/ygg_wasm.js
if grep -q 'from "wasi_snapshot_preview1"' crates/ygg-wasm/pkg/ygg_wasm.js; then
  echo "error: glue rewrite failed: bare wasi_snapshot_preview1 import remains" >&2
  exit 1
fi

# ---- wasm-opt 后处理（可选；binaryen ≥118 才能优化本 EH 模块）------------
# M1 C2 兼容性实证（EH 模块 = wasi-libc setjmp/longjmp 变体，产物含 try_table/throw）：
#   1. wasm-bindgen（walrus 重写）剥掉 LLVM target-features 元数据，wasm-opt
#      无法自动探测特性，必须显式开旗（旗面 = `--print-features` 实测集，
#      缺任一即校验失败："…requires [--enable-…]" / "all used types should
#      be allowed"——实测缺 tail-call/multivalue 时报后者）。
#   2. binaryen 117（wasm-pack 缓存版）：校验通过（-O0 往返成功），但任何
#      优化级别必崩：`unimplemented DCE control flow structure`
#      （DeadCodeElimination.cpp:179）。
#   3. binaryen 133（brew）：-Oz/-O2 成功（本模块 3,060,338 → 2,734,045 字节，
#      -10.6%），优化产物过 smoke + verify-determinism 活链，golden 哈希不变。
# 管线取 -Oz；YGG_WASM_OPT=off 关闭，=on 令失败致命（默认 auto：找到即用，
# 失败降级为未优化产物并告警，不阻断构建）。
YGG_WASM_OPT="${YGG_WASM_OPT:-auto}"   # auto | on | off
if [ "$YGG_WASM_OPT" != "off" ]; then
  WASM_OPT_BIN="$(command -v wasm-opt || true)"
  if [ -z "$WASM_OPT_BIN" ]; then
    if [ "$YGG_WASM_OPT" = "on" ]; then
      echo "error: YGG_WASM_OPT=on but wasm-opt not on PATH (brew install binaryen)" >&2
      exit 1
    fi
    echo "==> [opt] wasm-opt not on PATH — skipping (YGG_WASM_OPT=auto; brew install binaryen to enable)"
  else
    # shellcheck disable=SC2086 # OPT_FLAGS 旗面有意按词拆分。
    OPT_FLAGS="--enable-mutable-globals --enable-nontrapping-float-to-int --enable-simd --enable-bulk-memory --enable-sign-ext --enable-exception-handling --enable-tail-call --enable-reference-types --enable-multivalue --enable-gc"
    BEFORE_BYTES="$(wc -c < crates/ygg-wasm/pkg/ygg_wasm_bg.wasm | tr -d ' ')"
    echo "==> [opt] $WASM_OPT_BIN -Oz on EH module (before=${BEFORE_BYTES}B)"
    if "$WASM_OPT_BIN" crates/ygg-wasm/pkg/ygg_wasm_bg.wasm $OPT_FLAGS -Oz \
        -o crates/ygg-wasm/pkg/ygg_wasm_bg.opt.wasm; then
      mv crates/ygg-wasm/pkg/ygg_wasm_bg.opt.wasm crates/ygg-wasm/pkg/ygg_wasm_bg.wasm
      AFTER_BYTES="$(wc -c < crates/ygg-wasm/pkg/ygg_wasm_bg.wasm | tr -d ' ')"
      echo "    after=${AFTER_BYTES}B  saved=$((BEFORE_BYTES - AFTER_BYTES))B ($(( (BEFORE_BYTES - AFTER_BYTES) * 100 / BEFORE_BYTES ))%)"
    elif [ "$YGG_WASM_OPT" = "on" ]; then
      rm -f crates/ygg-wasm/pkg/ygg_wasm_bg.opt.wasm
      echo "error: wasm-opt failed on the EH module (binaryen must be >=118; see wasm-opt section above)" >&2
      exit 1
    else
      rm -f crates/ygg-wasm/pkg/ygg_wasm_bg.opt.wasm
      echo "warning: wasm-opt failed — keeping unoptimized module (binaryen >=118 required; YGG_WASM_OPT=on makes this fatal)" >&2
    fi
  fi
fi

ls -la crates/ygg-wasm/pkg/
echo "==> done. live-chain checks: node crates/ygg-wasm/smoke.mjs && node crates/ygg-wasm/verify-determinism.mjs"


