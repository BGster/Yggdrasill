# Yggdrasill · Docs Wiki 顶层索引

> **定位**：本文件是 Yggdrasill 仓库 LLM Wiki 的唯一顶层导航——每主题一文件、双向链接、始终最新（活文档，非流水账：变更就地改写对应主题文件，历史归 git 与结算产物）。
> **replaces**：原 `docs/CONTEXT.md` / `docs/DESIGN.md` 两单文件的章节总览职责（r2 拆分，两文件现降级为薄壳索引）。
> **维护规则**：见 [wiki-governance.md](wiki-governance.md)（注入 / 同步 / 核验三职责）。

## 文档树

```text
docs/
├── README.md            # 本文件：顶层索引（地图 + 导航 + 阅读路径）
├── wiki-governance.md   # 活文档闭环：执行前注入 / 执行中同步 / 结算前核验
├── CONTEXT.md           # 薄壳索引：领域/决策树入口 + 旧 §N 映射表
├── DESIGN.md            # 薄壳索引：系统设计树入口 + 旧 §N 映射表
├── context/             # 领域语境树（按「召回时点」切分，入口 context/README.md）
│   ├── README.md            # 召回入口：我在哪个时点 → 读哪个文件（含九幕一览 + compat 层清单）
│   ├── scenes.json          # 场景 / 需求阶段 → 文档 id 加载表（r2 消费）
│   ├── cold-start/          # 冷启动底座：vision / vocabulary / topology / principles / system-boundaries
│   ├── story/               # 行为叙事（理解性，不拦门禁）：run-story / requirement-story
│   ├── acts/                # 主流程幕 1:1 一文件：prologue / charter / coordinate / settle / release / retired
│   ├── req-acts/            # 需求级九幕聚合三份：intake(1-3) / delivery(4-6) / integration(7-9)
│   ├── rituals/             # 按动作时点：dispatch / cleanup / fallback / read-gate
│   ├── copy/                # 文案契约：README + flow / harness / lanes / map-guide / requirements / settlement / core
└── design/              # 设计契约树（有代码消费者的「改前必守」）——四模块，入口 design/README.md
    ├── README.md              # 树入口：模块 → 文件 → 召回时点（索引完备性由机检断言）
    ├── kernel/                # ① ygg 内核（7）：state-machine / events（39 口径）/ state-surface / wire-protocol / lua-api / compaction / sim-kernel
    ├── host/                  # ② egg CLI 与 host（4）：cli-daemon / serve / harness-tools / lanes
    ├── mod/                   # ③ agent-seed mod（7）：mod-package / prompt-surface / tool-bridge / action-faces / identity / testing-strategy / superagent-game
    └── packages/              # ⑤ packages 通用能力层（9）：README + entity / codec / quest / dialogue / role-skill / procgen / assetgen / textframe
```

> **两棵树的判据**：`design/` = 改之前必须遵守的契约（**有代码消费者**）；`context/` = 我要理解/执行这个工作流（**无代码消费者**）。判定三步与反例见 [wiki-governance.md](wiki-governance.md)「design / context 二分判据」。

## 导航表 · 领域语境（context/）

