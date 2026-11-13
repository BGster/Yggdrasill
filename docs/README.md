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
├── context/             # 领域语境树
│   ├── vision.md            # 产品愿景与双重身份
│   ├── vocabulary.md        # 术语速览与核心领域词汇
│   ├── principles.md        # 架构与运营原则 P1–P11
│   ├── topology.md          # Monorepo 拓扑与依赖白名单
│   ├── decisions-engine.md  # 决策台账 D1–D21
│   ├── decisions-agent.md   # 决策台账 D-MA1–23
│   ├── workflow-acts.md     # 五幕进行时台词 + 无 egg 执行章（r2 新增）
│   ├── dispatch-ritual.md   # 派发仪式正文（r2 新增）
│   ├── cleanup-policy.md    # 盘点清理政策（r2 新增）
│   └── copy-contract.md     # mod 文案契约：413 键句式表（r2 新增，知识权威源）
└── design/              # 系统设计树
    ├── system-boundaries.md     # 三层边界 + 协商矩阵 + Adapter 五义务
    ├── state-machine.md         # 状态机语义与微状态判定
    ├── run-story.md             # 运行级剧情（live）
    ├── requirement-story.md     # 需求级剧情（open）
    ├── events.md                # 事件总表（38 口径）
    ├── state-surface.md         # 状态面：存储键契约 + 拒绝面矩阵
    ├── lanes.md                 # 持久化泳道池与进度投影
    ├── tool-bridge.md           # 同步工具桥
    ├── prompt-surface.md        # beats 提示词面
    ├── identity.md              # 身份与权限
    ├── lua-api.md               # ygg.* 宿主 API
    ├── mod-package.md           # Mod 包契约
    ├── wire-protocol.md         # 事件 payload 线协议
    ├── action-faces.md          # event/op/action 三层动作面
    ├── cli-daemon.md            # CLI 与 daemon 线协议
    ├── compaction.md            # 压缩与检查点交接
    └── serve.md                 # serve 观察面
