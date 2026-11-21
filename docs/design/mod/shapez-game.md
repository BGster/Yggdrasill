# Yggdrasill · shapez v1 核心循环（S26-1 移植复现）
> **id** mod/shapez-game · **模块** ③ mod 载体 · **触发** 改 `packages/mods/shapez/**` 任一行之前
> **读不到代价** silent-drift（定点口径 / 链判定 / 计数口径漂移，重放断言失效）
> **定位**：shapez.io v1 核心循环的游戏契约——形状规范形、定点推进、处理器语义、交付过级与验收映射。计划即规格（r1 rev 1，`/tmp/s26a-plan.md` §3 v1 层）；本文是它的就地治理文档。
> **上游**：[mod-package.md](mod-package.md) · [entity.md](../packages/entity.md) · [codec.md](../packages/codec.md) · [procgen.md](../packages/procgen.md) · [quest.md](../packages/quest.md)
> **载体**：`packages/mods/shapez/`（五件套 mod；设计目标零 `crates/ygg-*` 改动）。
> **源仓**：tobspr-games/shapez.io @ `8a79de5`（GPL-3⚠，内部测试；**等价复现非逐行抄写**，源仓不入检出）。

## 1. 形态与核心循环

headless 模拟面（事件直发驱动；v2 接视口/音频数据流）。核心循环：

**矿机产形 → 传送带定点推进 → 切割/堆叠/旋转 → hub 交付 → 过级**。

- **开局** `event.start`：`simtick.setup`（schema 注册 + 定点常量 + hub 装配）。
- **放置** `sz.place.hub/belt/proc/miner`（载荷 `"x,y,rot"` / `"kind|x,y,rot"`）：实体落格 + `sz.layout_v` 版本 bump；矿机必须落在资源斑圆内（`data/shapez-world.json` patches）。
- **tick** `sz.tick`（载荷 `"n"`）：一拍 = ①处理器/矿机（出料→投产/推进）②传送带（路径推进+末端弹射）→ `sim.step(1)` + `sz.tick` +1；每 N 拍落锚 `sz.anchor.<tick> = procgen.hash(snapshot())`。
- **交付**：hub 收任意非空形状；**只有命中当前级目标**（规范形串相等）才计数（源 handleShapeDelivered 同构）；满额过级 = `quest.advance`（本级恰一次 I3）+ 新级 quest 实例 + 计数归零；末级照增进 freeplay（源 computeNextGoal 语义）。

**降级诚实账**：裸 KernelFacade（selftest/playtest）未挂 sim——首拍叙述一行 `[SIM-UNMOUNTED]`，KV 计拍照走（superagent clock 同款纪律）。

## 2. 数据形状

**shortkey 即规范形**（源 `serialize() == getHash()` 同位语义）：层自下而上以 `:` 相接；每层恰 8 字符 = 4 象限 ×（子形 `R/C/S/W` + 颜色 `r/g/b/y/p/w/u`）；象限序 `0=tr 1=br 2=bl 3=tl`；空象限 `--`；全空层非法。空项 = 空串 `""`（仅切割空半的传输占位，`is_valid("") == false`）。Lua 内部表示 = 层数组 `[1..n]` × 象限表 `[0..3]`（配对常量，不碰 `#`；R2）。

**Q32.32 定点**（R1：源全浮点 → 整数定点，黄金值自建不逐字节复刻）：raw = 值 × 2³²，全程 64 位整数 `//`。钉值（60Hz）：`spacing_q = 630×2³²//1000 = 2705829396`；`belt_step_q = 2000×2³²//60000 = 143165576`；`miner_step_q = 400×2³²//60000 = 28633115`（矿机 151 拍/件：150 拍差 46）；cutter `= //240000 = 35791394`（121 拍/件：120 拍差 16）；stacker `//480000 = 17895697`；rotater = belt_step；trash 即时。全部由 `belts.constants(tuning)` 从 `tuning.*Milli` 纯整数推导。

**方向系**：rot `0=+x 1=+y 2=-x 3=-y`（y 向下，屏幕系）。入料口 = 背面格，出料口 = 正面格。

**entity schema（v1 落 4 个）**：`sz.belt`（x/y/rot）· `sz.processor`（kind/in1/in2/progress/out1/out2）· `sz.hub` · `sz.miner`（shape/progress/out1）。计划「6–8 schema」为 v1–v3 合计口径：static 吸收进各实体 attrs、ejector/acceptor 吸收为 in*/out* 槽（计划 §2 映射表同口径）；v2 补 balancer/splitter 时再增。

**KV 键面（`sz.*` 单写者）**：`sz.tick` / `sz.level` / `sz.got` / `sz.layout_v` / `sz.setup` / `sz.hub_id` / `sz.level_quest` / `sz.path.<startid>`（codec.encode 的 `{s=spacing_q, i={{d,k}…}}`） / `sz.anchor.<tick>`（16 位 hex）。

