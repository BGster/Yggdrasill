# agent-seed · 通用冷启动元工作流 Mod

M-Agent 泳道的 seed mod：把「一次完整的 Agent 工作」配置成一个可游玩的 Mod。任何 Agent
装载它 + 注入任务描述即可开工，**零定制**——状态机、动作协议、门禁口径、演进策略、现场
规则、运行模式全在 Mod 里，不在提示词里。

怎么玩（构建 / 开局 / 单发推进 / 查规则 / 局末结算）见仓库根 `AGENTS.md`；本文件只说 Mod
内部结构。

## 状态机（六态）

```text
intake ──task.intake──▶ decompose ──plan.submit──▶ implement
                                                    │
                              slice.report（最后一片）│
                                                    ▼
             implement ◀──verify.submit（失败，打回）── verify
                                                    │
                                verify.submit（全绿/宽放）│
                                                    ▼
                         review ──review.note──▶ done（局末【结局）
```

状态由 `ygg.fsm` 单机单事件 `stage.advance` 承载（payload = 目标态，边 = 上图合法转移，
`when` 按目标态白名单），落 KV `fsm.stage`，随存档往返。非法目标 = 静默 no-op；真正的门
在 handler 里（阶段守卫 → 拒绝回执）。

## 动作协议

pendingChoice = **当前阶段允许的动作集**（协议约束，不是提示词约束）。选项文本自带载荷形
状，选中序号即派发同名动作事件；Agent 也可以直接派发事件（走同一套守卫与副作用）。内核
payload 只能是 JSON 标量，故复合载荷压成单个字符串：

| 事件 | 载荷（字符串 wire format） | 迁移 |
| --- | --- | --- |
| `task.intake` | `title\|description`（空载荷 = 默认占位任务） | intake → decompose |
| `plan.submit` | `s1=标题; s2=标题; …`（1–8 片；只给标题时自动 id） | decompose → implement |
| `slice.report` | `slice_id\|summary\|artifacts`（artifacts 半角逗号分隔） | implement → implement / verify |
| `verify.submit` | `true` / `false` / `all_green=…;report=…` | verify → review（绿/宽放）/ implement（打回） |
| `review.note` | `lesson\|score(1-5)`（缺省分 3） | review → done |
| `config.set` | `settings.<key>=<value>`（`settings.` 可省） | 任意态，不改变主状态 |
| `guide.show` | `<section id>`（空载荷 = 本阶段默认节） | 任意态，不改变主状态 |
| `map.show` | 空 / `0` / `full`（同一张全图） | 任意态，不改变主状态 |

**菜单顺序**：每态第一条 = 主前进动作（全 1 贪心走查可通关）；`guide.show` → `map.show` 依次紧随
主前进动作（verify 态排在其失败分支之后）；`config.set` 常驻且置末，配置内部固定为
`play.mode` → `evolution.mode` → `gates.require_all_green`。两个只读动作都在改配置之前。

越阶动作以可读回执拒绝：`【动作被拒】` + 当前阶段允许的动作清单；载荷不合法为 `【载荷被拒】`
+ 具体原因。两者都**原地重渲染**（主状态不变、pendingChoice 保持）。

## 现场规则（playbook 资源）

`data/playbook.json` 是 AGENTS.md 十节迁移来的规则资源（12 section / 74 条原子规则，schema
`agent-playbook.json`）；`mod.lua` 顶层 `ygg.data("ygg:agent/playbook")` 快照，`guide.show`
按 section id 取用。「规则在现场可查询」替代开局宣读。

- **载荷**：section id（`credentials` / `repository` / `branches` / `lanes` / `dispatch` /
  `integration` / `gates` / `live_evidence` / `evidence` / `output` / `pointers` /
  `hard_constraints`）；空载荷取本阶段默认节。
- **回执**：该节全文逐条 narrate——标题 + 摘要 + 每条规则（`［必须］/［禁止］/［应当］` +
  依据），外加该节附属面：验证命令（`gates`）、仓库布局（`repository`）、文档指针
  （`pointers`）、备注；末尾一行给「其余章节怎么查」。