```

## 导航表 · 领域语境（context/）

| 文件 | 一句话定位 | replaces（原 CONTEXT.md） |
| --- | --- | --- |
| [vision.md](context/vision.md) | 统一命题与双重产品身份 | §1 |
| [vocabulary.md](context/vocabulary.md) | 术语速查表 + 12 条权威领域词汇定义 | 术语速览 + §2 |
| [principles.md](context/principles.md) | P1–P11 工程原则与 PLAY 家族注记 | §3 |
| [topology.md](context/topology.md) | 仓库目录拓扑与 4 条硬性依赖不变量 | §4 |
| [decisions-engine.md](context/decisions-engine.md) | 引擎与平台架构决策台账 D1–D21 | §5 引言 + §5.1 |
| [decisions-agent.md](context/decisions-agent.md) | Agent 治理架构决策台账 D-MA1–24 | §5 引言 + §5.2 |
| [workflow-acts.md](context/workflow-acts.md) | 五幕进行时台词 + 入口/必做/出口 + 无 egg 执行章（r2 新增） | mod scenes/prompts/`*_common` 叙事正文 |
| [dispatch-ritual.md](context/dispatch-ritual.md) | 派发仪式正文：必读纪律 / 工件 / submit / 集成 / Doc sync（r2 新增） | mod subagent_brief 策略段 |
| [cleanup-policy.md](context/cleanup-policy.md) | 盘点清理政策：会话清理边界与判据 / 待办清理（r2 新增） | mod settlement_sessions_unsetted_hint 正文 |
| [copy-contract.md](context/copy-contract.md) | mod 文案契约：413 键句式模板 + 协议前缀归属（r2 新增，知识权威源） | mod data/texts 全部状态行文键 |

## 导航表 · 系统设计（design/）

| 文件 | 一句话定位 | replaces（原 DESIGN.md） |
| --- | --- | --- |
| [system-boundaries.md](design/system-boundaries.md) | 三层架构图、协商矩阵（Tier 1–3）、Adapter 五义务 C1–C5 | §1 |
| [state-machine.md](design/state-machine.md) | Mod 纯度、双层真状态压缩、微状态判定单一来源、响应式推进 | §2 章首 + §2.1/2.2/2.5/2.9 |
| [run-story.md](design/run-story.md) | 运行级剧情：微状态表、盘点子微状态、发布幕 | §2.3 |
| [requirement-story.md](design/requirement-story.md) | 需求九幕、转场表、旁路/覆盖层 | §2.4 |
| [events.md](design/events.md) | 39 事件三分类与 payload 落点（设计目标全集口径，r1 +lane.pool） | §2.6 |
| [state-surface.md](design/state-surface.md) | 状态存储键契约与拒绝面矩阵 | §2.7 + §2.8 |
| [lanes.md](design/lanes.md) | 泳道池认领/释放/分支命名与实时投影 | §2.10 |
| [tool-bridge.md](design/tool-bridge.md) | 同步工具桥双层白名单指针 | §2.11 |
| [prompt-surface.md](design/prompt-surface.md) | 7 片段 × 378 beats 键职责表 | §2.12 |
| [identity.md](design/identity.md) | 台账/身份章/profile/票据门（不进剧情） | §2.13 |
| [lua-api.md](design/lua-api.md) | ygg.* 全 API 面与副作用类别 | §3 章题 + §3.1 |
| [mod-package.md](design/mod-package.md) | pack.json / config.json / manifest.json 契约 | §3.2 |
| [wire-protocol.md](design/wire-protocol.md) | 逐事件 payload 格式与尾段链 | §3.3 |
| [action-faces.md](design/action-faces.md) | event/op/action 分层、I1–I3、A1–A4、registry 末注 | §3.4 |
| [cli-daemon.md](design/cli-daemon.md) | egg CLI、builtin 工具、daemon 协议、summary Schema | §3.5 |
| [compaction.md](design/compaction.md) | 压缩双入口、水位线、稳定检查点谓词 | §3.6 |
| [serve.md](design/serve.md) | serve 观察面全契约（CLI/面板/API/SSE/安全） | §3.7 |

## 导航表 · 横切

| 文件 | 一句话定位 | replaces |
| --- | --- | --- |
| [wiki-governance.md](wiki-governance.md) | 活文档闭环三职责 | DESIGN §4 |
| [CONTEXT.md](CONTEXT.md) | 领域/决策树薄壳入口（旧 §N 解析映射） | 原 CONTEXT.md 本体 |
| [DESIGN.md](DESIGN.md) | 系统设计树薄壳入口（旧 §N 解析映射） | 原 DESIGN.md 本体 |

## 阅读路径

- **新执行者（plan/development 前）**：[CONTEXT.md](CONTEXT.md)（壳）→ [vocabulary.md](context/vocabulary.md) → [principles.md](context/principles.md) → 本需求触碰的子系统主题文件（见 brief 的 `Mandatory references:` 行）。
- **决策检索**：D1–D21 → [decisions-engine.md](context/decisions-engine.md)；D-MAx → [decisions-agent.md](context/decisions-agent.md)。
- **门禁核验**：[requirement-story.md](design/requirement-story.md)（九幕/转场）+ [state-surface.md](design/state-surface.md)（拒绝面）+ [decisions-agent.md](context/decisions-agent.md)（D-MA22 集成职责）。
- **CLI/契约/serve 开发**：[cli-daemon.md](design/cli-daemon.md)、[action-faces.md](design/action-faces.md)、[wire-protocol.md](design/wire-protocol.md)、[serve.md](design/serve.md)。

## 旧式引用解析

旧文档与历史回执中的 `§N` 引用经两个薄壳的映射表解析（如 "DESIGN §2.10" → [lanes.md](design/lanes.md)）；`docs/CONTEXT.md` / `docs/DESIGN.md` 路径本身持续有效（薄壳即入口）。git 历史引用（如 D-MA16 double-claim 证据）指向两单文件的历史版本，不受拆分影响。
