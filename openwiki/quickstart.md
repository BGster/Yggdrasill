---
type: "Reference"
title: "五分钟跑通 — 构建、自测、金值与双端一致"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T07:10:30.723Z
sources:
  - id: openwiki-source-a07e566f3712cbd5931877e8
    resource: repo://crates/ygg-host/examples/image.rs
  - id: openwiki-source-22643d64c5d6dbbb15838db2
    resource: repo://crates/ygg-sandbox/examples/golden.rs
  - id: openwiki-source-ce6aebf2c3323e209b2938c2
    resource: repo://crates/ygg-sandbox/src/host_api/world/install.rs
  - id: openwiki-source-b9c1316bad48fa3bf2b0a7b3
    resource: repo://hosts/h5/index.html
  - id: openwiki-source-dfe0e9598ad418e75d354f01
    resource: repo://packs/example/golden.json
  - id: openwiki-source-2c5400885f18104f706a3405
    resource: repo://packs/example/pack.json
  - id: openwiki-source-6a5d7b301b84949a383f444c
    resource: repo://scripts/build-wasm.sh
  - id: openwiki-source-da9af83703d953f139bc0632
    resource: repo://scripts/gen-scene-readfirst.py
  - id: openwiki-source-b0d308427c9d0c9eb125b7ed
    resource: repo://scripts/persist-matrix.sh
  - id: openwiki-source-c105d9c13b085511821f0969
    resource: repo://scripts/wasm-lane.mjs
generated: { by: "pi", at: "2026-11-26T07:10:30.723Z" }
---

# 五分钟跑通 — 构建、自测、金值与双端一致

> **id** `quickstart` · **模块** 中枢 · **触发** 第一次 clone、怀疑「本机环境坏了」、改了内核要挑最省的一条验证
> **读不到代价** `hard-refusal`（命令与预期输出对不上就分不清是环境问题还是代码问题）

前置：Rust（stable，`wasm32-unknown-unknown` target）、Node ≥ 20、Python 3；跑 wasm 需要 `WASI_SDK_PATH=$HOME/.local/wasi-sdk-34.0-arm64-macos`。

## 1. 一次性构建 + 全量测试

```bash
cargo build --workspace
cargo test --workspace
```

预期：`test result: ok` 逐套出现，共 **63 套**；`crates/ygg-sandbox` 与 `crates/ygg-host` 占大多数。

## 2. 参考包自测（最快的一条回归）

```bash
cargo run -q --example selftest -p ygg-sandbox -- --mod packs/example
```

末尾：

```
---
cases passed=12 failed=0
ok=1
```

## 3. 金值（逐 tick 哈希）

```bash
cargo run -q --example golden -p ygg-sandbox -- packs/example --check packs/example/golden.json
# ok: packs/example/golden.json (seed=24301, ticks=50)
```

对不上时先看 `--seed`：缺省 `0x5eed`，金值文件是 `seed=24301`。

## 4. 镜像（宿主装载源）

```bash
cargo run -q --example image -p ygg-host -- packs/example --out hosts/h5/pack.image.js --check
# ok: hosts/h5/pack.image.js (20920 bytes)
```

`--check` 必须与 `--out` 联用；改了 `pack.json` 或任一 Lua/数据文件都会在这里红，因为 `pack_hash` 是 manifest 文本的 sha256。

## 5. 双端一致（wasm 与原生）

```bash
WASI_SDK_PATH="$HOME/.local/wasi-sdk-34.0-arm64-macos" bash scripts/build-wasm.sh
node scripts/wasm-lane.mjs
```

末尾：

```
  tick 49: 12893861340166594567 == golden
  tick 50: 3898440229879777255 == golden
ok: wasm 逐 tick (50/50) + replay 全等 packs/example/golden.json
```

长跑用矩阵脚本（三档 1k / 10k / 100k，只读，可反复跑）：

```bash
bash scripts/persist-matrix.sh
# 100k 最终 hash 15126981819641904924（native 2668ms / wasm 3764ms）
```

退出码 2 表示 wasm 产物过期——先重跑 `build-wasm.sh`，不要先怀疑内核。

## 6. 浏览器里玩一下

```bash
cd "$(git rev-parse --show-toplevel)" && python3 -m http.server 8080
# http://127.0.0.1:8080/hosts/h5/index.html?seed=24301
```

预期与四步验收见 `operating/h5-host`（装载 `hash 6492099460516690635`）。

## 7. 写你自己的包（最小路径）

```
mypack/
  pack.json        # {"schemaVersion":2,"id":"me:game","version":"0.1.0","capabilities":["world"],"entry":"src/main.lua"}
  src/main.lua     # 声明组件 → 造世界 → 注册系统 → world.seal() → world.commands=… → return {commands=…}
  data/*.json      # 可选，经 ygg.data 读
```

1. `world.component{name="hp",kind="i64"}` 声明列；
2. `world.spawn{comps={hp=10}}` 造实体；
3. `world.system{name="tick_hp",stage="rules",reads={},writes={"hp"},run=function(ctx) ... end}`；
4. `world.seal()`（此后注册即 `YGG-SIM-SEALED`）；
5. 入口返回 `{ commands = { ... } }`；
6. `cargo run -q --example selftest -p ygg-sandbox -- --mod mypack` 跑你的 `test/*.lua`。

照抄的对象是 `packs/example`（逐文件讲解见 `operating/example-pack`）。

## 8. 出问题先看哪儿

| 症状 | 先看 |
| --- | --- |
| `E_PACK` 握手失败 | `pack_hash` 与镜像不匹配 → 步骤 4 |
| `E_STEP_BUDGET` | 某次 Lua 入口超 200 万指令 → `contracts/kernel/sandbox` §4 |
| `E_STORE_FORMAT` | 存档格式与本仓不一致（v1 只认 `FMT_VERSION` 当前值） |
| wasm 与原生不一致 | `persist-matrix.sh` 退出码是否为 2（产物过期） |
| 页面白屏 | 没跑 `build-wasm.sh`，或用 `file://` 打开 |

## 9. 维护责任

- **触发**：改任一命令的产物路径/参数、改 crate 数量、改默认种子。
- **检查**：本页 §1–§6 的命令逐条可复制执行且输出与上文一致。
