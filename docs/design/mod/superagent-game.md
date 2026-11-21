# Yggdrasill · Superagent Game（P4 可运行多 agent 游戏）
> **id** mod/superagent-game · **模块** ③ mod 载体 · **触发** 改 `packages/mods/superagent/**` 任一行之前
> **读不到代价** silent-drift（信封形状 / 路由表 / 门禁口径漂移，重放断言失效）
> **定位**：《超级中介局》的游戏契约——核心循环、信封接口、inbox 路由、观察面与验收映射。计划即规格（r1 rev 1，`/tmp/p4-plan.md`）；本文是它的就地治理文档。
> **上游**：[mod-package.md](mod-package.md) · [tool-bridge.md](tool-bridge.md) · [serve.md](../host/serve.md) · [sim-kernel.md](../kernel/sim-kernel.md) · [quest.md](../packages/quest.md)
> **载体**：`packages/mods/superagent/`（五件套 mod；设计目标零 `crates/ygg-*` 改动）。

## 1. 形态与核心循环

可运行 game = mod 五件套 + `egg new/open` 开局可玩；非脚本、非服务。

**循环（委托 → 执行 → 交付）**：
1. **开局** `event.start`：世界种子（`pkg.procgen.hash(seedText)`）→ 世界规模批帧进 sim（`ygg.sim.spawn_batch`，裸实体批，`src/simframe.lua` 编码）→ NPC/道具 KV 实体面铺场（`pkg.entity` + `pkg.role_skill`）→ NPC 委托方 `quest.open` 发布委托（giver=NPC ref，taker 待定）→ 任务板待选。
2. **委托**：玩家/事件接单（`commission.take`，assign taker）→ 任务板可观察（`quest.open_ids`）。
3. **执行**：在局文员干活 = 游戏内动作 + 挂载点业务文件读写（`ygg.fs.*` 写穿 `mounts/biz`）；外线 agent 干活 = 出向 `agent_call`（`ygg.meta("tool_calls")` → `superagent.agent_call`）→ 回执信封下行。
4. **交付**：回执落账 → `quest.advance` 推进至 done（`quest.completed` 恰一次，I3）→ 报酬 = role-skill 技能成长（`grant_skill` / `recharge`）→ 下一单。

**业务钟**：一个动作 = 一个业务日 = `ygg.sim.step(1)` + KV `sg.day` +1。sim 是进程内状态（egg 常量种子重建），日的真源是 KV；sim.step 的职责是让 Tier 1 状态哈希随进度移动。委托限期 = 开单日 + `deadlineDays`（业务日），逾期自动 `quest.abandon`（R4 收口）。

**降级诚实账**：裸 `KernelFacade`（playtest / selftest）未挂 sim/fs 桥——叙述一行 `[SIM-UNMOUNTED]`，KV 语义照走，绝不静默假装（工具桥 `[VFS LEDGER ONLY]` 同款纪律）。

## 2. 信封接口（多 Agent Harness，不绑死一家）

**通用 JSON 信封**（全标量字段，D11 标量律；声明账 = `data/envelope.json`，与 pack.json `inboxRoutes` 逐条对账）：

```json
{"v":1,"kind":"agent.chat","externalId":"<唯一幂等键>","agent":"<agent-id>","session":"<opaque>","text":"<单行正文>","refs":"<逗号分隔实体/任务ref>","ts":<int毫秒>}
```

六类 kind → 六条路由（`pack.json inboxRoutes`；wire 段序 = `fields`）：

| kind | 语义 | 入口 |
| --- | --- | --- |
| `agent.chat` | agent 经对话通道发言 | `--requirement`（named 写绑定：首段=agent-id，余段=正文）或适配器直写 JSONL |
| `agent.action` | agent 申报游戏动作（接单意向） | JSONL / 适配器 |
| `quest.delivery` | agent 申报交付回执（refs 首段 = 委托 key） | JSONL / 适配器 |
| `quest.custom` | 自定义任务模板描述符（text = 模板串） | JSONL / 适配器 |
| `harness.ping` | 存活探测（只记台账） | JSONL / 适配器 |
| `agent.error` | 错误上报（入对话线程记账） | JSONL / 适配器 + mod 内失败记账 |

**入向裁决（计划 §2）**：输入走 inbox（持久、幂等、带提交者身份位 `actor`/`actor_session`，宿主注入）；观察走 serve 只读面；`--drive cmd:` 只作冒烟回归的脚本决策通道。

**幂等**：`sg.seen.<externalId> = kind`。重放 = 同 externalId 再入 → 叙述 `【重放】` 行、零状态变更（宿主 `replayPrefix` 语义）；**cursor「dispatch 成功且存档后才前移」由宿主 drain 语义承载**，mod 只做键级去重。

**出向**：`pack.json tools.superagent.agent_call`（cli、`program=adapter/superagent-convert`、`{{text}}/{{agent}}/{{session}}` 占位、`resultEvent="harness.replied"`、`inlineWaitMs=5000` 同进程就地回投——确定性重放的根据）。回执线形 `<call_id>|<tool_id>|<status>|<code>|<output>|<external_id>`；调用登记 `sg.call.<call_id> = ext|questKey`，回执按登记落账 → 交付。**超时不自动重试**：失败 = `agent.error` 记账 + 对话线程一行；重试是 harness 侧策略（不进 mod）。

