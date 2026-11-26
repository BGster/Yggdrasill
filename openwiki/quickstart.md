---
type: "Reference"
title: "召回入口 — 这一刻该打开哪一页"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-c1b4b807feda7647ba638e19
    resource: repo://.ygg/mods/agent-seed/data/config.json
  - id: openwiki-source-9706fc11c20facdfc630a8e9
    resource: repo://.ygg/mods/agent-seed/src/domain/readset.lua
  - id: openwiki-source-da9af83703d953f139bc0632
    resource: repo://scripts/gen-scene-readfirst.py
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 召回入口 — 这一刻该打开哪一页

> **id** quickstart · **触发** 每次开工、每次换幕、每次不确定「规则在哪」时
> **读不到代价** silent-drift（凭记忆或凭某份摘要行动，而不是凭正文）
> **replaces**：`docs/context/README.md`（含其 `copy/` 旧路径）与 `docs/README.md` 的召回表（搬运基线 `3a2536f`）

**先读这一句**：本页是**路由**，不是规则。路由指向的正文才是规则；把摘要当正文，是这套知识库最常见的违规形态。

## 1. 按「这一刻」找页

| 这一刻 | 打开 |
| --- | --- |
| 完全新手，想知道这是什么 | [overview.md](overview.md) → [operating/cold-start/vision.md](operating/cold-start/vision.md) |
| 开工：开局 / 写章程 / 开工 | [operating/acts/prologue.md](operating/acts/prologue.md) → [charter.md](operating/acts/charter.md) |
| 协调：派发、受理、门禁、冲突、amend | [operating/acts/coordinate.md](operating/acts/coordinate.md) |
| 派一个执行者 / 写 brief / 对齐 base | [operating/rituals/dispatch.md](operating/rituals/dispatch.md) |
| 需求立项（第 1–3 幕） | [operating/req-acts/intake.md](operating/req-acts/intake.md) |
| 需求交付（第 4–6 幕） | [operating/req-acts/delivery.md](operating/req-acts/delivery.md) |
| 需求集成（第 7–9 幕） | [operating/req-acts/integration.md](operating/req-acts/integration.md) |
| 结算：验证、演进、会话、待办 | [operating/acts/settle.md](operating/acts/settle.md) · [rituals/cleanup.md](operating/rituals/cleanup.md) |
| 发布 / 退役 | [operating/acts/release.md](operating/acts/release.md) · [retired.md](operating/acts/retired.md) |
| 无 egg 环境怎么推进 | [operating/rituals/fallback.md](operating/rituals/fallback.md) |
| 不确定要不要签名、签什么 | [operating/rituals/read-gate.md](operating/rituals/read-gate.md) |
| 系统这一句回执是什么意思 | [operating/scenes/README.md](operating/scenes/README.md) |
| 改内核：状态 / 事件 / KV / API / 压缩 / WASM | [contracts/kernel/](contracts/kernel/state-machine.md)（先看 [architecture/README.md](architecture/README.md) 的触碰面表） |
| 改宿主：CLI / daemon / serve / 泳道 / 工具桥运行时 | [contracts/host/](contracts/host/cli-daemon.md) |
| 改 mod：pack.json / 文案 / 身份 / 门禁 | [contracts/mod/](contracts/mod/mod-package.md) |
| 写或改一个包 | [contracts/packages/README.md](contracts/packages/README.md) |
| 改知识库自己的规则 | [governance.md](governance.md) |
| 讲不清一整局 / 一条需求怎么走 | [operating/story/run-story.md](operating/story/run-story.md) · [requirement-story.md](operating/story/requirement-story.md) |

## 2. 必读集与理解性文档

必读集**只由 `scripts/gen-scene-readfirst.py` 的输入决定**，登记在 `.ygg/mods/agent-seed/data/config.json` 的 `readPlan` 里，生成自 `scenes.json`：

| 幕 / 阶段 | 必读 id |
| --- | --- |
| prologue | `cold-start/vocabulary` |
| charter | `acts/charter` |
| coordinate | `acts/coordinate`、`req-acts/intake` |
| settle | `acts/settle`、`rituals/cleanup` |
| release | `acts/release`、`acts/retired` |
| 需求阶段 | draft / planning / plan_gate → `req-acts/intake`；ready / developing / accept / accept_gate → `req-acts/delivery`；integrating / done → `req-acts/integration` |

**不参与门禁**的理解性文档（`reference`）：`story/run-story`、`story/requirement-story`、`cold-start/system-boundaries`、`rituals/read-gate`。它们**不进 `readPlan`、不出 `[READ FIRST]` 块、不产生签名义务**——登记 id 只为机检可达（链接不悬空）。

要纳入必读，就把 id 挪进 `scene` / `reqStage`：单点生效，生成器与门禁自动跟随。判据见 [governance.md](governance.md) §5。

## 3. 一条最短路径

1. [overview.md](overview.md) 的冷启动四条命令能跑通（`scripts/install-egg.sh` → `egg new` → `charter.set` → `run.begin`）。
2. 读 [operating/acts/prologue.md](operating/acts/prologue.md)，把章程三件套写实。
3. 进协调幕前读 [coordinate.md](operating/acts/coordinate.md) + [intake.md](operating/req-acts/intake.md)，并**签名**（见 [read-gate.md](operating/rituals/read-gate.md)）。
4. 派活时读 [dispatch.md](operating/rituals/dispatch.md)；收活时读 [delivery.md](operating/req-acts/delivery.md)。
5. 批结算读 [settle.md](operating/acts/settle.md)，然后 [release.md](operating/acts/release.md)。

## 4. 判据

① 阻断——用本页当规则执行、从摘要或上一轮记忆推进、跳过签名、把 `reference` 页当成必读。② 症状——`[READ FIRST]` 的路径打不开、签名 id 拼错、同一问题在两页得到不同答案。③ 确认——本页每行链接可解析、必读集与 `config.json` 的 `readPlan` 一致、`reference` 行不在 `[READ FIRST]` 里出现。

维护责任 = 加页 / 改必读集的人；触发 = 新增幕或阶段、页面改名、`scenes.json` 增删 id；检查 = `scripts/gen-scene-readfirst.py --check` 无漂移、本页链接全部可解析。
