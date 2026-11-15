# Yggdrasill · Context 召回树（docs/context）
> **id** readme · **幕** all · **触发** 任何时刻 / 进树第一读
> **读不到代价** silent-drift（迷路后从记忆推进，是 §1.0c scene gate 的首要失效来源）
> **ver** 000000

**定位**：五幕（立局/协调/盘点/发布/退役）与需求级九幕的「进行时台词」知识权威树的**唯一入口**——场景叙事、阶段策略、每幕入口/必做/出口动作清单，以及**无 egg 环境的人工执行法**。mod 的 `scenes` / `prompts` / 六个 `<node>_common` 键是这棵树的运行时投影（状态前缀 + 指针）；语义以树内文件为准。

**replaces**：r2「mod 剧情提示词迁出」前散置于 `.ygg/mods/agent-seed/data/texts/core.json`（scenes/prompts）与 `beats-flow.json`（`*_common`）的幕叙事正文；以及 r1 之前平铺在 `docs/context/` 的 17 个文件（按「召回时点」重切成 27 份权威文件，D-MA27）。

**上游**：[../README.md](../README.md) · **相关**：[scenes.json](scenes.json)（场景 → 文档加载表，r2 消费） · [rituals/dispatch.md](rituals/dispatch.md) · [rituals/cleanup.md](rituals/cleanup.md) · [copy/README.md](copy/README.md)（七域索引） · [../design/run-story.md](../design/run-story.md)（微状态机契约，语义不重复讲述）

## 我在哪个时点 → 读哪个文件

| 时点 | 必读 | id |
| --- | --- | --- |
| 进程 boot | [acts/prologue.md](acts/prologue.md) + 冷启动词表 | `acts/prologue` · `cold-start/vocabulary` |
| 立局（写章程 / 开工） | [acts/charter.md](acts/charter.md) | `acts/charter` |
| 协调（每次派发前） | [acts/coordinate.md](acts/coordinate.md) + [req-acts/intake.md](req-acts/intake.md) | `acts/coordinate` · `req-acts/intake` |
| 派发 subagent 前 | [rituals/dispatch.md](rituals/dispatch.md) | `rituals/dispatch` |
| 需求 draft / planning / plan_gate | [req-acts/intake.md](req-acts/intake.md) | `req-acts/intake` |
| 需求 ready / developing / accept / accept_gate | [req-acts/delivery.md](req-acts/delivery.md) | `req-acts/delivery` |
| 需求 accept_gate / integrating / done | [req-acts/integration.md](req-acts/integration.md) | `req-acts/integration` |
| 盘点（批结算 / 退役前） | [acts/settle.md](acts/settle.md) + [rituals/cleanup.md](rituals/cleanup.md) | `acts/settle` · `rituals/cleanup` |
| 发布声明 | [acts/release.md](acts/release.md) + [acts/retired.md](acts/retired.md) | `acts/release` · `acts/retired` |
| 改 mod 文案 / 键面 | [copy/README.md](copy/README.md)（按域分流） | `copy/README` |
| 查裁决 / 取决策号 | [archive/decisions-engine.md](archive/decisions-engine.md) · [archive/decisions-agent.md](archive/decisions-agent.md) | `archive/*` |
| 无 egg / 无 mod | [rituals/fallback.md](rituals/fallback.md) | `rituals/fallback` |

## 目录树

```text
docs/context/
├── README.md            # 本文件：召回入口（时点 → 文件）
├── scenes.json          # 场景 / 需求阶段 → 文档 id 加载表（r2 消费）
├── cold-start/          # 冷启动底座（原样迁入）
│   ├── vision.md  vocabulary.md  topology.md  principles.md
├── acts/                # 主流程五幕 + 退役终态，1:1 一文件（每份 ≤40 行）
│   ├── prologue.md  charter.md  coordinate.md
│   ├── settle.md    release.md  retired.md
├── req-acts/            # 需求级九幕，按共享时点聚合三份（每份 ≤45 行）
│   ├── intake.md        # 1-3 立项 / 计划中 / 计划门禁
│   ├── delivery.md      # 4-6 待开发 / 开发中 / 交付受理（+ Doc sync、旁路）
│   └── integration.md   # 7-9 验收门禁 / 集成 / done（+ 证据清单、集成纪律）
├── rituals/             # 按动作时点的仪式与政策
│   ├── dispatch.md  cleanup.md  fallback.md
├── copy/                # mod 文案契约（七域索引 + 七域表）
│   ├── README.md  flow.md  harness.md  lanes.md
│   ├── map-guide.md  requirements.md  settlement.md  core.md
└── archive/             # 决策台账（D1-D21 / D-MA1-29）
    ├── decisions-engine.md  decisions-agent.md
```

## 需求九幕一览