| 文件 | 一句话定位 | replaces（原 CONTEXT.md） |
| --- | --- | --- |
| [context/README.md](context/README.md) | 召回入口：时点 → 文件（含九幕一览 + compat 层清单） | 原平铺 17 文件的唯一入口 |
| [context/scenes.json](context/scenes.json) | 场景 / 需求阶段 → 文档 id 加载表（r2 消费） | — |
| [context/cold-start/vision.md](context/cold-start/vision.md) | 统一命题与双重产品身份 | §1 |
| [context/cold-start/vocabulary.md](context/cold-start/vocabulary.md) | 术语速查表 + 12 条权威领域词汇定义 | 术语速览 + §2 |
| [context/cold-start/principles.md](context/cold-start/principles.md) | P1–P11 工程原则与 PLAY 家族注记 | §3 |
| [context/cold-start/topology.md](context/cold-start/topology.md) | 仓库目录拓扑与 5 条硬性依赖不变量 | §4 |
| [context/cold-start/system-boundaries.md](context/cold-start/system-boundaries.md) | 产品形态、三层架构图、协商矩阵（Tier 1–3）、Adapter 五义务 C1–C5 | §1 |
| [context/story/run-story.md](context/story/run-story.md) | 运行级剧情：微状态表、开场序列、盘点子微状态、发布幕 | §2.3 |
| [context/story/requirement-story.md](context/story/requirement-story.md) | 需求九幕、转场表、旁路/覆盖层 | §2.4 |
| [context/rituals/read-gate.md](context/rituals/read-gate.md) | 推进必签的判定：通读 vs 版本核对、窗口、四维拒台词 | — |
| [context/acts/](context/acts/) | 主流程幕 1:1 一文件（prologue / charter / coordinate / settle / release / retired，每份 ≤40 行） | mod scenes/prompts/`*_common` 叙事正文 |
| [context/req-acts/](context/req-acts/) | 需求级九幕聚合三份（intake 1-3 / delivery 4-6 / integration 7-9，每份 ≤45 行） | 同上（需求级台词与动作清单） |
| [context/rituals/dispatch.md](context/rituals/dispatch.md) | 派发仪式正文：分阶段必读 / 工件 / submit / 集成 / Doc sync | mod subagent_brief 策略段 |
| [context/rituals/cleanup.md](context/rituals/cleanup.md) | 盘点清理政策：会话清理边界与判据 / 待办清理 | mod settlement_sessions_unsetted_hint 正文 |
| [context/rituals/fallback.md](context/rituals/fallback.md) | 无 egg 执行章 + 续作降级说明 | mod subagent_fallback_note 正文 |
| [context/scenes/README.md](context/scenes/README.md) | 剧本索引：七个回执片段与「每一刻读哪份政策」；不复制回执模板（唯一副本在 mod `data/texts/`） | mod data/texts 全部回执键 |

## 导航表 · 设计契约（design/）

四模块，入口 [design/README.md](design/README.md)（模块 → 文件 → 召回时点）。判据：**有代码消费者**。

### ① kernel · ygg 内核

| 文件 | 一句话定位 | replaces（原 DESIGN.md） |
| --- | --- | --- |
| [state-machine.md](design/kernel/state-machine.md) | Mod 纯度、双层真状态压缩、微状态判定单一来源、响应式推进 | §2 章首 + §2.1/2.2/2.5/2.9 |
| [state-surface.md](design/kernel/state-surface.md) | 状态存储键契约与拒绝面矩阵 | §2.7 + §2.8 |
| [events.md](design/kernel/events.md) | 39 事件三分类与 payload 落点（设计目标全集口径，r1 +lane.pool） | §2.6 |
| [wire-protocol.md](design/kernel/wire-protocol.md) | 逐事件 payload 格式与尾段链 | §3.3 |
| [lua-api.md](design/kernel/lua-api.md) | ygg.* 全 API 面与副作用类别 | §3 章题 + §3.1 |
| [compaction.md](design/kernel/compaction.md) | 压缩双入口、水位线、稳定检查点谓词 | §3.6 |
| [sim-kernel.md](design/kernel/sim-kernel.md) | 三层引擎 Tier 1：确定性 tick/ECS、快照回放 state hash、热核面下沉铁律 | 全文（新） |

### ② host · egg CLI 与 host

| 文件 | 一句话定位 | replaces（原 DESIGN.md） |
| --- | --- | --- |
| [cli-daemon.md](design/host/cli-daemon.md) | egg CLI、builtin 工具、daemon 协议、summary Schema | §3.5 |
| [serve.md](design/host/serve.md) | serve 观察面全契约（CLI/面板/API/SSE/安全） | §3.7 |
| [harness-tools.md](design/host/harness-tools.md) | .pi/tools 检测脚本契约、注册面与无 egg 参数约定 | harness 机证 |
| [lanes.md](design/host/lanes.md) | 泳道池认领/释放/分支命名与实时投影 | §2.10 |

### ③ mod · agent-seed mod

| 文件 | 一句话定位 | replaces（原 DESIGN.md） |
| --- | --- | --- |
| [mod-package.md](design/mod/mod-package.md) | pack.json / config.json / manifest.json 契约 | §3.2 |
| [prompt-surface.md](design/mod/prompt-surface.md) | 6 个 beats 片段 × 417 键 + core 36 键职责表 | §2.12 |
| [tool-bridge.md](design/mod/tool-bridge.md) | 同步工具桥双层白名单指针 | §2.11 |
| [action-faces.md](design/mod/action-faces.md) | event/op/action 分层、I1–I3、A1–A4、registry 末注 | §3.4 |
| [identity.md](design/mod/identity.md) | 台账/身份章/profile/票据门（不进剧情） | §2.13 |
| [testing-strategy.md](design/mod/testing-strategy.md) | Mod 测试策略与门禁清单 | — |

