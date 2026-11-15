# Yggdrasill · Context 召回树（docs/context）
> **id** readme · **幕** all · **触发** 任何时刻 / 进树第一读
> **读不到代价** silent-drift（迷路后从记忆推进，是 §1.0c scene gate 的首要失效来源）
> **ver** 2375a1

> **定位**：五幕（立局/协调/盘点/发布/退役）的「进行时台词」知识权威——场景叙事、阶段策略、每幕入口/必做/出口动作清单，以及**无 egg 环境的人工执行法**。mod 的 `scenes` / `prompts` / 六个 `<node>_common` 键是本文的运行时投影（状态前缀 + 指针）；语义以本文为准。本文件是这棵树的召回入口（时点 → 文件见下表）。
> **replaces**：r2「mod 剧情提示词迁出」前散置于 `.ygg/mods/agent-seed/data/texts/core.json`（scenes/prompts）与 `beats-flow.json`（`*_common`）的幕叙事正文。
> **上游**：[../README.md](../README.md) · **相关**：[rituals/dispatch.md](rituals/dispatch.md) · [rituals/cleanup.md](rituals/cleanup.md) · [copy/README.md](copy/README.md)（七域索引） · [story/run-story.md](story/run-story.md)（微状态机契约，语义不重复讲述）
> **判据**：本树收的是**无代码消费者的行为面**；`docs/design/` 收的是有代码消费者的契约面——二分判据见 [../wiki-governance.md](../wiki-governance.md)。

## 我在哪个时点 → 读哪个文件

### 必读（拦门禁：进 `[READ FIRST]` 块，须 `config.set read.<id>=<ver>` 签名）

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

### 理解性阅读（**不拦门禁**）

这三份**没有代码消费者**，按 [../wiki-governance.md](../wiki-governance.md) 的「design / context 二分判据」归本树。
它们登记在 [scenes.json](scenes.json) 的 `reference` 段，**不进 `readPlan`**、不出 `[READ FIRST]` 块、不产生签名义务
（负向断言见 `crates/ygg-sandbox/tests/agent_seed_semantics.rs` 的 `context_scene_reference_ids_resolve`）。

| 时点 | 读 | id |
| --- | --- | --- |
| 理解「一整局怎么跑」/ 改运行级转场 | [story/run-story.md](story/run-story.md) | `story/run-story` |
| 理解「一条需求走九幕」/ 改需求级转场与门禁判定 | [story/requirement-story.md](story/requirement-story.md) | `story/requirement-story` |
| 冷启动 / 「系统由哪几层构成」/ 改三层边界与 Adapter 义务 | [cold-start/system-boundaries.md](cold-start/system-boundaries.md) | `cold-start/system-boundaries` |

## 目录树

```text
docs/context/
├── README.md            # 本文件：召回入口（时点 → 文件）
├── scenes.json          # 场景 / 需求阶段 → 文档 id 加载表（r2 消费）
├── cold-start/          # 冷启动底座（原样迁入）
│   ├── vision.md  vocabulary.md  topology.md  principles.md
│   └── system-boundaries.md   # 三层架构与产品定位（r6 从 design 归位）
├── story/               # 行为叙事（理解性，不拦门禁）
│   ├── run-story.md            # 运行级剧情（r6 从 design 归位）
│   └── requirement-story.md    # 需求级九幕（r6 从 design 归位）
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

## 兼容层已退役（r5 完成，2026-11-15；**本节仅为历史追溯**）

r1 重切文件边界时曾保留 **15 个旧路径**（13 软链 + 2 薄壳）给当时的冻结消费者：
`.ygg/mods/agent-seed/**` 的文案指针与 `crates/ygg-sandbox/tests/agent_seed_semantics.rs` 按路径读取的 copy 表。
r5 已把两侧引用全部改指权威文件，兼容层随之删除——**现在树里只有权威文件，没有第二入口**。

> **下表是历史记录，不是入口也不是迁移指引**：这些旧路径**已不存在**，任何按表复活的尝试都会被指针机检判红
> （`design/pointer` 锚点用例 + `design_tree_index_is_complete_and_ids_match_paths`）。要查当时的理由看
> `archive/decisions-*.md` 与 git 历史，不要照表改路径。

| 已删除的旧路径 | 形态 | 现在的权威 |
| --- | --- | --- |
| `workflow-acts.md` | 薄壳（多目标指针） | `acts/` + `req-acts/` + `rituals/fallback.md` |
| `dispatch-ritual.md` | 薄壳（多目标指针） | `rituals/dispatch.md` + `req-acts/integration.md` + `req-acts/delivery.md` + `rituals/fallback.md` |
| `cleanup-policy.md` | 软链 | `rituals/cleanup.md` |
| `decisions-engine.md` / `decisions-agent.md` | 软链 | `archive/decisions-engine.md` / `archive/decisions-agent.md` |
| `topology.md` / `principles.md` | 软链 | `cold-start/topology.md` / `cold-start/principles.md` |
| `copy-contract.md` | 软链 | `copy/README.md` |
| `copy-contract-{core,flow,harness,lanes,map-guide,requirements,settlement}.md` | 软链 | `copy/{core,flow,harness,lanes,map-guide,requirements,settlement}.md` |

- **判据来源**：D-MA29（兼容层两分法 + 必须带退场条件）；退场条件在 r5 兑现。
- **为什么能删**：两类冻结消费者都已改指——mod 的 23 个散文键指针（25 处路径）改指子目录权威文件，
  crates 的 7 个 copy 表读取路径改指 `docs/context/copy/*.md`；锚点用例（`narrative_pointer_anchors_exist_in_docs_context`）
  现在真的能看见子目录指针（r5 修好了只认 `[A-Za-z0-9._-]` 的旧解析器），所以「没死链」是被查出来的，不是被假设的。
- **核验命令**：`ls -l docs/context/*.md | grep -- '->' | wc -l` 应为 **0**；
  `bash scripts/docs-ver.sh --check` 应 `ok=1`；`bash .pi/tools/docs-consistency.sh --repo .` 应 `ok=1`。
- **已真删**（更早一轮）：`vision.md`、`vocabulary.md` —— 权威在 `cold-start/` 下，引用已全部改写。
- **r6 新增归位（不是兼容层，是搬家）**：`docs/design/` 的 `run-story` / `requirement-story` / `system-boundaries`
  三份因**无代码消费者**（判据见 [../wiki-governance.md](../wiki-governance.md)）移居本树
  （`story/` 与 `cold-start/`）；它们在 design 的旧路径同批退役且**不留软链**。
- 旧路径不再作为可执行指针存在；历史与当时理由只在 git 与 `archive/decisions-*.md` 里。

## 维护闭环

- `ver` 由 `scripts/docs-ver.sh` 生成/刷新（定宽占位归一化后取 `git hash-object` 前 6 位，D-MA29），**人不手改**；`--check` 只报告不写，并顺带做本树内的链接闭包。
- 场景加载表的唯一来源是 [scenes.json](scenes.json)；`--check` 会校验「表里的每个 id 都能解析到文件，且文件头 `id` 与表中值逐字相等」。
- 改任一文件后：`bash scripts/docs-ver.sh`（刷新）→ `bash scripts/docs-ver.sh --check`（`changed=0`）→ `bash .pi/tools/docs-consistency.sh`（`ok=1`）。
