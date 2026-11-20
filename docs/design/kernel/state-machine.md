# Yggdrasill · 状态机语义与微状态判定
> **id** kernel/state-machine · **模块** ① ygg 内核 · **触发** 改状态机语义 / 增删事件 / 改微状态判定之前
> **读不到代价** silent-drift（按记忆改判定 → 双重推进或旁路打开，门禁全绿也看不出来）

> **定位**：Mod 纯度边界、双层真状态压缩、微状态判定单一来源（run_phase/req_phase）与响应式推进契约。
> **replaces**：DESIGN §2 章首 + §2.1 + §2.2 + §2.5 + §2.9（L66–95, L206–247, L329–339）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../../README.md) · **相关**：[events.md](events.md) · [run-story.md](../../context/story/run-story.md) · [requirement-story.md](../../context/story/requirement-story.md) · [state-surface.md](state-surface.md)
> **搬运**：以下正文自 DESIGN.md L66–L69 + DESIGN.md L70–L77 + DESIGN.md L78–L95 + DESIGN.md L206–L247 + DESIGN.md L329–L339 逐字迁移，不重写语义。

## 2. Mod 剧情与状态机语义

> 本章是 `agent-seed` Mod 行为语义的权威（剧情双层重写的成果，依据见 §2.2.1）。组织原则：**以剧情节点（微状态）为纲、以事件为推进货币**——每个节点的七项要素（主人 / 进入 / 幕内事件 / 退场 / 产出证据 / 拒绝 / 文案键）分布在：微状态表（主人、进入、退场、证据）、转场表（幕内与退场事件）、拒绝面矩阵（拒绝面）、beats 分组（文案键，§2.12）。两条设计目标：**压缩状态数量**（11 → 4 个真状态）与**主流程自循环**（协调 ⇄ 盘点 ⇄ 发布 → 下一批；终局只经显式退役）。

### 2.1 Mod 是什么（边界与纯度）

Mod 是一个**纯确定性状态机**：无 IO、无时钟、无随机（P8，与沙箱安全边界同一件事，见 [lua-api.md](lua-api.md)「设计依据」）。与外部世界的一切交互（git、文件、进程）都经宿主中介能力（`ygg.fs` VFS、`ygg.tool.call` 同步工具桥）发生，宿主未挂载时优雅降级且不破坏状态语义。

- **唯一写入口 = 事件**：`(state, event) -> state'`，全有或全无（P1）；事件之外不存在任何改状态的路径（L1 推进唯一性，§3.4）。
- **唯一输出 = KV + 叙事 + 菜单**：每次 dispatch 产出叙述回执（`ygg.narrate`）、持久化键值（`ygg.kv`）与至多一个 `pendingChoice` 菜单（`ygg.choice`）。
- **模块顶层纯度**（P6）：`src/**` 五层只导出纯函数与常量；处理器注册只发生在 `mod.lua` 装配入口。

#### 2.1.1 设计依据（内核规格 / 首个里程碑 / 并发模型）

**内核从第 1 天起就是完整 RPG 级规则内核**：实体、属性、标签、事件总线、FSM、Lua 条件六件齐备，文字冒险只是它的第一个内容形态。**首个里程碑是「引擎 + 手写参考 Mod」，内核语义被证明之后才接生成流水线。** **并发模型 = 单实例同步 FIFO + 宿主侧分片**：每个 `KernelFacade` 严格单线程 FIFO，水平扩展靠相互隔离的分片，分片之间只经规范的 tick 边界消息通道通信。

**依据**：反方案是「先写一个文字冒险专用内核、等需要 RPG 时再重写」——那会把已验证的内容资产连同语义一起丢进重写；先接生成流水线则会把未证明的内核缺陷放大成内容缺陷。单实例 FIFO 是拿并发换确定性：确定性是回放、机检与「同样输入同样回执」的前提，`probe_determinism` 在 1 万实体上验证亚毫秒 dispatch，说明这个取舍还没到瓶颈。