## 3. 模块图（多文件纯度：mod.lua 装配，src/** 纯导出）

| 文件 | 职责 | 源对应 |
| --- | --- | --- |
| `src/shapes.lua` | shortkey 编解码 + 切割/堆叠/旋转纯函数 | shape_definition(.manager).js |
| `src/belts.lua` | 路径链重建 + 定点推进 + KV 持久化 | belt_path.js + systems/belt.js |
| `src/processors.lua` | miner/cutter/stacker/rotater/trash | item_processor.js + miner/hub 系统 |
| `src/hub.lua` | 交付计数 + 过级 + quest 实例 | hub_goals.js |
| `src/simtick.lua` | tick 信封 + 落点裁决 + 快照/锚 | game_root tick 循环 |

**落点裁决**（simtick.resolve）：同格多实体取 id 最小者；belt→邻路收件口（跨路径移交）、hub→交付、processor→入料槽、空项只进邻带/垃圾。

## 4. v1 简化登记（逐条可断，selftest 钉住）

1. **链判定放宽**：后带不逆风即接（源要求静息朝向严格对齐 + 曲线变体；v1 无曲线段，曲线长度记 1.0 对源 0.78）。
2. **无压缩堆叠分支**（numCompressedItems*）：v1 间距下限 0.63 定点版恒成立。
3. **布局重建不迁状态**：按 startid 命中才保留路径态（KV 真源 + 带数/最大 id 指纹失效）。
4. **单口串行出料**：out1→out2 晋升、每拍每台至多一件（源双槽独立方向齐发）；出料仅在空闲拍（progress==0），加工期产物占槽不可射。
5. **矿机出料不受 progress 门**：progress 是产量累加器（产后余数保留，源 `miningProgress -= 1` 同构），非忙态。
6. **处理器开工即算产物**：输入齐 + 槽空 → 消耗输入、产物落槽、progress 起步（源在完成时产出；时序差已登记）。

## 5. 验收映射（计划 §3 v1 验收 + brief 四层）

| 验收 | 实现面 | selftest 用例 |
| --- | --- | --- |
| 形状 KAT（黄金值自建） | shapes.lua 纯函数 | test-a 10 用例（往返/切割源序/旋转闭包/堆叠 gap 算法/层数上限/错误码） |
| belt 10⁴ tick 重放（I3） | simtick 锚 + procgen.hash | test-b：双局 20 锚逐值相等 + 终态快照逐字节相等 + KV 锚对账 |
| 最小玩法场景过到 level 2 | 两线十实体布局 | test-z：1→2 矿机线 tick 界 + 事件恰一次；2→3 切割线 2×10⁴ 拍内；同动作双局终态哈希相等 |
| PERF 基线 | selftest 普通 Lua 口径 | `PERF shapez.tick1k_ns`（两线八实体，实测值见交付回执） |

门禁：`cargo run -q --example selftest -- --mod packages/mods/shapez --packages packages/libs`（本 mod 26 用例 + 四包 103 用例）不回退 pkg 199/0、agent 79/0、superagent 183/0；manifest 双绿（shapez + agent-seed）；零 `crates/**` 改动。

## 6. 设计依据（为什么这样落）

**三要素（§1.7）**：① 数据形状 = shortkey 规范形串 + 4 entity schema + `sz.path.*`/`sz.anchor.*` KV 键形；不变量 = 间距 ≥ 0.63 定点版、shortkey 层序 canonical、重放逐锚 hash 相等、本级 quest completed 恰一次。② API 面 = mod 内 5 模块 + 消费 `pkg.entity`（spawn/get/set/ids/define）、`pkg.codec`（encode/decode 持久化与快照）、`pkg.procgen`（hash 锚定）、`pkg.quest`（关卡实例）；**新宿主面 = 零**。③ 性能预算 = v1 单拍 ≤100µs @ selftest 口径（实测 tick1k_ns 入回执；包加载无新增面）。

**R1–R7 落点**：R1 浮点→定点（§2 钉值 + KAT）；R2 数组（象限 0..3 表内保留、层 1 基显式长度）；R3 字符串（byte 索引 `sub`、层拆分手写、拼接序 KAT 钉死）；R4 整数截断（全 `//`，无 floor/trunc 混用）；R5 无类继承（table+显式委托，superagent 方言）；R6 确定性（无 os.time/clock、seed 显式落 data、v1 无随机消费面——procgen.run 地形属 v2）；R7 GPL-3（行为参照非抄写，源仓不入检出，产物内部测试）。

**反方案否决**：物品上带不作实体（10⁴ tick × 实体化 = 属性袋写入放大，KV `sz.path.*` 批量态更省且天然可序列化）；处理器不做双槽独立方向（v1 场景为直线产线，串行口吞吐 ≥ 处理器周期上限，语义差已登记 §4）。
