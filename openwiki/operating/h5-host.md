---
type: "Reference"
title: "浏览器宿主 — hosts/h5 用法与验收"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T07:10:30.723Z
sources:
  - id: openwiki-source-aea4c7f6d9ed847b18b874d2
    resource: repo://artifacts/w2-s1-browser/evidence.txt
  - id: openwiki-source-c0d5856e80be2560c5caa5c8
    resource: repo://crates/ygg-host/src/image.rs
  - id: openwiki-source-d26ec1db7f712a4392f9eab5
    resource: repo://hosts/h5/main.js
  - id: openwiki-source-8e4bf269973ae240f415c5ef
    resource: repo://hosts/h5/pack.image.js
  - id: openwiki-source-5c7a7b397a55246dead72d32
    resource: repo://hosts/h5/render.js
  - id: openwiki-source-dfe0e9598ad418e75d354f01
    resource: repo://packs/example/golden.json
  - id: openwiki-source-c105d9c13b085511821f0969
    resource: repo://scripts/wasm-lane.mjs
generated: { by: "pi", at: "2026-11-26T07:10:30.723Z" }
---

# 浏览器宿主 — hosts/h5 用法与验收

> **id** `operating/h5-host` · **模块** 操作 · **触发** 想眼见为实、改绑定面后要验证、排查「页面白屏」
> **读不到代价** `silent-drift`（没有真宿主验收，wasm 绑定坏了也只有 native 测试在说绿）

`hosts/h5` 是一个**零构建**宿主：三个 ES module + 一个 canvas，没有打包器、没有 `node_modules`、没有 `npm run`。它证明「换宿主引擎不必改内核」。

## 1. 起服务

```bash
WASI_SDK_PATH="$HOME/.local/wasi-sdk-34.0-arm64-macos" bash scripts/build-wasm.sh   # 产出 crates/ygg-wasm/pkg/
cd /Users/zeki-leon/tmp/worktrees/yggdrasill && python3 -m http.server 8080
# 打开 http://127.0.0.1:8080/hosts/h5/index.html?seed=24301&trace=1
```

（`file://` 打不开：ES module 与 wasm 都要走 HTTP。）

## 2. 三个文件各自管什么

| 文件 | 职责 | 不做什么 |
| --- | --- | --- |
| `index.html` | 骨架：`#board` canvas、`#session-meta`、`#status`、`#trace-log`、Step / End turn / Snapshot / Open 四个按钮 | 不含逻辑 |
| `main.js` | 会话：写死 `PROTOCOL="ygg/1"`、`CODEC="json"`、`PACK_ID="ygg:example"`；读 `pack.image.js` 建会话；`?seed=`（默认 24301）；`?trace=1` 打开追踪；`decodeSnapshotEntities` 手解快照二进制；`applyDelta` 应用增量 | 不含规则 |
| `render.js` | 画：`CELL_SIZE=56`、`BOARD_PAD=12`，`decodePos(v)={x:floor(v/1000), y:v%1000}`，命中测试按 `canvas.width/rect.width` 缩放 | 不产生状态 |

**渲染边界**：宿主只做「把世界画出来」与「把点击变成命令」。它不解释规则、不预测结果、不缓存权威状态——每帧的真相来自 `events`/`delta` 与快照。

## 3. 四步验收（含预期值）

默认 `?seed=24301`，参考包 10×5，初始 10 个单位。

| 步 | 动作 | 预期 |
| --- | --- | --- |
| ① 装载 | 打开页面 | `tick 0 \| hash 6492099460516690635 \| 实体 10 \| seed 24301`，`status` 显示「就绪：tick 0，pack=ygg:example」 |
| ② 合法移动 | 点与 #1 相邻的空格（或方向键） | `status` 报 `move(#1, dx=1, dy=0) 已生效（tick 0）`；`hash` 变 `4206962874467499479`；`trace` 一行 `[move] tick=0 changed=1`（delta 恰 1 格：`pos`+`moved`） |
| ③ 非法命令 | 点**不相邻**的格子 | `status` 出现 `[E_RULE] E_STEP_REFUSED: move 被 Mod 拒绝：move：每次只能正交移动一格`；`tick` 与 `hash` 都不变（仍是 4206962874467499479） |
| ④ 快照恢复 | 干净会话连点 Step 两次 → `Snapshot` 下载 → 再 Step 两次 → `Open snapshot` 上传 | 先 `tick 2 \| hash 15782364623611949364`，推进到 `tick 4 \| hash 16727159050170007205`，恢复后回到 `tick 2 \| hash 15782364623611949364` |

再加一条确定性对照：干净会话 Step 三连应得到 `3894146741098379338` / `15782364623611949364` / `1588387159733934705`，与 `packs/example/golden.json` 的前三条**逐位相同**。

## 4. 常见故障

| 现象 | 原因 |
| --- | --- |
| 白屏，控制台 `Failed to resolve module specifier` | 用 `file://` 打开，或没先跑 `build-wasm.sh`（`crates/ygg-wasm/pkg/` 不存在） |
| `[E_PACK]` 握手失败 | `pack_hash` 与镜像不匹配——改了包却没重生成 `hosts/h5/pack.image.js` |
| `[E_PROTOCOL]` 立刻返回 | 帧里带了浮点或多字段（`deny_unknown_fields`）——宿主自己造的帧错了 |
| 移动后画面不动 | delta 未按 `(eid,cid)` 回写：`applyDelta` 只认本帧 `changed` 里的列 |
| 页面里 tick 与 hash 对不上 golden | 种子不同：页面默认 24301，`?seed=` 会改世界 |

## 5. 维护责任

- **触发**：改 `crates/ygg-host` / `crates/ygg-wasm` 的绑定面、改协议帧、改参考包（镜像与 golden 都要重生成）、改页面交互。
- **检查**：§3 四步 + `node scripts/wasm-lane.mjs`（同一套断言在无浏览器环境下跑）。
