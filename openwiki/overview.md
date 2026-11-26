---
type: "Reference"
title: "Yggdrasill 知识库"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-8339725b1afcf7133cd86688
    resource: repo://.ygg/mods/agent-seed/pack.json
  - id: openwiki-source-651d1fb6c9e49916a916ab51
    resource: repo://Cargo.toml
  - id: openwiki-source-8774eec48fcb6d9c1cb2563e
    resource: repo://crates/ygg-core/src/event.rs
  - id: openwiki-source-683c84ca41d2dbee8643a1d8
    resource: repo://crates/ygg-lint/src/rules.rs
  - id: openwiki-source-a4a7569ab993da998f5988fd
    resource: repo://crates/ygg-sandbox/examples/egg/main.rs
  - id: openwiki-source-b09b7c233d20c123d81292ab
    resource: repo://crates/ygg-sandbox/examples/serve/roots.rs
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

> **id** overview · **触发** 每次新会话开工、不确定「这是什么仓库」时
> **读不到代价** silent-drift（把地图当规则、或按记忆讲系统分层）

# Yggdrasill 知识库

**一句话**：Yggdrasill 是一个用 Lua 写游戏、用事件把游戏推进成可玩状态机、并让 Agent 在同一个契约面里「自己造世界」的 harness。仓库同时是**引擎**和**它自己的第一个玩家**——两边共用一套 `ygg.*` 面。

## 1. 三个系统，一套契约

| 系统 | 是什么 | 代码位置 | 契约页 |
| --- | --- | --- | --- |
| **内核（kernel）** | 状态、事件、KV、快照、压缩、WASM、规则检查；只认注册面，不认游戏语义 | `crates/ygg-{core,schema,modules,sandbox,wasm,store,lint,sim}` | [architecture/README.md](architecture/README.md) → `contracts/kernel/` |
| **宿主（host）** | 把内核装进可用产品：`egg` CLI、daemon、`serve` 观察面、泳道池、工具桥 | `crates/ygg-sandbox/examples/egg/`、`crates/ygg-store` | `contracts/host/` |
| **Mod（agent-seed）** | 一局的所有规则与文案：剧本、门禁、派发、结算、身份、Proposal | `.ygg/mods/agent-seed/` | `contracts/mod/` |
| **包（packages）** | 可复用的 Lua 能力（实体、对话、任务、程序化、资产、编解码、文本帧）与示例 mod | `packages/libs/`、`packages/mods/` | `contracts/packages/` |

三层之间的关系是**单向依赖**：mod 与包只经 `ygg.*` 面触内核，内核零游戏语义；宿主只装配与转发，不改判语义。这条边界不是风格问题，是可在代码里指认的接缝——见 [architecture/README.md](architecture/README.md)。

## 2. 冷启动

```bash
# 构建 + 安装（幂等；macOS 上脚本会自己 ad-hoc 签名，否则首次执行被内核杀成 exit 137）
scripts/install-egg.sh

# 开局：--mod 指 mod 目录，--in 指 run 根，event.start 发首事件；输出末行 run-root: <path>
ROOT="$(.ygg/bin/egg new --mod .ygg/mods/agent-seed --in .ygg/runs --event event.start | tail -1 | sed 's/^run-root: //')"
A=".ygg/bin/egg open $ROOT"
$A --event charter.set --payload '"<标题>|<目标>|<成功判据>"'
$A --event run.begin

# 观察（只读；127.0.0.1:7800；空根启动，POST /api/roots 注册根）
.ygg/bin/egg serve
curl -X POST http://127.0.0.1:7800/api/roots -H 'Content-Type: application/json' -d '{"root": "<repo>/.ygg/runs"}'

# 推进：交互驱动（人选题），或逐事件直发
$A --drive stdin
$A --event map.show          # 操作视图 + 全部章节索引，payload 忽略
$A --event config.set --payload '"play.mode=goal"'   # 仅在用户授权后
```