- **阶段默认节**（每阶段回执只提示一行，全文按需拉）：

  | 阶段 | 默认节 | 轮换口径（只读 KV，不写状态） |
  | --- | --- | --- |
  | `intake` | `output` | 固定 |
  | `decompose` | `integration` | 固定 |
  | `implement` | `repository` → `lanes` → `dispatch` | 按下一片序号轮换 |
  | `verify` | `gates` → `live_evidence` | 按已失败次数轮换 |
  | `review` | `evidence` / `hard_constraints` | 按计划规模 |

- **非法 id**：`【指南被拒】` + 全部 12 个合法 id，主状态与 pendingChoice 不变。
- 渲染路径不写任何状态（默认节的轮换只读 KV 计数），故同一步重渲染不会漂移。

## 剧情节点图（map.show）

本 Mod 的「剧情」就是六态工作流，`map.show` 把状态机图渲染成 ASCII 节点图。图**不是手画**：
`mod.lua` 里 `MAP_NODES`（有序节点表）+ `MAP_EDGES`（带动作名的边表）是单源，渲染器逐节点
取出边——改状态机就改这两张表，不会出现图文不一致。不额外加资源/Schema（图是代码的结构事实，
不是可编辑的故事内容；文本词条仍在 `data/scenes.json` 的 `beats.map_*`）。

```text
【剧情节点图 · 六态工作流】当前 >> [implement] << ← 你在这里｜进度 1/3 片
  [intake]    ──task.intake──▶ [decompose]
  [decompose] ──plan.submit──▶ [implement]
▶ [implement] ├─slice.report──▶ [implement]（非最后一片，同态继续）
              └─slice.report──▶ [verify]（全部切片后）
  [verify]    ├─verify.submit=true──▶ [review]
              └─verify.submit=false──▶ [implement]（打回，重做最后一片）
  [review]    ──review.note──▶ [done]（局末【结局）
  [done]      ──（局末·无出边）
常驻动作（不推进主图，任意阶段可用）：config.set · guide.show · map.show——任务/切片/门禁/配置一律不变
```

- **载荷**：空 / `0` / `"full"` → 同一张全图（内核 `--payload` 缺省值就是 `0`）；其他值 →
  `【地图被拒】` + 用法，主状态不变。目前不做局部视图（六态图本来就一眼看得完）。
- **当前节点**：头行 `>> [<stage>] << ← 你在这里` 与行首 `▶` 同源于 KV `fsm.stage`；有计划时头行
  额外附 `｜进度 X/Y 片`（`plan.done` / `plan.count`）。
- **不改状态**：与 `config.set` / `guide.show` 一样是常驻动作，回执末尾重渲染当前阶段的
  `pendingChoice`——不会吞掉主前进动作。

## 两层配置

- **静态默认**：`data/config.json`（schema `agent-config.json`，`ygg.data` 装载期快照）
- **动态覆盖**：KV `settings.*`（`config.set` 写入）——如 `config.set play.mode=goal`
- **判定**：动态覆盖 > 静态默认

### `play.mode`：`goal` 挂机 / `guide` 逐步确认（缺省 `guide`）

| 值 | 含义 | 每阶段提示词面（两版取一） |
| --- | --- | --- |
| `guide`（缺省） | 每一步推进都要人选择/提交，Mod 不自动连跑 | `逐步确认（guide）：…**这一步由你确认后提交**…` |
| `goal` | 宿主（CLI 交互模式）免 stdin 自动推进到结局并自动结算 | `挂机（goal）：无需确认——宿主自动…` |

- 每个阶段各有两版文案（`data/scenes.json` 的 `beats.<stage>_guide` / `beats.<stage>_goal`），
  按该阶段动作写（立项要人确认提交描述、切片要人确认计划与上报、门禁要人确认结果…），
  不是一句全局套话；局末另有 `beats.play_done_goal` 说明本局是挂机跑完的。
- 每阶段回执顶部固定一行 `【运行模式】play.mode = …`（拒绝回执与局末也带）——当前模式始终可见。
- 模式只改**提示词与宿主推进节奏**：状态机、动作协议、只读动作（`config.set` / `guide.show` /
  `map.show`）行为完全不变；查询类动作在两种模式下都可用。
- 宿主（`examples/agent.rs`）**每步现读**（静态 `data/config.json` → 最新 autosave 的 KV
  `settings.play.mode`）：guide 阶段 `config.set play.mode=goal` 下一步即生效，转入挂机直到结局。
