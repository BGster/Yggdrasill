# agent-seed · 通用冷启动元工作流 Mod

M-Agent 泳道的 seed mod：把「一次完整的 Agent 工作」配置成一个可游玩的 Mod。
任何 Agent 装载它 + 注入任务描述即可开工，**零定制**——状态机、动作协议、门禁
口径、演进策略全在 Mod 里，不在提示词里。

```bash
# 装载 + 全 1 贪心走查到【结局
cargo run --example playtest -- packages/mods/agent-seed

# 门禁失败 → 打回 implement → 再全绿（回退路径）
cargo run --example playtest -- packages/mods/agent-seed --choices 1,1,1,1,1,2,1,1,1

# 门禁宽放（config.set 控制流转：失败也放行进复盘；verify 序号 4 = 关掉 require_all_green）
cargo run --example playtest -- packages/mods/agent-seed --choices 1,1,1,1,1,4,2,1

# verify 态的选项顺序：1 全绿 / 2 报失败 / 3 map.show / 4-5 config.set
# 下面这条：verify 先看节点图（3）→ 关掉门禁（4）→ 报失败（2）→ 宽放进复盘（1）
cargo run --example playtest -- packages/mods/agent-seed --choices 1,1,1,1,1,3,4,2,1
```

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

状态由 `ygg.fsm` 单机单事件 `stage.advance` 承载（payload = 目标态，边 = 上图
合法转移，`when` 按目标态白名单），落 KV `fsm.stage`，随存档往返。非法目标 =
静默 no-op；真正的门在 handler 里（阶段守卫 → 拒绝回执）。

## 动作协议

pendingChoice = **当前阶段允许的动作集**（协议约束，不是提示词约束）。选项文本
自带载荷形状，选中序号即派发同名动作事件；Agent 也可以直接派发事件（走同一套
守卫与副作用）。内核 payload 只能是 JSON 标量，故复合载荷压成单个字符串：

| 事件 | 载荷（字符串 wire format） | 迁移 |
| --- | --- | --- |
| `task.intake` | `title\|description`（空载荷 = 默认占位任务） | intake → decompose |
| `plan.submit` | `s1=标题; s2=标题; …`（1–8 片；只给标题时自动 id） | decompose → implement |
| `slice.report` | `slice_id\|summary\|artifacts`（artifacts 半角逗号分隔） | implement → implement / verify |
| `verify.submit` | `true` / `false` / `all_green=…;report=…` | verify → review（绿/宽放）/ implement（打回） |
| `review.note` | `lesson\|score(1-5)`（缺省分 3） | review → done |
| `config.set` | `settings.<key>=<value>`（`settings.` 可省） | 任意态，不改变主状态 |
| `map.show` | 空 / `0` / `full`（同一张全图） | 任意态，不改变主状态 |

**菜单顺序**：每态第一条 = 主前进动作（全 1 贪心走查可通关）；`map.show` 紧随主前进动作
（verify 态排在其失败分支之后），`config.set` 常驻且置末——只读动作总在改配置之前。

越阶动作以可读回执拒绝：`【动作被拒】` + 当前阶段允许的动作清单；载荷不合法为
`【载荷被拒】` + 具体原因。两者都**原地重渲染**（主状态不变、pendingChoice 保持）。

## 剧情节点图（map.show）

本 Mod 的「剧情」就是六态工作流，`map.show` 把状态机图渲染成 ASCII 节点图。图**不是手画**：
`mod.lua` 里 `MAP_NODES`（有序节点表）+ `MAP_EDGES`（带动作名的边表）是单源，渲染器逐节点取出边
——改状态机就改这两张表，不会出现图文不一致。不额外加资源/Schema（图是代码的结构事实，不是可
编辑的故事内容；文本词条仍在 `data/scenes.json` 的 `beats.map_*`）。

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
常驻动作（不推进主图，任意阶段可用）：config.set · map.show——任务/切片/门禁/配置一律不变
```

- **载荷**：空 / `0` / `"full"` → 同一张全图（内核 `--payload` 缺省值就是 `0`）；其他值 →
  `【地图被拒】` + 用法，主状态不变。目前不做局部视图（六态图本来就一眼看得完）。
- **当前节点**：头行 `>> [<stage>] << ← 你在这里` 与行首 `▶` 同源于 KV `fsm.stage`；有计划时头行
  额外附 `｜进度 X/Y 片`（`plan.done` / `plan.count`）。
- **不改状态**：与 `config.set` 一样是常驻动作，回执末尾重渲染当前阶段的 `pendingChoice`——不会
  吞掉主前进动作。

## 两层配置

- **静态默认**：`data/config.json`（schema `agent-config.json`，`ygg.data` 装载期快照）
- **动态覆盖**：KV `settings.*`（`config.set` 写入）
- **判定**：动态覆盖 > 静态默认

配置控制状态流转的两处实证：

1. `gates.require_all_green=false` → `verify.submit all_green=false` 也放行进 review
   （回执注明宽放路径）；`true` → 打回 implement，退回最后一片重做。
2. `evolution.mode` 决定局末复盘的修订回执：`auto` 已在白名单（prompts/beats）内
   登记自动修订请求、`suggest` 产出建议包、`off` 仅报告；额度由 `evolution.max_per_run`
   约束（运行期可用 `config.set evolution.max_per_run=<0-8>` 改），进入复盘的瞬间
   结算（`evolution.requested` 落 KV，不随后续 `config.set` 抖动）。`auto` 仅登记
   宿主可执行的自动修订请求；真实 revise 执行面待接，内核无 IO 不改文件。

## 文件

| 文件 | 作用 |
| --- | --- |
| `mod.lua` | Mod 本体：状态机（含节点图表）/ 动作协议 / 渲染 / 两层配置 |
| `pack.json` / `manifest.json` | 打包声明与内容寻址清单（`cargo run --example pack` 生成） |
| `data/scenes.json` | 六节文本资源（scenes/endings/prompts/options/beats/tuning） |
| `data/config.json` | 静态配置默认值 |
| `schemas/*.json` | 资源 schema（与 `packages/schema/schemas/` 单源一致） |