**一次进程只发一个事件**（`--event` 单次）；没有、也不会有 `--events` 批量入口、时间线重放脚本或结算循环。读完回执再决定下一步。暂停 = 输入 `q` 或结束 stdin；非法输出、超时、`HUMAN[:reason]` 都会安全暂停而不自动选 1。

## 3. 仓库地图

```text
crates/            内核与宿主（Rust；8 个 workspace crate）
crates/ygg-sandbox/examples/egg/   egg CLI：new / open / serve / daemon / verify …
packages/libs/     可复用 Lua 包（八包，各有契约页与 selftest）
packages/mods/     示例与测试 mod（shapez / superagent / tla-models / test-adventure / pkg-selftest）
packages/schema/   JSON Schema 四件（manifest / agent-config / scene-texts / scene-texts-fragment）
.ygg/mods/agent-seed/   Agent harness 本体：剧本数据、门禁、文案、测试
.ygg/runs/         run 产出（不进 git）
scripts/           安装、契约同步、场景 readPlan 生成、版本账本、文档闭包
openwiki/          本知识库（生成物：页面 + claims；不手改生成页）
```

## 4. 权威顺序

**源码与测试 > `openwiki/contracts/` > `openwiki/operating/` > 仓库根 `AGENTS.md` > 其余一切**（历史计划稿、会话记录、旧文档快照）。一句话：行为以代码为准，契约页解释代码，操作页约束做法，`AGENTS.md` 只留冷启动与不可逆红线。详见 [governance.md](governance.md) §1。

## 5. 完整页面地图

### 中枢

| 页面 | 用途 |
| --- | --- |
| [overview.md](overview.md) | 本页：身份、三系统、冷启动、地图 |
| [quickstart.md](quickstart.md) | 按「这一刻」找页：召回入口 |
| [governance.md](governance.md) | 知识库自己的宪法：边界、`ver` 账本、签名、批结算维护 |

### 契约（`contracts/`，解释「系统是什么」）

| 分组 | 页面 |
| --- | --- |
| 索引 | [architecture/README.md](architecture/README.md) — 触碰面 → 契约页 |
| kernel | [state-machine](contracts/kernel/state-machine.md) · [state-surface](contracts/kernel/state-surface.md) · [events](contracts/kernel/events.md) · [wire-protocol](contracts/kernel/wire-protocol.md) · [lua-api](contracts/kernel/lua-api.md) · [compaction](contracts/kernel/compaction.md) · [sim-kernel](contracts/kernel/sim-kernel.md) |
| host | [cli-daemon](contracts/host/cli-daemon.md) · [serve](contracts/host/serve.md) · [lanes](contracts/host/lanes.md) · [rt-face](contracts/host/rt-face.md) · [harness-tools](contracts/host/harness-tools.md) |
| mod | [mod-package](contracts/mod/mod-package.md) · [prompt-surface](contracts/mod/prompt-surface.md) · [tool-bridge](contracts/mod/tool-bridge.md) · [action-faces](contracts/mod/action-faces.md) · [identity](contracts/mod/identity.md) · [testing-strategy](contracts/mod/testing-strategy.md) · [tla-models](contracts/mod/tla-models.md) · [superagent-game](contracts/mod/superagent-game.md) · [shapez-game](contracts/mod/shapez-game.md) |
| packages | [索引](contracts/packages/README.md) · [entity](contracts/packages/entity.md) · [dialogue](contracts/packages/dialogue.md) · [quest](contracts/packages/quest.md) · [procgen](contracts/packages/procgen.md) · [assetgen](contracts/packages/assetgen.md) · [codec](contracts/packages/codec.md) · [textframe](contracts/packages/textframe.md) · [role-skill](contracts/packages/role-skill.md) |

### 操作（`operating/`，约束「人怎么做事」）