**判据**：① 阻断——新增只有文字冒险用得上的私有状态形式，或绕过 `KernelFacade` 的 FIFO 直接并发写内核；② 症状——同一事件序列两次回放产出不同 KV / 回执，或 `crates/ygg-sandbox/tests/probe_determinism.rs` 的非确定性断言转红；③ 确认——`cargo test -p ygg-sandbox --test probe_determinism`。

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

#### 2.2.1 设计依据（为什么真状态只有 4 个）

真状态的判据是**不可逆语义 ∨ 改变全局行为**，只有满足它的才配做状态；`split` / `merged` / `dropped` 是「关闭的理由」，所以降为 `closed_reason` 字段而不是状态。

**依据**：状态越多，转场表越大、拒绝面越难穷举，而「某个状态忘了写拒绝面」是这类系统最常见的旁路来源。反方案（每个关闭理由一个终态）会让转场表项数量级增长，且每个新终态都要在菜单、文案、渲染、门禁判定四处同步；字段化之后只剩一条判定 `status=="done"`。

**判据**：① 阻断——新增一个**独立存储**的需求级或运行级状态（而不是派生微状态或字段）；② 症状——`req_phase()` / `run_phase()` 之外出现第二处状态判定来源，或 §2.8 键表里出现第二个状态键；③ 确认——`cargo test -p ygg-sandbox --test selftest` 的徽章 / 微状态派生用例与 `state-surface.md` §2.8 键表断言。

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

1. **唯一声明后继**：迁移确定性且只在一处声明；`useMemo` 依赖必须是 KV 派生标量（状态/作业/待决/就绪/计数），保存重载后可复算，不依赖 Lua 内存 diff。（宿主时钟不可读：`kvRevision` 无读面且语义非单调、`ygg.ticks()` 生产恒 0——四格清单与逐条证据见 [state-surface.md](state-surface.md) 的「沙箱时序源」节。）
2. **零外部判断**：无需人类批准或未挂载的外部 I/O（无文件读、无会话状态）。
3. **绝不绕过门禁**：五个里程碑动作（`proposal.accept` / `gate.approve` / `coordinate.submit` / `verify.submit` / `review.note`）绝不由效应触发；效应不推进任何「主人=人」的幕。
4. **无渲染的效应**：效应可改变 KV 并 `ygg.narrate`，但绝不调用 `render()` 或 `ygg.choice`——choice 一旦产生，内核会丢弃队列余部（`transition.rs`）。

#### 2.9.1 省回合动作与尾段协议

- **内联执行者台账**：`proposal.submit` 的保留尾段允许 `exec=<agent_id>[:<reason>]`，在提案入库前、同一事务内通过与 `config.set executor.*` 相同的 `src/domain/executor.lua` 实现登记。登记失败则整个载荷被拒绝且不产生提案。固定相对顺序为 `…|exec=<id>[:<reason>]|actor=<actor>|actor_session=<session>`；三段各自可省略、总计至多三段。reason 内使用 `:`（如 `exec=agent2:handoff`），因为外层按 `|` 切分。
- **宿主身份段不可伪造**：`actor=` / `actor_session=` 只能由宿主在 `inboxRoutes.<kind>.actor / .actor_session` 声明路由上注入；调用方自带则宿主以 `ACTOR_FORGED` 拒绝，不 dispatch、不持久化。尾段位置错误由 Mod 分别以 `req_prop_exec_misplaced` / `req_prop_actor_misplaced` / `req_prop_actor_session_misplaced` 拒绝；宿主会把短于声明七字段的载荷补位，因此中段错位不能静默吞掉。三态互证只比较 `exec=` 与宿主 `actor=`；`actor_session` 纯审计，永不参与判定。首条带身份的成功写先到先得认领不可变 `run.json opener_actor`，失败/拒绝不写，同一 actor 重写为 no-op；身份模型正文见 [identity.md](../mod/identity.md)。
- **阶段合并**：`requirement.dispatch` 遇到 `draft` 行时直接派发 plan 并写入 `planning`，默认管线没有 analysis 回合。`analyzing` 仅为旧存档兼容态，以及 `planning` 行显式提交并接受 analysis 提案时的兼容态；`R_ANALYZING` 与读者保留，但不新增迁移代码。

