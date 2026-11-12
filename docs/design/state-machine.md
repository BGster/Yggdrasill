# Yggdrasill · 状态机语义与微状态判定

> **定位**：Mod 纯度边界、双层真状态压缩、微状态判定单一来源（run_phase/req_phase）与响应式推进契约。
> **replaces**：DESIGN §2 章首 + §2.1 + §2.2 + §2.5 + §2.9（L66–95, L206–247, L329–339）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../README.md) · **相关**：[events.md](events.md) · [run-story.md](run-story.md) · [requirement-story.md](requirement-story.md) · [state-surface.md](state-surface.md)
> **搬运**：以下正文自 DESIGN.md L66–L69 + DESIGN.md L70–L77 + DESIGN.md L78–L95 + DESIGN.md L206–L247 + DESIGN.md L329–L339 逐字迁移，不重写语义。

## 2. Mod 剧情与状态机语义

> 本章是 `agent-seed` Mod 行为语义的权威（D-MA23 剧情双层重写）。组织原则：**以剧情节点（微状态）为纲、以事件为推进货币**——每个节点的七项要素（主人 / 进入 / 幕内事件 / 退场 / 产出证据 / 拒绝 / 文案键）分布在：微状态表（主人、进入、退场、证据）、转场表（幕内与退场事件）、拒绝面矩阵（拒绝面）、beats 分组（文案键，§2.12）。两条设计目标：**压缩状态数量**（11 → 4 个真状态）与**主流程自循环**（协调 ⇄ 盘点 ⇄ 发布 → 下一批；终局只经显式退役）。

### 2.1 Mod 是什么（边界与纯度）

Mod 是一个**纯确定性状态机**：无 IO、无时钟、无随机（P8 / D13）。与外部世界的一切交互（git、文件、进程）都经宿主中介能力（`ygg.fs` VFS、`ygg.tool.call` 同步工具桥）发生，宿主未挂载时优雅降级且不破坏状态语义。

- **唯一写入口 = 事件**：`(state, event) -> state'`，全有或全无（P1）；事件之外不存在任何改状态的路径（L1 推进唯一性，§3.4）。
- **唯一输出 = KV + 叙事 + 菜单**：每次 dispatch 产出叙述回执（`ygg.narrate`）、持久化键值（`ygg.kv`）与至多一个 `pendingChoice` 菜单（`ygg.choice`）。
- **模块顶层纯度**（P6）：`src/**` 五层只导出纯函数与常量；处理器注册只发生在 `mod.lua` 装配入口。

### 2.2 双层结构与真状态压缩

状态分两层。**外层真状态**：内核 FSM 与需求状态机的持久化取值，压缩到 **4 个**。**内层微状态**（剧情节点）：由 KV 标志位经判定函数（§2.5）派生，不独立存储、不消耗转场预算。

真状态判据（满足任一条才配成为真状态）：

1. **跨越不可逆语义**——完成后不可回退（`done`、`retired`）；
2. **改变其他模块的全局行为**——退役后一切业务事件拒绝（`retired`），需求关闭后退出派发队列（`done`）。

| 层 | 载体 | 原状态（11 个） | 真状态（4 个） | 降级去向 |
| --- | --- | --- | --- | --- |
| 运行级 | `fsm.stage` | `coordinate` / `review` / `done` | **`live` / `retired`** | 协调/盘点/发布 → `run.phase` 微状态；`done` → `retired` |
| 需求级 | `req.<i>.status` | `draft`/`planning`/`developing`/`done` + `split`/`merged`/`dropped` | **`open` / `done`** | 进行中各态 → 九幕微状态；旁路 → `closed_reason` 字段（R11） |
| 提案级 | `prop.<j>.status` | 4 值 | 不变 | — |
| 泳道级 | `lane.<k>.*` | 3 值 | 不变 | — |

`coordinate` 与 `review` 来回切换、语义可逆——不满足任一判据，降为 `live` 的微状态；`draft` / `planning` / `developing` 同属「进行中」，降为 `open` 的微状态；`split` / `merged` / `dropped` 是**关闭原因**，降为字段（R11）。

### 2.5 微状态判定的单一来源

