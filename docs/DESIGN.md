# Yggdrasill · 系统架构、行为语义与接口契约（薄壳索引）

> **定位**：`docs/design/*` 树的入口薄壳——原单文件已按 r2 拆分为 LLM Wiki，本文件保留定位职责并承载旧式 `§N` 引用的解析映射。
> **replaces**：原 DESIGN.md 全文（L1–739 @ `3a2536f`），逐节映射见下表。
> **上游**：[README.md](README.md) · **相关**：[CONTEXT.md](CONTEXT.md)（领域/决策薄壳）

**定位（载体改写，语义保留）**：本树规定 Yggdrasill **始终最新的产品形态、系统行为、状态机语义与规范接口契约**——当前设计方案权威快照，不是流水账：变更就在对应主题文件就地改写，历史归 git 与结算产物。覆盖范围是**整个仓库**，按子系统分文件：内核（[lua-api.md](design/lua-api.md)）、Mod 包（[mod-package.md](design/mod-package.md)）、宿主 CLI/Daemon（[cli-daemon.md](design/cli-daemon.md)）、serve 观察面（[serve.md](design/serve.md)）等各有其章，修改哪个子系统就更新哪个文件。领域词汇、愿景、架构原则与决策台账（`D1–D21` 与 `D-MA` 系列台账，上界随台账顺延不写死）见 [CONTEXT.md](CONTEXT.md) 薄壳所引 `docs/context/*` 树。为使契约在实现重构下保持持久，其定义保持在边界层（**JSON Schema、CLI/JSON-RPC 线协议、Lua 沙箱宿主 API、事件 Payload、状态迁移表**），而非内部源码函数名或行号。术语速览见 [context/vocabulary.md](context/vocabulary.md)；动作面分层见 [action-faces.md](design/action-faces.md)。

## 拆分映射表（旧 § → 新文件；旧式引用按此行解析）

| 旧章节 | 新文件 |
| --- | --- |
| §1 产品形态与三层系统边界（含 1.1 协商矩阵、1.2 Adapter 五义务） | [design/system-boundaries.md](design/system-boundaries.md) |
| §2 章首（组织原则） | [design/state-machine.md](design/state-machine.md) |
| §2.1 Mod 是什么（边界与纯度） | [design/state-machine.md](design/state-machine.md) |
| §2.2 双层结构与真状态压缩 | [design/state-machine.md](design/state-machine.md) |
| §2.3 运行级剧情（live） | [design/run-story.md](design/run-story.md) |
| §2.4 需求级剧情（open） | [design/requirement-story.md](design/requirement-story.md) |
| §2.5 微状态判定的单一来源 | [design/state-machine.md](design/state-machine.md) |
| §2.6 事件总表（38 口径） | [design/events.md](design/events.md) |
| §2.7 拒绝面矩阵 | [design/state-surface.md](design/state-surface.md) |
| §2.8 状态存储契约 | [design/state-surface.md](design/state-surface.md) |
| §2.9 响应式推进契约 | [design/state-machine.md](design/state-machine.md) |
| §2.10 持久化泳道池与实时进度投影 | [design/lanes.md](design/lanes.md) |
| §2.11 工具桥契约 | [design/tool-bridge.md](design/tool-bridge.md) |
| §2.12 提示词面（beats）契约 | [design/prompt-surface.md](design/prompt-surface.md) |
| §2.13 身份与权限（不进剧情） | [design/identity.md](design/identity.md) |
| §3 章题 | [design/lua-api.md](design/lua-api.md) |
| §3.1 Lua 沙箱宿主 API 契约 | [design/lua-api.md](design/lua-api.md) |
| §3.2 Mod 包与多文件纯度契约 | [design/mod-package.md](design/mod-package.md) |
| §3.3 事件 Payload 线协议契约 | [design/wire-protocol.md](design/wire-protocol.md) |
| §3.4 三层动作面契约（含 registry 末注 33/34 口径） | [design/action-faces.md](design/action-faces.md) |
| §3.5 宿主 CLI 与 Daemon JSON-RPC 线协议 | [design/cli-daemon.md](design/cli-daemon.md) |
| §3.6 上下文压缩与检查点交接契约 | [design/compaction.md](design/compaction.md) |
| §3.7 serve 观察面契约 | [design/serve.md](design/serve.md) |
| §4 活文档闭环 | [wiki-governance.md](wiki-governance.md)（载体指称已改述，三职责语义保留） |

## 旧引用解析注记

- 决策表行、crates 注释与测试断言中的 `docs/DESIGN.md §N` 引用（如 §3.2 → [mod-package.md](design/mod-package.md)、§3.7 → [serve.md](design/serve.md)、§3.4 → [action-faces.md](design/action-faces.md)、§3.5 → [cli-daemon.md](design/cli-daemon.md)）经本表解析，持续有效——方案 A 裁决下 `crates/**` 零改动。
- §2.6 的 38 口径与 §3.4 末注的 registry 33/34 口径**有意并存**（前者 = D-MA23 设计目标全集，后者 = 已装载 Mod 事实集），拆分后分居 [events.md](design/events.md) 与 [action-faces.md](design/action-faces.md) 并互链，不得合并。