#### 2.9.2 现行效应及折叠边界

- **`auto_dispatch`**：`proposal.accept(analysis)` 或 `gate.approve(plan)` 的 handler 用 `hooks.mark_auto_dispatch(index)` 只标记刚就绪的需求；效应仅在本回合无待决提案、无待决门禁、无冲突且有空闲 job 槽时派发这一条。`requirement.add` 的 draft 永不自动派发；`gate.reject` / `proposal.reject` 不派生；`gate.approve(accept) -> done` 不派生，释放槽位由主代理决定。不得新增 `derive.auto` 开关形成第二旁路。
- **`tools.receipt`**：`src/runtime/tools.lua` 在 `tool.result` 到达后只 latch `tool.notified.<call_id>` 并叙述一次，不触碰 `req.*` / `prop.*` / 门禁 / 泳道 / 阶段状态。KV 标量依赖与 latch 幂等保证 replay 两次内收敛，绝不出现第二条 `[TOOL SETTLED]`。
- **`tools.reactive`**：成功 `tool.result`（`status ok`、`code 0`）由紧随 receipt 注册的第二个效应折叠进领域 KV；它与 receipt 共享同一 `flush_effects`，但不共享职责。反应器只能写 KV 与叙述，不得 render/choice、改阶段或提案/门禁状态，也不得 `flow.refuse`——拒绝前缀会被宿主计为失败步，使事实探针伪装成 Mod 拒绝。`vfs.write` 是真实事件处理器而非反应器，故可使用 `reject_payload`。
- **失败探针是警告，不是门禁**：`[LANE PROBE FAILED]`（`commits=0` 或 `clean=0`）只叙述并留下 `lane.<k>.probe.*` 查询面；接入 `gates.gate_problem()` 会让一棵脏树永久死锁 `coordinate.submit` / `review.note`，且没有「撤销探针」动作。冲突雷达是唯一例外：它经 `props.req_do_conflict` 写 `req.<i>.conflict`，进入 `req_conflict_total` 并阻塞派发/收口，只有 `requirement.resolve` 清除。
- **机证与人证同级**：`gate.verify_suite` 与 `verify.submit` 写同一 KV、省一回合，但不更高级；必须保留 `verify.source`，且只能在 settle 相位、`gate_problem() == nil` 时记录 `passed=1`。报告不得把机器路径描述为高于人工声明。
- **截断与降级必须诚实**：集合比较尊重宿主输出上限；`parse_result` 返回 `truncated` 并写 `tool.<call_id>.truncated` 时雷达跳过，因为截断名单会造成假阴性，而继续比对会制造假冲突。`vfs.write` 无 `ygg.fs` 时只记同一台账并叙述 `[VFS LEDGER ONLY]`，不得声称已写盘；两种宿主形态下 KV 与叙述必须一致。

现行效应还包括**盘点收口效应**（R9：盘点四项齐备自动转发布幕——判据纯 KV、无外部证据、非门禁动作、不渲染）。所有效应都满足上述四不变量。

#### 2.9.3 设计依据（省回合三规则与钩子边界）

响应式钩子（`useMemo` / `useEffect` / `flush_effects`）在 `render()` 发出唯一 `ygg.choice` **之前**收敛，所以效应可以推进状态并叙述，但**永不自己渲染或发菜单**——一旦 choice 已产出，内核会丢弃队列剩余部分；这也是「自动后继」必须被四条不变量约束的原因。

**依据**：省回合的做法里最贵的是「等一轮再派发」。三条规则各砍一处往返——`draft` 阶段合并（派发即入 planning）、`proposal.submit` 内联 `exec=` 注册（省一次 `config.set`）、`gate.approve(plan)` 后的响应式派发（省一次 `requirement.dispatch`）。反方案（在 handler 末尾追加渲染）会让「choice 已发出」之后的状态变更被静默丢弃。

