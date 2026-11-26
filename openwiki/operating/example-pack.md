---
type: "Reference"
title: "参考包 — packs/example 逐文件讲解"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T07:10:30.723Z
sources:
  - id: openwiki-source-d45758f359e131762be9742d
    resource: repo://packs/example/data/board.json
  - id: openwiki-source-2c5400885f18104f706a3405
    resource: repo://packs/example/pack.json
  - id: openwiki-source-56b902e6771442837186415e
    resource: repo://packs/example/src/ai.lua
  - id: openwiki-source-643615af918693ff05b2df97
    resource: repo://packs/example/src/board.lua
  - id: openwiki-source-88acf093bd8533ac1f3048fd
    resource: repo://packs/example/src/combat.lua
  - id: openwiki-source-533eaac139cebd8db482c862
    resource: repo://packs/example/src/command.lua
  - id: openwiki-source-be3c0a4489d552e4f50bae0f
    resource: repo://packs/example/src/main.lua
  - id: openwiki-source-6a1f2ceb3386d6bd09cfc4b5
    resource: repo://packs/example/src/turn.lua
  - id: openwiki-source-f5be37635801335a2d02a84c
    resource: repo://packs/example/src/units.lua
  - id: openwiki-source-d0bfaf08595e0b573250de8c
    resource: repo://packs/example/test/turn.lua
generated: { by: "pi", at: "2026-11-26T07:10:30.723Z" }
---

# 参考包 — packs/example 逐文件讲解

> **id** `operating/example-pack` · **模块** 操作 · **触发** 第一次写包、想找「这条契约在真实代码里长什么样」
> **读不到代价** `silent-drift`（照错的范式抄包 → 抄进一堆不需要的抽象，或者漏掉 `seal`/命令面契约）

参考包刻意小：10×5 棋盘、两个 5 人队、每 tick 一个单位行动、相邻自动互击。它不是游戏，是**七个契约的可执行样例**。

## 1. `src/main.lua` — 入口 chunk 与装配序

```lua
local units  = require("src.units")
local board  = require("src.board")   -- 数据在 data/board.json，经 ygg.data 取
local turn   = require("src.turn")
local ai     = require("src.ai")
local combat = require("src.combat")
local command= require("src.command")

units.declare()                       -- ① 声明组件
local field = board.new(ygg.data("ygg:example/board"))   -- ② 造世界（spawn 全部单位）
turn.install()                        -- ③ 注册系统
ai.install(field)
combat.install()
world.seal()                          -- ④ 封盘（此后再注册即 SEALED）
world.commands = command.install(field)  -- ⑤ 命令面
return { commands = world.commands }     -- ⑥ 返回值就是声明面
```

演示的契约：`contracts/kernel/packaging` §4（装载序）、`contracts/kernel/world` §5（`seal` 时机）、`contracts/kernel/lua-api` §3（入口返回值即命令面）。

## 2. `src/units.lua` — 组件声明与初始摆放

四个组件：`pos`（i64，`x*1000+y` 编码）、`hp`（i64）、`team`（i64）、`moved`（i64，0/1）。`deploy(field)` 每行最左空格给 0 队、最右给 1 队。

演示：`world.component{name,kind}` 只声明一次；「一列一语义」——`moved` 是独立列而不是塞进 `pos` 的高位，因为 delta 面靠 `(eid,cid)` 定位改动。

## 3. `src/board.lua` — 数据驱动与坐标编码

`ygg.data("ygg:example/board")` 读 `data/board.json`（10×5，中间四格是墙）。`encode(x,y)=x*1000+y`、`decode`、`in_bounds`、`open`、`adjacent`、`distance`；`DIRECTIONS` 顺序固定为东南西北（决定 AI 并列时的选择序）。

演示：规则包里没有文件 IO，数据只经 `ygg.data`；坐标编码进 `i64` 而不是两个组件——因为「位置」在一次移动里是**一次原子写**。

## 4. `src/turn.lua` — 系统与 tick 内的行动序

注册 `stage="rules"` 的系统：每 tick 取 `actor = tick % count + 1`，只让这一个单位行动（`moved=0` 的才动），跑完把 `moved` 复位。

演示：`world.system{name,stage,run,reads,writes}` 的 `writes` 必须诚实——`seal` 会校验同阶段写写冲突；行动序是 `tick` 的函数，因此不依赖注册序之外的隐式状态。

## 5. `src/ai.lua` — 确定性决策

`step_toward(host, me, taken, goal)`：距离 ≤1 就原地（不再侧移）；否则在四个方向里挑**严格更近**的空格；并列时用 `host.rng("ai.move")` 取一格。

演示：`contracts/kernel/determinism` §6——命名流按名派生，AI 的并列选择可复现；「原地」分支是修过的缺陷（旧版贴脸时会左右横跳）。

## 6. `src/combat.lua` — tick 边界结算与最小写面

`resolve` 统计相邻的敌对单位各扣 1；**没有任何一对相邻时一格都不写**，避免 delta 虚胖；hp 归零用 `ctx.despawn`，在 tick 边界落地。

演示：`contracts/kernel/world` §2（结构写延迟到 tick 边界）、§6（`changed` 只含真实写）。

## 7. `src/command.lua` — 命令面与拒绝语义

`move(args)`：要求整数 `dx/dy` 且 `|dx|+|dy| == 1`（正交一步），目标格必须开放且无人；`attack(args)`：目标存在、敌对、相邻；`end_turn` 清 `moved`。

拒绝一律 `false, "<原因>"` 且**不改状态**（内核在调用前后各复原一次）。演示：`contracts/kernel/lua-api` §3。

## 8. `test/turn.lua` — 12 个用例

坐标往返、相邻判定、越界拒绝、占位拒绝、攻击前提、hp 下限、`end_turn` 清标记、50 tick 轮转、真实棋盘铺子形状、两次装载确定性、非法步距拒绝、贴脸不横跳。

跑：

```bash
cargo run -q --example selftest -p ygg-sandbox -- --mod packs/example      # 12/12
cargo run -q --example golden   -p ygg-sandbox -- packs/example --check packs/example/golden.json
cargo run -q --example image    -p ygg-host    -- packs/example --check hosts/h5/pack.image.js
```

## 9. 这个包故意**没有**做的东西

渲染、输入、音频、存档格式、网络、多包组合、热重载。它们在宿主侧（`hosts/h5`）或不在 v1 范围——包只证明「规则侧契约够用」。

## 10. 维护责任

- **触发**：改任一 Lua 模块（须重跑 `example pack` 重算 manifest，并重生成 `golden.json` 与 `pack.image.js`）、加用例、改棋盘数据。
- **检查**：§8 三条命令 + `cargo test -p ygg-sandbox --test example_pack`。