| # | 幕 | 主人 | 一句话 |
| --- | --- | --- | --- |
| 1 | 立项 | 人 | 把需求写清楚（标题/描述/依赖） |
| 2 | 计划中 | 执行者 | 产出计划稿并提交 plan 提案 |
| 3 | 计划门禁 | **人** | 审计划：过或打回 |
| 4 | 待开发 | 资源 | 计划已过，等泳道槽位派发 |
| 5 | 开发中 | 执行者 | 实现并提交 development 提案 |
| 6 | 交付受理 | 主代理 | 串行审阅交付：受理或打回 |
| 7 | 验收门禁 | **人** | 过验收：核验 develop@<sha> 证据 |
| 8 | 集成 | 执行者 | 功能分支已并入 develop，确认 SHA |
| 9 | done | — | 终态，集成证据在册 |

## 读法

- 本树是**给人与 agent 读的执行底座**：每一幕「我在哪 / 说什么 / 做什么 / 下一步动作名」。
- mod 运行时在每幕开场输出场景行（`scenes.<act>`），在每幕内输出菜单与阶段提示（`prompts.<act>`、`<act>_common`）；这些键值只保留状态变量与短句，**叙事与策略正文以本树为唯一权威**（改文案先改本树再同步 mod，见 [../wiki-governance.md](../wiki-governance.md)）。
- 菜单措辞（core `options` 27 键）由 mod 运行时渲染；本树只在各幕「菜单语义对照」小节解释每个菜单项的语义，不复制键值（键值契约见 [copy/core.md](copy/core.md) options 节）。
- **每份文件头部 4 行是机读契约**：`id`（本文件在树内的唯一标识，同时也是签名 `read.<id>=<ver>` 的键）、`幕`、`触发`、`读不到代价`（四值域 `irreversible` / `hard-refusal` / `silent-drift` / `none`）、`ver`（内容指纹）。

## compat 层（旧路径兼容，**不是权威**）

r1 重切文件边界后，以下 **15 个旧路径**仍必须存在——因为冻结消费者（`.ygg/mods/agent-seed/**` 的 45 处指针、`crates/ygg-sandbox/tests/agent_seed_semantics.rs` 按路径读取的 7 个 copy 表）按旧路径取锚点，删掉即死链 / 测试转红。

| 旧路径 | 形态 | 指向 | 退场条件 |
| --- | --- | --- | --- |
| `workflow-acts.md` | 薄壳（指针壳） | `acts/` + `req-acts/` + `rituals/fallback.md` | r2 落地后由 r5 删除 |
| `dispatch-ritual.md` | 薄壳（指针壳） | `rituals/dispatch.md` + `req-acts/integration.md` + `rituals/fallback.md` | r2 落地后由 r5 删除 |
| `cleanup-policy.md` | 软链 | `rituals/cleanup.md` | r2 落地后由 r5 删除 |
| `decisions-engine.md` | 软链 | `archive/decisions-engine.md` | r2 落地后由 r5 删除 |
| `decisions-agent.md` | 软链 | `archive/decisions-agent.md` | r2 落地后由 r5 删除 |
| `topology.md` | 软链 | `cold-start/topology.md` | r2 落地后由 r5 删除 |
| `principles.md` | 软链 | `cold-start/principles.md` | r2 落地后由 r5 删除 |
| `copy-contract.md` | 软链 | `copy/README.md` | r2 落地后由 r5 删除 |
| `copy-contract-core.md` | 软链 | `copy/core.md` | r2 落地后由 r5 删除 |
| `copy-contract-flow.md` | 软链 | `copy/flow.md` | r2 落地后由 r5 删除 |
| `copy-contract-harness.md` | 软链 | `copy/harness.md` | r2 落地后由 r5 删除 |
| `copy-contract-lanes.md` | 软链 | `copy/lanes.md` | r2 落地后由 r5 删除 |
| `copy-contract-map-guide.md` | 软链 | `copy/map-guide.md` | r2 落地后由 r5 删除 |
| `copy-contract-requirements.md` | 软链 | `copy/requirements.md` | r2 落地后由 r5 删除 |
| `copy-contract-settlement.md` | 软链 | `copy/settlement.md` | r2 落地后由 r5 删除 |

- **13 软链 + 2 薄壳**：软链是 1:1 迁移后的同字节入口（`git hash-object` 与原文件一致）；薄壳是拆分型文件的「多目标指针」，正文不超过 10 行、不自带规则。
- 判据来源：D-MA29（兼容层两分法 + 必须带退场条件）。核验命令：`ls -l docs/context/*.md | grep -- '->' | wc -l` 应为 13；`bash scripts/docs-ver.sh --check` 应 `ok=1`。
- **已真删**（零冻结引用）：`vision.md`、`vocabulary.md` —— 权威在 `cold-start/` 下，改动方的引用已全部改写。

## 维护闭环

- `ver` 由 `scripts/docs-ver.sh` 生成/刷新（定宽占位归一化后取 `git hash-object` 前 6 位，D-MA29），**人不手改**；`--check` 只报告不写，并顺带做本树内的链接闭包。
- 场景加载表的唯一来源是 [scenes.json](scenes.json)；`--check` 会校验「表里的每个 id 都能解析到文件，且文件头 `id` 与表中值逐字相等」。
- 改任一文件后：`bash scripts/docs-ver.sh`（刷新）→ `bash scripts/docs-ver.sh --check`（`changed=0`）→ `bash .pi/tools/docs-consistency.sh`（`ok=1`）。
