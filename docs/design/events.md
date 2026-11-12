# Yggdrasill · 事件总表（39 口径）

> **定位**：剧情/操作/基础设施三类 39 个事件与 payload 落点（D-MA23 基础上 r1/D-MA24 增 lane.pool；与 action-faces 的 registry 口径并存互链）。
> **replaces**：DESIGN §2.6（L248–284）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../README.md) · **相关**：[wire-protocol.md](wire-protocol.md) · [action-faces.md](action-faces.md)
> **搬运**：以下正文自 DESIGN.md L248–L284 逐字迁移，不重写语义。

### 2.6 事件总表（39 个，三分类）

事件是唯一推进货币。三类切分：**剧情事件**（推进微状态，20 个）、**操作事件**（增删改查，9 个）、**基础设施事件**（10 个）。其中 6 个为 D-MA23 新增：`charter.set` / `run.begin` / `integration.confirm` / `integration.reject` / `release.confirm` / `release.skip`；1 个为 r1（D-MA24）新增：`lane.pool`（分池注册表动作）。注册面另含 `ygg.narrative.goto`（node:render 渲染事件）= 40 个已注册 handler（语义哨兵测试计数口径）。

**剧情事件**（payload 契约与微状态落点；行内管道符均为转义字面量）：

| 事件 | 微状态落点 | payload |
| --- | --- | --- |
| `event.start` | live·立局（进入） | `""` 或 `"<title>\|<goal>\|<success>"`（可选章程载荷，R3；不转场） |
| `charter.set` | live·立局（内） | `"<title>\|<goal>\|<success>"`（可反复修订，R4 后锁定） |
| `run.begin` | 立局 → 协调 | `""`（前置：章程三件套齐备） |
| `requirement.add` | open·立项（进入） | `"<title>\|<description>[#deps]"` |
| `requirement.dispatch` | 立项 → 计划中 / 待开发 → 开发中 | `"rN"` |
| `proposal.submit` | 计划中 → 计划门禁 / 开发中 → 交付受理 | `"<req>\|<rev>\|<stage>\|<summary>\|<artifacts>\|…"` |
| `proposal.accept` | 交付受理 → 验收门禁 | `"pN"` |
| `proposal.reject` | 交付受理 → 待开发 | `"<pN>\|<reason>"` |
| `gate.approve` | 计划门禁 → 待开发 / 验收门禁 → 集成 | `"rN"` |
| `gate.reject` | 计划门禁 → 计划中 / 验收门禁 → 待开发 | `"<rN>\|<feedback>"` |
| `integration.confirm` | 集成 → done | `"<rN>\|develop@<sha>"` |
| `integration.reject` | 集成 → 待开发 | `"<rN>\|<reason>"` |
| `coordinate.submit` | 协调 → 盘点 | `""`（前置见 R8） |
| `verify.submit` | 盘点·待验证 | `"true"` / `"all_green=false;report=…"` |
| `settlement.evolution` | 盘点·待登记 | `"artifact\|summary[\|c=…][\|cls=…][\|sessions=…][\|todo=…]"` |
| `settlement.sessions` | 盘点·待结会话 | `"moved\|remaining\|destination"` |
| `settlement.todo` | 盘点·待清待办 | `"remaining\|basis"` |
| `release.confirm` | 发布 → 批结算 | `"master@<sha>\|vYYYY.MM.DD.N"` |
| `release.skip` | 发布 → 批结算 | `"<reason>"`（必填：本批无需发布的理由） |
| `review.note` | 发布 → 协调（未武装）/ → retired（武装） | `"lesson\|score"` |

`proposal.submit` 尾段链保留固定相对序 **`exec` → `actor` → `actor_session`**（各自可独立缺席）：`exec=<agent_id>[:<reason>]` 必须在链中 `actor=` 之前并同事务绑定执行者台账；`actor=` / `actor_session=` 由**宿主注入**（声明路由），调用方自带同名段在宿主侧即被拒（`ACTOR_FORGED`）——线协议正文见 §3.3，身份语义见 §2.13。

**操作事件**（9 个，不推进任何幕）：`requirement.list` / `requirement.split` / `requirement.merge` / `requirement.amend` / `requirement.conflict` / `requirement.resolve` / `lane.pool` / `lane.claim` / `lane.release`——只改操作对象与标志位（旁路语义见 §2.4 末段，泳道语义见 §2.10）。

**基础设施事件**（10 个）：`config.set` / `config.open` / `config.close` / `map.show` / `ygg.choice_selected` / `vfs.write` / `tool.call` / `tool.result` / `agent.summary` / `stage.advance`。

> 与 §3.4 末注的关系：末注的 registry 口径与复算命令描述**已装载 Mod 的事实集**；本表是 D-MA23 设计目标全集（既有 32 + 新增 6 = 38）。代码落地后两者一致；运行期 registry 仍是 fail-closed 的运行时唯一事实。