### ⑤ packages · 通用能力层

| 文件 | 一句话定位 | replaces |
| --- | --- | --- |
| [packages/README.md](design/packages/README.md) | 八包契约入口：判据三问、包清单与状态、依赖序、模板与命名规则 | — |
| [entity.md](design/packages/entity.md) | 实体与属性袋（schema 校验、存活索引、生命周期事件） | — |
| [codec.md](design/packages/codec.md) | 序列化与 KV 形状（存档往返、sha256） | — |
| [quest.md](design/packages/quest.md) | 任务流转（stage 状态机、依赖与交付） | — |
| [dialogue.md](design/packages/dialogue.md) | 对话调度（节点图、文案单副本） | — |
| [role-skill.md](design/packages/role-skill.md) | 角色与技能（属性袋组合、事件计数冷却） | — |
| [procgen.md](design/packages/procgen.md) | 程序化生成（确定性种子、地形与布局） | — |
| [assetgen.md](design/packages/assetgen.md) | 素材生成（纹理 / 几何 / 音频，seed 驱动字节一致） | — |
| [textframe.md](design/packages/textframe.md) | 帧文本（字符网格 → 分层合成 → 叙事行） | — |

## 导航表 · 横切

| 文件 | 一句话定位 | replaces |
| --- | --- | --- |
| [wiki-governance.md](wiki-governance.md) | 活文档闭环三职责 + **design / context 二分判据**（有/无代码消费者 + 判定三步 + 反例） | DESIGN §4 |
| [CONTEXT.md](CONTEXT.md) | 领域/决策树薄壳入口（旧 §N 解析映射） | 原 CONTEXT.md 本体 |
| [DESIGN.md](DESIGN.md) | 系统设计树薄壳入口（旧 §N 解析映射） | 原 DESIGN.md 本体 |

## 阅读路径

- **新执行者（plan/development 前）**：[CONTEXT.md](CONTEXT.md)（壳）→ [context/README.md](context/README.md)（召回入口：时点 → 文件）→ [context/cold-start/vocabulary.md](context/cold-start/vocabulary.md) → 当前幕对应的 `context/acts/<act>.md` 与 `context/req-acts/<stage>.md` → [context/rituals/dispatch.md](context/rituals/dispatch.md) → 本需求触碰的子系统契约文件（入口 [design/README.md](design/README.md)，按 kernel / host / mod / packages 四模块取；见 brief 的 `Mandatory references:` 行；plan 对照 playbook run_machine/actions，development 再对照 gates/live_evidence）。
- **决策检索**：没有决策台账也没有编号——逐条裁决都写在它治理的那份文件的「设计依据」段（规范 + 依据 + 三问判据）。从 [design/README.md](design/README.md)（触碰面 → 文件）或 [wiki-governance.md](wiki-governance.md)（治理法则）进。
- **门禁核验**：[requirement-story.md](context/story/requirement-story.md)（九幕/转场）+ [state-surface.md](design/kernel/state-surface.md)（拒绝面）+ [context/req-acts/integration.md](context/req-acts/integration.md)（集成分工）。
- **CLI/契约/serve 开发**：[cli-daemon.md](design/host/cli-daemon.md)、[action-faces.md](design/mod/action-faces.md)、[wire-protocol.md](design/kernel/wire-protocol.md)、[serve.md](design/host/serve.md)。

## 旧式引用解析

旧文档与历史回执中的 `§N` 引用经两个薄壳的映射表解析（如 "DESIGN §2.10" → [lanes.md](design/host/lanes.md)；"DESIGN §1 / §2.3 / §2.4" → [context/cold-start/system-boundaries.md](context/cold-start/system-boundaries.md) / [context/story/run-story.md](context/story/run-story.md) / [context/story/requirement-story.md](context/story/requirement-story.md)）：r6 之后映射表是**跨树**的，`design/` 只收有代码消费者的契约面。`docs/CONTEXT.md` / `docs/DESIGN.md` 路径本身持续有效（薄壳即入口）。git 历史引用（如 自动收波（已废弃） double-claim 证据）指向两单文件的历史版本，不受拆分影响。