- 合法性：`play.mode` 只认 `goal` / `guide`，其他值走 `【载荷被拒】`（KV 不写、状态不变）。

配置控制状态流转的三处实证：

1. `play.mode` 决定推进节奏：`goal` = 宿主自动按推荐主动作推进到结局，`guide` = 每步都要人确认；
   每个阶段的提示词与 `【运行模式】` 行都跟着变。
2. `gates.require_all_green=false` → `verify.submit all_green=false` 也放行进 review（回执
   注明宽放路径）；`true` → 打回 implement，退回最后一片重做。
3. `evolution.mode` 决定局末复盘的修订回执：`auto` 已在白名单（prompts/beats）内登记自动
   修订请求、`suggest` 产出建议包、`off` 仅报告；额度由 `evolution.max_per_run` 约束，进入
   复盘的瞬间结算（`evolution.requested` 落 KV，不随后续 `config.set` 抖动）。`auto` 仅登记
   宿主可执行的自动修订请求；真实 revise 执行面待接，内核无 IO 不改文件。

## 入口红线（入库面）

游戏设置（`data/config.json`）、动态 `settings.*`、本 Mod 的 playbook 与经验/成长（演进）
产物**都会入库**：四处只承载非敏感行为开关、数值与经验文本。LLM key / token / 连接串 /
subagent 参数只在本地 `.env` / `.env.local`；Mod 资源与报告只出现键名。开局与复盘回执各带
一条红线提醒，逐条规则见 `guide.show credentials`。

## 文件

| 文件 | 作用 |
| --- | --- |
| `mod.lua` | Mod 本体：状态机（含节点图表）/ 动作协议 / 指南渲染 / 两层配置（含 `play.mode`） |
| `pack.json` / `manifest.json` | 打包声明与内容寻址清单（`cargo run --example pack` 生成） |
| `data/scenes.json` | 六节文本资源（scenes/endings/prompts/options/beats/tuning） |
| `data/config.json` | 静态配置默认值 |
| `data/playbook.json` | 现场规则资源（12 section / 74 条规则 + 命令/布局/指针） |
| `schemas/*.json` | 资源 schema（与 `packages/schema/schemas/` 单源一致） |

## 走查

```bash
# 装载 + 全 1 贪心走查到【结局
cargo run --example playtest -- .ygg/mods/agent-seed

# 门禁失败 → 打回 implement → 再全绿（verify 序号 2 = 如实报失败）
cargo run --example playtest -- .ygg/mods/agent-seed --choices 1,1,1,1,1,2,1,1,1

# 中途切 goal 挂机（intake 序号 4 = config.set play.mode=goal，之后按 1 推进；提示词切成挂机版）
cargo run --example playtest -- .ygg/mods/agent-seed --choices 4,1,1,1,1,1,1,1,1

# 门禁宽放（verify 序号 6 = config.set 关掉 require_all_green，再报失败）
cargo run --example playtest -- .ygg/mods/agent-seed --choices 1,1,1,1,1,6,2,1

# verify 态的选项顺序：1 全绿 / 2 报失败 / 3 guide.show / 4 map.show / 5-7 config.set
# （config.set 内部固定 play.mode → gates → evolution）
# 下面这条：verify 先看地图（4）→ 关掉门禁（6）→ 报失败（2）→ 宽放进复盘（1）
cargo run --example playtest -- .ygg/mods/agent-seed --choices 1,1,1,1,1,4,6,2,1

# 查一节规则（任意阶段可用；换成 guide.show 的默认载荷 = 本阶段默认节）
.ygg/bin/ygg-agent --mod .ygg/mods/agent-seed --run-id probe \
  --runs-root .ygg/runs --resume --event guide.show --payload '"gates"'

# 看剧情节点图（空 / 0 / full 同一张全图）
.ygg/bin/ygg-agent --mod .ygg/mods/agent-seed --run-id probe \
  --runs-root .ygg/runs --resume --event map.show --payload '"full"'

# 改完 data/ 或 schemas/ 必跑（manifest 内容寻址）
cargo run --example pack -- .ygg/mods/agent-seed
```