菜单、文案、渲染、进度投影与文档全部从**两个函数**派生，任何消费方不得自带第二份状态名分支（这是「剧情读不出来」缺陷的根治——原判定散在 `requirements.lua` / `menus.lua` / `render.lua` / `review.lua` 四处）：

```lua
-- 运行级微状态（完整判定序；外层先判 stage==retired → 退役）
function run_phase()
  if STAGE == "retired" then return "退役" end
  local p = str("run.phase")
  if p == "settle"  then return "盘点" end
  if p == "release" then return "发布" end
  if p == "charter" then return "立局" end
  return "协调"                       -- p == ""：章程齐 ∧ 常驻主态
end
```

```lua
-- 需求级微状态（判据顺序即优先级；返回 closed 时由 closed_reason 区分旁路）
function req_phase(i)
  if req_status(i) ~= "open"       then return "closed"   end
  if flag(i, "integrating")        then return "集成"     end
  if req_gate(i) == "accept"       then return "验收门禁" end
  if has_pending(i, "development") then return "交付受理" end
  if req_job(i) == "development"   then return "开发中"   end
  if req_plan_ok(i)                then return "待开发"   end
  if req_gate(i) == "plan"         then return "计划门禁" end
  if has_pending(i, "plan") or req_job(i) == "plan" then return "计划中" end
  return "立项"
end
```

判定序的关键点：`integrating` 先于一切（集成幕是最近一次裁决）；**待开发先于计划门禁与计划中**——`plan_ok=true` 时即使有遗留 pending 提案也判待开发，三条打回路径的落点一致性由它保证；`closed` 不是幕名，由 `closed_reason` 区分正常交付与旁路关闭。

**中文名 ↔ 代码标识符映射（落地实现口径）**：两个判定函数在代码里返回**英文标识符**（菜单键 /
渲染键 / 摘要 `phase` 字段需要非空稳定键；`run.phase` 存储值 `""` 规范化为 `coordinate`），
文案面（拒绝理由 / 图例）经 `run_phase_label()` 换中文名。判定序与上两段伪码逐字一致：

| 层 | 代码标识符（判定函数返回） | 中文名（文案面） |
| --- | --- | --- |
| 运行级 | `retired` / `charter` / `coordinate` / `settle` / `release` | 退役 / 立局 / 协调 / 盘点 / 发布 |
| 需求级 | `draft` / `planning` / `plan_gate` / `ready` / `developing` / `dev_review` / `accept_gate` / `integrating` / `done` / `closed` | 立项 / 计划中 / 计划门禁 / 待开发 / 开发中 / 交付受理 / 验收门禁 / 集成 / 完成 / 旁路关闭（由 `closed_reason` 细分 `split` / `merged` / `dropped`） |

### 2.9 响应式推进契约

在发出 `pendingChoice` 之前，Mod 按确定性的注册顺序冲刷已注册的响应式效应（`useEffect`）——`render()` 顶部、`flush_effects`，最多 `MAX_PASSES = 4`，不收敛即为硬错误。每个效应必须遵守四条不变量：

1. **唯一声明后继**：迁移确定性且只在一处声明；`useMemo` 依赖必须是 KV 派生标量（状态/作业/待决/就绪/计数），保存重载后可复算，不依赖 Lua 内存 diff。
2. **零外部判断**：无需人类批准或未挂载的外部 I/O（无文件读、无会话状态）。
3. **绝不绕过门禁**：五个里程碑动作（`proposal.accept` / `gate.approve` / `coordinate.submit` / `verify.submit` / `review.note`）绝不由效应触发；效应不推进任何「主人=人」的幕。
4. **无渲染的效应**：效应可改变 KV 并 `ygg.narrate`，但绝不调用 `render()` 或 `ygg.choice`。

现行效应清单（全部满足四不变量）：`auto_dispatch`（`gate.approve(plan)` 后将刚变待开发的需求在同回合自动派发 development，条件 = 无待决提案/门禁/冲突且槽位空闲）；`tools.receipt` / `tools.reactive`（工具回执落账与结果折叠，§2.10 / §2.11）；**盘点收口效应**（R9：盘点四项齐备自动转发布幕——判据纯 KV、无外部证据、非门禁动作、不渲染，「唯一声明后继」的又一实例）。