**判据**：① 阻断——在 `useEffect` 里调用 `render()` / `ygg.choice`，或把门禁类动作（主人 = 人）做成自动后继；② 症状——一次派发出现两个 choice，或 `flush_effects` 撑满 `MAX_PASSES` 后报错（不收敛是缺陷，不是警告）；③ 确认——`cargo test -p ygg-sandbox --test selftest` 的同回响应式派发与 `useMemo` 失效用例。

### 2.10 Mod 行为验收（触源码或场景措辞时）

修改 `.ygg/mods/agent-seed/src/**`、`.ygg/mods/agent-seed/mod.lua` 或 `data/texts/` 的地图措辞，除 lint 与 playtest 外，必须运行：

```bash
cargo test -p ygg-sandbox --test selftest
```

该套件覆盖阶段合并与 analysis 兼容、plan/accept 门禁通过与打回、stale/amend、同回响应式派发与 useMemo 失效、门禁独立性、自身 pending 阻塞、development 提案打回、内联 exec 尾段、settlement 合并段、旧存档缺 `plan_ok`、双 pack 操作/设计地图契约、工具桥请求/结果/replay/声明奇偶、泳道工具与冲突雷达、VFS 台账、settle 机证以及声明完整性。测试计数以当次命令真实输出为准，不在文档硬编码；新增行为必须新增或修改对应用例。

all-1 playtest 只覆盖 happy path，不会走 `gate.reject` / `requirement.amend` / `proposal.reject`，也看不见省回合机制；`/tmp` 临时探针只能补充，不能替代语义套件，未覆盖场景必须逐项声明。三问判据：① 阻止「改 Mod/地图后只跑 lint+playtest 就称验收」；② 症状是分支语义或地图措辞静默回归而门面检查仍绿；③ 除上述命令外做变异探针——暂时恢复旧语义，相关测试必须转红，再恢复实现并复绿。

维护责任 = 触碰上述路径的执行者；触发 = 每个相关 commit；检查 = lint、playtest、语义套件与变异探针真实输出。缺任一项不得声称行为验收完成。

### 2.11 内核/宿主字面三件套

修改 `crates/**`（状态机、driver、宿主持久化面）必须逐字运行并分别记录真实退出码与测试计数：

```bash
cargo test --workspace
cargo test --example egg -p ygg-sandbox
cargo run -q --example selftest -p ygg-sandbox -- --mod packages/mods/pkg-selftest --packages packages/libs
```

第二条不可省：Cargo 的 `--workspace` 不编译 example target 的单元测试，`crates/ygg-sandbox/examples/egg.rs` 中持久化原子性、reset、热载与结算策略断言不会因只跑 workspace 而执行。`cargo check`、`verify-suite --mode fast`、过滤测试、单测选择或 `YGG_VERIFY_COMMANDS` 覆盖各有合法用途，但只能证明其自身，不能替代字面三件套。

第三条是 **Lua 自检套件**（`--mod packages/mods/pkg-selftest`），不是 Rust 集成测试：v3 收口刻意删除了 `crates/ygg-sandbox/tests/selftest.rs`（`git log -S"--test selftest"` → `b4beaf7 feat(goal v3): 收口`），旧命令实测 `error: no test target named 'selftest' in 'ygg-sandbox' package`（exit=101）。判据保留：① 阻止「改了 `crates/**` 只跑 workspace 就称验收」（workspace 看不见 example target，也看不见 Mod 语义）；② 症状是 example target 断言或 Lua 侧包语义静默回归而 `cargo test --workspace` 仍绿；③ 确认 = 三条命令的真实退出码与用例数（`cases passed=<n> failed=<n>`）逐条记录，不以 `cargo check` / 过滤测试 / `YGG_VERIFY_COMMANDS` 替代。

三问判据：① 阻止「触内核/宿主后只跑 workspace 就称门禁绿」；② 可见症状是 workspace 与 example egg 的 `test result` 批次数不同，前者缺 example 批；③ 用 `cargo test --workspace | grep -c '^test result'` 与 `cargo test --example egg -p ygg-sandbox | grep -c '^test result'` 对比并保存两条原生命令的末行。维护责任 = crates 变更执行者；触发 = 每个 crates diff；漏跑或替代命令 = 假绿，不得验收。

