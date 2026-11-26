---
type: "Reference"
title: "构建与门禁 — 工具链、命令清单与 CI"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T17:33:41.429Z
sources:
  - id: openwiki-source-e05e3253cc972e8bd548b5cf
    resource: repo://.github/workflows/check.yml
  - id: openwiki-source-ea70eb6c045047448e446296
    resource: repo://.gitignore
  - id: openwiki-source-f4f3c3407ada59508fa211cb
    resource: repo://crates/ygg-sandbox/examples/selftest.rs
  - id: openwiki-source-8e4bf269973ae240f415c5ef
    resource: repo://hosts/h5/pack.image.js
  - id: openwiki-source-6a5d7b301b84949a383f444c
    resource: repo://scripts/build-wasm.sh
  - id: openwiki-source-b0d308427c9d0c9eb125b7ed
    resource: repo://scripts/persist-matrix.sh
generated: { by: "pi", at: "2026-11-26T07:10:30.723Z" }
---

# 构建与门禁 — 工具链、命令清单与 CI

> **id** `operating/build` · **模块** 操作 · **触发** 首次克隆、改了代码要交付、CI 红而本地绿
> **读不到代价** `hard-refusal`（漏一条门禁就提交 → CI 红、或 worse：wasm 产物坏了但没人发现）

## 1. 工具链

| 需要 | 用途 | 怎么装 / 在哪 |
| --- | --- | --- |
| Rust stable | 全部 crate | `rustup` 默认即可；`cargo fmt`/`clippy` 需显式 `rustup component add rustfmt clippy` |
| `wasm32-unknown-unknown` target | 浏览器绑定 | `rustup target add wasm32-unknown-unknown` |
| WASI sysroot | `mlua`（vendored Lua）编 wasm 时的 C 链接 | `WASI_SDK_PATH=$HOME/.local/wasi-sdk-34.0-arm64-macos`（路径按机器调整） |
| `wasm-bindgen` CLI | 生成 JS 胶水（**必须 `--reference-types`**） | `cargo install wasm-bindgen-cli --version <与 Cargo.lock 一致>` |
| `wasm-opt`（可选） | 产物瘦身 | Binaryen；缺失时脚本跳过 |
| Node ≥ 20 | wasm 冒烟泳道 | `node scripts/wasm-lane.mjs` |
| Python 3 | 起静态服务器 + 一致性脚本 | `python3 -m http.server 8080` |

`wasm-pack` **不可用**：它不透传 `--reference-types`，而 `mlua` 的异常处理模块要求这个标志。产物一律走 `scripts/build-wasm.sh`。

## 2. 六条门禁（本地等价于 CI）

```bash
cargo fmt --all -- --check                                    # ① 格式
cargo clippy --workspace --all-targets -- -D warnings          # ② 静态检查（警告即错误）
cargo test --workspace                                         # ③ 全量测试
cargo run -q --example selftest -p ygg-sandbox -- --mod packs/example                       # ④ 包自测（加法 --filter <子串>）
cargo run -q --example golden   -p ygg-sandbox -- packs/example --check packs/example/golden.json # ⑤ 确定性金值
cargo run -q --example image    -p ygg-host    -- packs/example --check hosts/h5/pack.image.js    # ⑥ 镜像同步
```

失败时的定位线索：

| 门禁 | 红了先看什么 |
| --- | --- |
| ① fmt | 直接 `cargo fmt --all` 后再 diff，不要手调 |
| ② clippy | `-D warnings` 常抓到「未用变量」「可简化的 match」；`#[allow]` 需写理由 |
| ③ test | 名字就是契约名（如 `tampered_hash_is_located_at_its_frame`），按名字读用例 |
| ④ selftest | 失败行给出用例文件与用例名；`--filter <子串>` 缩范围 |
| ⑤ golden | 说明确定性被破坏：先比 `git diff` 是否改了 Lua 或哈希面，再决定是「预期变更」还是「回归」 |
| ⑥ image | 只是产物过期：跑不带 `--check` 的同一条命令重生成，**确认是预期变更**再提交 |

## 3. wasm 构建与泳道

```bash
WASI_SDK_PATH="$HOME/.local/wasi-sdk-34.0-arm64-macos" bash scripts/build-wasm.sh   # 产出 crates/ygg-wasm/pkg/ + 自动冒烟
node scripts/wasm-lane.mjs                                                          # 50 tick 逐帧 + replay 对比 golden
node scripts/wasm-lane.mjs --ticks 100000                                           # 长跑（约 4 秒）
bash scripts/persist-matrix.sh                                                      # 原生 / wasm / 墙钟 三档同 hash
```

`build-wasm.sh` 做四件事：`cargo build --target wasm32…` → `wasm-bindgen --target web --reference-types` → 把裸 `wasi_snapshot_preview1` 导入重写成 `../wasi-shim.js` 并注入内存绑定 → 可选 `wasm-opt -Oz`，最后跑冒烟。

## 4. 什么进 git，什么不进

| 产物 | 是否入库 | 理由 |
| --- | --- | --- |
| `crates/ygg-wasm/pkg/`（wasm + 胶水） | **否**（`.gitignore`） | 二进制，随工具链浮动；页面 import 它，本地构建即可 |
| `crates/ygg-wasm/wasi-shim.js` | 是 | 手写 shim，不是产物 |
| `hosts/h5/pack.image.js` | **是** | 是包镜像的确定性派生物，`--check` 门禁靠它在 git 里比对 |
| `packs/example/golden.json` | 是 | 确定性基线，改动必须是有意的 |
| `target/` | 否 | — |

## 5. CI 覆盖了什么、仍然不覆盖什么

`.github/workflows/check.yml` 两个 job：

| job | 跑什么 |
| --- | --- |
| `rust` | `cargo fmt --check`、`cargo clippy -D warnings`、`cargo test --workspace`；文档闭包（`wiki-ver --check` 与 `wiki-consistency` 双 `ok=1`）；参考包三件套（`selftest`、`golden --check`、`image --check`）；一条结构守卫（`serde_json` 不得开 `preserve_order` —— 键序即线格式） |
| `wasm` | 装 WASI SDK 33.0 与 `wasm-bindgen-cli`（版本对齐 `Cargo.lock`）→ `scripts/build-wasm.sh` → `node scripts/wasm-lane.mjs`（50 tick 逐位 == `golden.json` + replay 全等） |

**CI 仍然不跑**：真实浏览器泳道（`hosts/h5` 要人眼与人手）、10 万 tick 的 `scripts/persist-matrix.sh`（成本高，本地跑）、GPU/音频类宿主侧行为。这三样必须在本地跑——「CI 绿」不等于可交付。

## 6. 维护责任

- **触发**：加/删 crate、改工具链版本、改 `scripts/build-wasm.sh`、改 CI 步骤、改门禁清单。
- **检查**：`bash scripts/wiki-ver.sh --check; bash scripts/wiki-consistency.sh`（双 `ok=1`），再加本地 `node scripts/wasm-lane.mjs` 与浏览器四步（CI 的 `wasm` job 跑前者，不跑后者）。