| 分组 | 页面 |
| --- | --- |
| 幕 | [prologue](operating/acts/prologue.md) · [charter](operating/acts/charter.md) · [coordinate](operating/acts/coordinate.md) · [settle](operating/acts/settle.md) · [release](operating/acts/release.md) · [retired](operating/acts/retired.md) |
| 需求阶段 | [intake](operating/req-acts/intake.md) · [delivery](operating/req-acts/delivery.md) · [integration](operating/req-acts/integration.md) |
| 仪式 | [dispatch](operating/rituals/dispatch.md) · [read-gate](operating/rituals/read-gate.md) · [cleanup](operating/rituals/cleanup.md) · [fallback](operating/rituals/fallback.md) |
| 冷启动展开 | [vision](operating/cold-start/vision.md) · [vocabulary](operating/cold-start/vocabulary.md) · [principles](operating/cold-start/principles.md) · [topology](operating/cold-start/topology.md) · [system-boundaries](operating/cold-start/system-boundaries.md) |
| 剧情 | [run-story](operating/story/run-story.md) · [requirement-story](operating/story/requirement-story.md) |
| 文案域 | [scenes/README](operating/scenes/README.md) — 回执键索引（正文在 `data/texts/`） |

## 6. 旧 `§N` 引用怎么解析

历史文档与回执里的 `§N` 按下表解析；映射是**跨树**的（`§1`/`§2.3`/`§2.4` 与 `§4` 落在 `operating/`，其余落在 `contracts/`）。

| 旧引用 | 现在读 |
| --- | --- |
| 术语速览 / §2 | [operating/cold-start/vocabulary.md](operating/cold-start/vocabulary.md) |
| §1（产品形态与三层架构） | [operating/cold-start/system-boundaries.md](operating/cold-start/system-boundaries.md) |
| §2 章首 + §2.1/2.2/2.5/2.9 | [contracts/kernel/state-machine.md](contracts/kernel/state-machine.md) |
| §2.3 / §2.4 | [operating/story/run-story.md](operating/story/run-story.md) / [operating/story/requirement-story.md](operating/story/requirement-story.md) |
| §2.6 / §2.7 + §2.8 / §2.10 | [events](contracts/kernel/events.md) / [state-surface](contracts/kernel/state-surface.md) / [lanes](contracts/host/lanes.md) |
| §2.11 / §2.12 / §2.13 | [tool-bridge](contracts/mod/tool-bridge.md) / [prompt-surface](contracts/mod/prompt-surface.md) / [identity](contracts/mod/identity.md) |
| §3 章题 + §3.1 / §3.2 / §3.3 / §3.4 / §3.5 / §3.6 | [lua-api](contracts/kernel/lua-api.md) / [mod-package](contracts/mod/mod-package.md) / [wire-protocol](contracts/kernel/wire-protocol.md) / [action-faces](contracts/mod/action-faces.md) / [cli-daemon](contracts/host/cli-daemon.md) / [compaction](contracts/kernel/compaction.md) |
| §3.7 | [contracts/host/serve.md](contracts/host/serve.md) |
| §4 | [contracts/packages/](contracts/packages/README.md)（八包各一页） |
| DESIGN §4（文档二分） | [governance.md](governance.md) |

## 7. 判据

① 阻断——把本页当权威（它只是地图）、按旧 `§N` 猜内容而不查映射、在 `contracts/` 与 `operating/` 之间放第三棵树。② 症状——同一主题两页各写一半、地图里的链接失效、有人拿 `AGENTS.md` 当规则全文。③ 确认——`operating/scenes/README.md` 与 `data/texts/` 七片段一一对应、`architecture/README.md` 的触碰面表与 `contracts/` 实文件相等、`scripts/wiki-consistency.sh` 为绿。

维护责任 = 加页或改页结构的人；触发 = 新增 / 合并页面、`§N` 映射变化、冷启动命令变化；检查 = 地图分组与 `openwiki/` 实目录相等，示例命令在本机可复现。