**适配器参考实现（`adapter/`）**：`superagent-convert`（薄转换器：信封 ↔ stub harness CLI 语法互转）+ `stub-harness`（确定性 echo agent，只读、零副作用）。multica/herdr 转换器只留文档示例，不写死任何一家 SDK（§5）。

## 3. 路由与观察面

- **面板**：`pack.json panel.entry = panel/index.html`；serve iframe 托管于 `/panel/<run-id>/index.html`，页面拉 `/api/runs/<id>/events` 渲染 `agent.dialogue` / 任务事件行 + `/api/stream` SSE 变更触发刷新。serve 零写端点（0b 红线）——对话**绝不**经 serve 下行。
- **机器可读投影**：对话行/台账双写 `ygg.emit`（`agent.dialogue` / `agent.recorded` / `item.transferred`，载荷全标量），事件页与重放对账都读它。带 `ygg.choice` 的 handler 不 emit（宿主 D1 语义：choice 前的派生事件被丢弃）——板面动作的对话行落 `pkg.dialogue` 线程 KV，经 `agent.summary` 的 `meta.dlg = {threads, lines}` 读出；入向 handler（无 choice）的行走事件页。
- **线程键约定**（cast.lua）：NPC = `cast.<npc key>`，外线 agent = `agent.<agent-id>`，委托 = `q.<quest key>`。报酬成长 = 交付单点调 `reward(taker_id, skill)`：首授 `grant_skill`，再授 `recharge` 回满格；外部 agent 台账 `sg.agent.<id> = chats|deliveries`。

## 4. 验收映射（契约 §五 P4）

| 验收 | 实现面 | 冒烟 |
| --- | --- | --- |
| 开一局 | 五件套 + 开局铺场 | `egg new --mod packages/mods/superagent --in /tmp/ygg-p4-runs --event event.start` EXIT=0 |
| 完整任务流 | quests.lua 委托流转 + 挂载点干活 | 脚本化事件序列 `commission.take/work/deliver` → `quest.completed` 在场 ∧ 重放 `sim.hash` 逐字节相等 |
| agent 对话参与 | inbox 信封 + agent_call 出向 + 面板 | `--requirement` 注入 → `--wait-tools` drain → 线程增行 + 回执落账；serve 面板可见 |
| 挂载点读写 | pack.json `mounts`（`/tmp/ygg-p4-mount`，合成数据） | `fs.write` → `fs.read` 逐字节相等 ∧ `<run>/mounts/biz` 在场 ∧ `fs.jsonl` 审计在场 |

## 5. multica / herdr 转换器示例（文档扩展点，不写死 SDK）

```sh
# multica 侧（伪码 ~20 行）：信封 → multica 会话消息
#   multica send --session "$session" --agent "$agent" --text "$text"
#   reply=$(multica recv --session "$session" --last)
#   echo "$reply" | superagent-convert in --kind agent.chat --external-id "$ext"
# herdr 侧：herdr run --id "$agent" --prompt "$text" → stdout 即 output
# 两者差异集中在「怎么把一个信封变成一次 CLI 调用」——契约面是本文件 §2 的
# 信封与 resultWire，不是任何一家 SDK。
```

## 6. 设计依据（为什么这样落）

- **世界铺场走「sim 裸实体批 + KV 实体面」双层**：sim 批帧让世界规模进 Tier 1 状态哈希（重放断言非常数）；人物/道具的语义数据（属性袋/角色）落 KV 面的三系统包。带列 sim 帧（Number/Str/Bool/Ref/Tag）不用——三系统数据形状全落 KV/entity/派生事件面（计划 §4「热核面 (b) 预计零项」），且沙箱方言里手写列载荷编码只添第二份编解码器风险。sim 帧 builder 只产裸实体批一种形状（FNV-1a 64 与 canonical.rs / procgen 跨语言同源）。
- **quest.delivery 与工具回执分线**：`harness.replied` 是出向工具的 `resultEvent`（宿主 inline 回投事件），不是 inbox kind；六类 kind 与计划 §3 信封一一对应。两条线在 agents.lua 汇合到同一套交付落账。
- **确定性优先**：出向工具用 `inlineWaitMs>0`（同进程就地回投）而非脱体 worker（墙上时钟序）；业务钟 sim 步数由事件序决定，不用墙上时钟——「同信封序列重放 sim.hash 逐字节相等」因此可逐字节复算。
- **mod 层组织（三系统 × 包复用）**：人物 = `pkg.entity` schema spawn + `pkg.role_skill` 角色/技能 + `pkg.dialogue` 线程；道具 = 属性袋 + 转移语义（mod 层 set + `item.transferred`）+ `pkg.codec` 存档往返；任务 = `pkg.quest` 模板/实例/推进 + `pkg.dialogue` 节点文案单副本。全部 (c) mod 层，零新包（§1.7 判据：每能力恰一用点不立包）。

维护责任 = 改 `packages/mods/superagent/**` 的执行者；触发 = 任一相关 commit（同分支就地改写本文）；检查 = `tests/` 自测用例 + 终验四条冒烟（§4）。
