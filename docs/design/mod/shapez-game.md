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

- **开局** `event.start`：`simtick.setup`（schema 注册 + 定点常量 + hub 装配）；载荷串 = 地图种子（首局落 `sz.seed`，first-wins；R6 禁缺省熵，缺省也是钉死常量 `shapez-v2`）。
- **放置** `sz.place.hub/belt/proc/miner`（载荷 `"x,y,rot"` / `"kind|x,y,rot"`）：实体落格 + `sz.layout_v` 版本 bump；矿机必须落在资源斑圆内（v2 = mapgen 分块 procgen，懒生成由放置触发）。
- **tick** `sz.tick`（载荷 `"n"`）：一拍 = ①处理器/矿机（出料→投产/推进）②传送带（路径推进+末端弹射）→ `sim.step(1)` + `sz.tick` +1；每 N 拍落锚 `sz.anchor.<tick> = procgen.hash(snapshot())`。
- **交付**：hub 收任意非空形状；**只有命中当前级目标**（规范形串相等）才计数（源 handleShapeDelivered 同构）；满额过级 = `quest.advance`（本级恰一次 I3）+ 新级 quest 实例 + 计数归零；末级照增进 freeplay（源 computeNextGoal 语义）。

**降级诚实账**：裸 KernelFacade（selftest/playtest）未挂 sim——首拍叙述一行 `[SIM-UNMOUNTED]`，KV 计拍照走（superagent clock 同款纪律）。

## 2. 数据形状

**shortkey 即规范形**（源 `serialize() == getHash()` 同位语义）：层自下而上以 `:` 相接；每层恰 8 字符 = 4 象限 ×（子形 `R/C/S/W` + 颜色 `r/g/b/y/p/w/u`）；象限序 `0=tr 1=br 2=bl 3=tl`；空象限 `--`；全空层非法。空项 = 空串 `""`（仅切割空半的传输占位，`is_valid("") == false`）。Lua 内部表示 = 层数组 `[1..n]` × 象限表 `[0..3]`（配对常量，不碰 `#`；R2）。

**Q32.32 定点**（R1：源全浮点 → 整数定点，黄金值自建不逐字节复刻）：raw = 值 × 2³²，全程 64 位整数 `//`。钉值（60Hz）：`spacing_q = 630×2³²//1000 = 2705829396`；`belt_step_q = 2000×2³²//60000 = 143165576`；`miner_step_q = 400×2³²//60000 = 28633115`（矿机 151 拍/件：150 拍差 46）；cutter `= //240000 = 35791394`（121 拍/件：120 拍差 16）；stacker `//480000 = 17895697`；rotater = belt_step；trash 即时。全部由 `belts.constants(tuning)` 从 `tuning.*Milli` 纯整数推导。

**方向系**：rot `0=+x 1=+y 2=-x 3=-y`（y 向下，屏幕系）。入料口 = 背面格，出料口 = 正面格。

**entity schema（v1 落 4 个）**：`sz.belt`（x/y/rot）· `sz.processor`（kind/in1/in2/progress/out1/out2）· `sz.hub` · `sz.miner`（shape/progress/out1）。计划「6–8 schema」为 v1–v3 合计口径：static 吸收进各实体 attrs、ejector/acceptor 吸收为 in*/out* 槽（计划 §2 映射表同口径）；v2 补 balancer/splitter 时再增。

**KV 键面（`sz.*` 单写者）**：`sz.tick` / `sz.level` / `sz.got` / `sz.layout_v` / `sz.setup` / `sz.hub_id` / `sz.level_quest` / `sz.path.<startid>`（codec.encode 的 `{s=spacing_q, i={{d,k}…}}`） / `sz.anchor.<tick>`（16 位 hex）；v2 增 `sz.seed` / `sz.mapgen_v`（进档）与 `sz.chunk.<cx>.<cy>`（分块缓存，**不进档**）。

## 3. 模块图（多文件纯度：mod.lua 装配，src/** 纯导出）

| 文件 | 职责 | 源对应 |
| --- | --- | --- |
| `src/shapes.lua` | shortkey 编解码 + 切割/堆叠/旋转纯函数 | shape_definition(.manager).js |
| `src/belts.lua` | 路径链重建 + 定点推进 + KV 持久化 | belt_path.js + systems/belt.js |
| `src/processors.lua` | miner/cutter/stacker/rotater/trash | item_processor.js + miner/hub 系统 |
| `src/hub.lua` | 交付计数 + 过级 + quest 实例 | hub_goals.js |
| `src/simtick.lua` | tick 信封 + 落点裁决 + 快照/锚 | game_root tick 循环 |
| `src/mapgen.lua`（v2） | 分块 procgen + `patch_at` + `sz.seed`/`sz.mapgen_v` 守护 | map_chunk.js |
| `src/savegame.lua`（v2） | 存档信封 save/load + 三重对账 | （源存档面的包面等价物） |

**落点裁决**（simtick.resolve）：同格多实体取 id 最小者；belt→邻路收件口（跨路径移交）、hub→交付、processor→入料槽、空项只进邻带/垃圾。

## 2.5 v2 分块地图（map_chunk.js 等价面；`src/mapgen.lua` 纯 Lua over procgen 包面，包面零改）

**生成**：每 chunk 一条命名流 `chunk.<cx>.<cy>`，`procgen.run(def.<cx>.<cy>, sz.seed, stream)`——流隔离（procgen I2）即分块隔离，与源 `x.."|"..y.."|"..seed` 同构。算法经 `procgen.algo("shapez-chunk", fn)` 注册于 `simtick.setup`；chunk 上下文（cx/cy）只能走 def.params（registry.run 只传 def.params 给算法，流名对算法不可见），故 def 按块运行期 `procgen.define`（D4 运行期 def 不跨存档无碍：产物落 KV，读档不靠 def）。

**数据形状**：patch = `{x, y, subshape, color, circles}`（chunk 本地坐标），circles 元素 = `{cx, cy, rx, ry}`（圆心 + 整数半轴）；命中判据整数域 `(dx·ry)² + (dy·rx)² ≤ (rx·ry)²`。**着色优先序**（源先色斑后形斑、先到先得）：patch 表保生成序，`patch_at` 按「patch 序×圆序」首中即胜（与源 lowerLayer 首漆者赢的点查等价）。

**整数化钉序**（黄金值自建，不逐字节复刻源）：R1' 源椭圆浮点 scale 0.9..1.1 弃用 → rx/ry 各自 ±1 整数抖动（钳 ≥1）；chance 千分位 `900 − clamp(dist·20,0,500)` 用 `rng:int(0,999)` 抽；块距 `round(hypot(cx+0.5,cy+0.5))` 的整数同构 = `(isqrt(4n+2)+1)//2`（n=cx²+cx+cy²+cy）；权重显式数组喂 `rng:weighted`（R3' pairs 无序）；风车不成整斑（源 windmillCount 守卫同构）。抽取序 KAT 钉死：预定义短路 → chance（色） → chance（形） → 每圆 rx→ry→cx→cy。

**预定义五斑**（源 generatePredefined，撑 level 1/2/3 目标可达）：(0,0) 红斑锚(7,7)、(-1,0) Cu 锚(7,7)、(0,-1) Ru 锚(5,9)、(-1,-1) 绿斑不钉位、(5,-2) Su 锚(5,9)——钉值入 test-e KAT。

**正交纪律**：chunk 只供矿机落点与产形（`{subshape,color}` 两元沿 v1，`shapes.miner_shape` 零改）；不参与路径推导、不 bump `sz.layout_v`、tick 热路径零地图开销；懒生成由放置事件触发；产物落 KV `sz.chunk.<cx>.<cy>`（codec.encode 的 `{v,patches}`），缓存版本不符 `MAPGEN_V` 视同未中重生成；**负坐标 floor 除 `//` 全程禁 `%`**（R2'）。

## 2.6 v2 存档信封（P4 superagent 信封先例；`src/savegame.lua`）

**信封** = `codec.export(ns)`，`ns={entities=true, ids="index", keys=点名键集, mod="shapez"}`。点名键三族（D9 显式点名；ygg.kv 无遍历面 → 动态键全部构造性枚举）：① 静态键 sz.tick/level/got/layout_v/setup/hub_id/level_quest/seed/mapgen_v；② 锚键 `sz.anchor.<t>`，t=anchorEveryTicks·j ≤ sz.tick（算术枚举）；③ `quest.keys()` 自报清单。`sz.path.*`（ensure 可重建）与 `sz.chunk.*`（seed 唯真源，惰性重生成）**不进档**——源 BaseMap schema 本就只存 `{seed}`，同构先例。

**协议**：`save(path)`=codec.write（封签；未开局拒）；`load(path)`=codec.read（强校）→ 守卫（`sz.setup` 在场即拒——防误载脏 KV；exported_from/mapgen_v/seed 三检）→ schema 预注册（import 校验需 spec，不落 sz.setup）→ `codec.import(bundle,{on_conflict="overwrite"})`（保 id：空局升序导入致密分配自然相等）→ **kv 段由 mod 逐键回写**（codec D9 包不写回；quest.* 键逐字节还原包的 KV 投影，对账兜底）→ `simtick.mount` → 三重对账：复导出（checksum 域归一后）逐字节 + 实体计数 + 锚拍快照哈希（锚拍存档硬校，非锚拍记 `snap_checked=false`）。

**判据口径**：往返逐字节 = write 回执字节 == 读回重编码；辅 `codec.roundtrip(path).equal` 双保险。B 局载入续跑（A/B/C 三局）：B@k 快照哈希==A@k、B≡C 于 [k..N] 全锚、终态快照逐字节、传输零损（文件字节面）。**剧本约束**：k 拍前零传送带——路径态是唯一不可重建的易变态，k 前不放带则逐字节判据与「path 不进档」同时成立；带链在 save 后双机同日志放置。

## 2.7 v3 长局压测台与拆除面（S26-3；`src/bot.lua` + `sz.demolish`）

**拆除面**（mod 直发事件唯一新入口）：`sz.demolish` 载荷 `"kind|x,y"`，kind ∈ belt|proc|miner（hub 不拆）；同格多实体取 id 最小者（simtick.resolve 同款纪律）；`entity.destroy` + `bump_layout`。kv 无删除面 ⇒ `pkg.entity.rec.<id>.schema` 墓碑（每 destroy 恰留 1 键，entity.md S9 口径）与 `sz.path.<startid>` 失效带是巡检对象。拆中段带 = 路径按新起点重建，带内运输件随旧路径态散失（源拆带散落同构）。

**bot 决策源**：逐拍独立 procgen 流 `bot.<世界拍>`（流隔离 = 无跨拍 rng 状态；流种子 = `sz.seed`）⇒ 决策 = f(seed, tick, 世界态)，存读档与重放天然安全（R6'）。决策表（`rng:weighted`，数组声明序即池序）：带链延伸 35 ｜ 拆带（尾端）25 ｜ 放处理器（miner 斑内 / cutter·stacker·rotater·trash 轮转）30 ｜ 存档点标记（纯读快照探针，不落 KV）10。KAT 钉值 @2000 抽（seed=`s26-v3-golden`）：extend 709 / demolish 493 / place 600 / savepoint 198。bot **无 KV 状态键**——决策序不落日志，同 seed 双局逐锚 + 终态快照比对即重放闭环；首带种子格 (50..53,50) 撑无带世界的确定性落格。

**长局剧本**（`bot.run(cfg)`）：决策 tick 段（流名与存档节拍一律用世界拍——续跑局从 k+1 同流续推）→ 拆建循环段（`bot.churn`，纯 spawn/destroy @1000+ 网格，不 tick）。闭环模式（`closed`）= **零销毁轨**：demolish 全程空转、首存拍（含）前 extend 空转 ⇒ 每个存拍世界零带零洞（id 致密），save→load 续跑局的致密重排与原局逐位相等；周期存读档每 save_every 拍（save→reset→load→续跑，末拍不存）。探针注入制：clock/gc 由调用面给，模块零 `os`/`collectgarbage` 引用（生产装载安全）。

**五指标仪表盘**（selftest 普通 Lua 面）：① 残留键（census 分桶：rec 墓碑恰 +spawn 数 / attr 余键 =0 / 失效带）② 吞吐（codec.export 与 read+import 计时，fast 单发 / full median3）③ 内存分相（双 collect 后 count）④ tick 分布（逐拍 os.clock → P50/P99 + 均值桶）⑤ 回放哈希（500 拍锚 + 探针序列 + 终态导出逐字节）。落点：`PERF shapez.v3.*` 八行固定键 + INFO 注记；full 档读数入下表（生产沙箱 deny os/collectgarbage 且 kv 无遍历面 ⇒ 仪表盘只在 selftest 面成立）。

**压测巡检记录**（full 档实测；后续每次 P-K 复验重跑 full 档重测本表）：

| # | 指标 | 读数（full 档首测 2026-11-22） | 口径/方法 |
| --- | --- | --- | --- |
| ① | 残留键 | 主局 spawn=35 = rec 键增量 35（attr 余键 0）；churn 10⁴ 轮墓碑字节 328,898 B；失效带 path_dead=0 | census 分桶差分（ygg_kv_all 镜像；生产沙箱无遍历面） |
| ② | 吞吐 | export 200,965 entities/s（49.8ms @10⁴ 裸带，档 829,758 B）；read+import 3,105 e/s（3,220.7ms）；存读档轮 1,139ms/轮 @2×10³ 中档 ×100 轮 | codec 内存面；median3；不含 write/fs/sha256 下沉面 |
| ③ | 内存 | 长局分相 build 1,345 → final 1,843 KB（+497.5 KB，含 profile 数组自重）；泄漏画像 100 轮 △=−123.6 KB（负增长）；churn +3.5 MB/10⁴ 键（≈350B/destroy，可数） | 双 collect 后 collectgarbage("count") |
| ④ | tick 分布 | 闭环小世界 P50=105µs / P99=130µs，10 桶均值 0.027–0.111ms；高水位 @10⁴ 实体 P50=277,497µs / P99=356,849µs（**belts 路径序 O(P²) 支配**——非三线阈值，登记待排） | 逐拍 os.clock；mlua `Lua::new` 普通面 |
| ⑤ | 回放 | 同 seed 双局锚·探针·终态哈希逐值相等；闭环 B≡A 于 [k..N] 全锚 + 终态导出逐字节；终态哈希 `9b4869746f661426` | bot 流 KAT + savegame.load 三重对账 |

**S19 复验注记（2026-11-22，sha256 原生下沉后）**：full 档原命令重跑 6/0；PERF 八行 export 165,514 e/s、import 2,911 e/s、mem_final 1,840 KB、tick P50/P99 111/134 µs、终态哈希 `9b4869746f661426`——与首测同口径同带（export/import ±7% 运行噪声；selftest 普通面无 `ygg.codec`，sha256 恒走 `sha256.lua`，S19 加速面只在宿主注入侧，见 [codec.md §5](../packages/codec.md)），回放终态哈希与首测**逐字节相等**（回放锚不变）。契约面同分钟 A/B（codec.sha256 64KiB）：before 0.771/0.779 → after 0.787 MB/s（回退体不回退）；运行面 release 同面 4.20 → 239.12 MiB/s（56.87×）。

**三线阈值判定**（S26-3 首判；判据原文见计划 §3，三线全锚「成为验收瓶颈」语义）：

- **S23 残留键/内存：未过线**。泄漏画像 100 轮负增长（−123.6 KB）；拆建 KV 增速 ≈350B/destroy 可数可控；验收②（扣墓碑 △≤0.5%）可达。不转正 crates（entity.md S9 后续①维持挂账不触发）。
- **S19 sha256 下沉：过线 → 排期槽**。判据① 10⁴ 实体档 import 3,220.7ms ≥ 1.9s 基线 ✓；判据② 周期存读档总耗时（泄漏画像 113.9s + 主局存读 0.11s）≥ 长跑 tick 总耗时（≈11s，存档吃掉压测预算）✓。before = 3.22s @10⁴ 裸带档（口径：codec.read+import 内存面；基线口径差异待 P-K 复验对齐）。
- **S20 export 批式：未过线**。export 200,965 e/s ≫ 3,196 e/s 基线（单次 49.8ms ≪ 3.1s）——口径差异登记（本轮不含 write/fs；P-K 复验对齐口径后重测）。

> **full 档预算注记**：墙钟实测 2m37.7s ≤ 3min（方法：`time` 包裹同命令复跑，bot 决策确定性 ⇒ 读数逐值可复现）；fast 档 6.6s ≤ 8s。

## 4. 简化登记（逐条可断，selftest 钉住）

1. **链判定放宽**：后带不逆风即接（源要求静息朝向严格对齐 + 曲线变体；v1 无曲线段，曲线长度记 1.0 对源 0.78）。
2. **无压缩堆叠分支**（numCompressedItems*）：v1 间距下限 0.63 定点版恒成立。
3. **布局重建不迁状态**：按 startid 命中才保留路径态（KV 真源 + 带数/最大 id 指纹失效）。
4. **单口串行出料**：out1→out2 晋升、每拍每台至多一件（源双槽独立方向齐发）；出料仅在空闲拍（progress==0），加工期产物占槽不可射。
5. **矿机出料不受 progress 门**：progress 是产量累加器（产后余数保留，源 `miningProgress -= 1` 同构），非忙态。
6. **处理器开工即算产物**：输入齐 + 槽空 → 消耗输入、产物落槽、progress 起步（源在完成时产出；时序差已登记）。
7. **静态斑退役（v2）**：`data.patches` 两斑换 procgen 分块生成（§2.5）；斑表不再入 data/schema。
8. **无种子退役（v2）**：v1 无随机消费面；v2 `sz.seed` 显式（R6' 禁缺省熵）+ `sz.mapgen_v` 钉版本拒漂移（R5'）。
9. **形斑单色元（v2）**：源四象限混合定义（A,A,B,B）裁剪为 `{subshape,color}` 两元——`miner_shape` 面 v1 已钉死两元输入；混色留 v3。
10. **存档最小面（v2）**：mb 级长档（10⁴ 实体）不在本轮；带内运输件随 `sz.path.*` 不进档（k 前零带剧本约束下无损，见 §2.6）。
11. **拆带弃件（v3）**：`sz.demolish` 拆中段带 = 路径按新起点重建，带内运输件随旧 `sz.path.*` 态散失（源拆带散落同构）；失效带即巡检对象（§2.7）。
12. **闭环轨零销毁（v3）**：`savegame.load` 的「保 id」= 致密重排（importing.lua 头注：内核自 1 起致密单调分配），洞位 id 不在存读档契约内 ⇒ bot `closed` 模式全程禁 demolish、零件不上带；高频创销压力由非闭环轨（churn 段 + g3 双局）承担；多存点周期存读档的等价以「每存拍世界致密」成立（§2.7）。

13. **仪表盘 selftest 读数面（v3）**：生产沙箱 deny `os`/`collectgarbage`、kv 无遍历面 ⇒ 五指标只在 selftest 普通Lua面成立（R-b 同款）；bot 模块本体零探针引用（clock/gc 注入制，生产装载不受影响）。

## 5. 验收映射（计划 §3 v1 验收 + §5 v2 验收 + brief 四层）

| 验收 | 实现面 | selftest 用例 |
| --- | --- | --- |
| 形状 KAT（黄金值自建） | shapes.lua 纯函数 | test-a 10 用例（往返/切割源序/旋转闭包/堆叠 gap 算法/层数上限/错误码） |
| belt 10⁴ tick 重放（I3） | simtick 锚 + procgen.hash | test-b：双局 20 锚逐值相等 + 终态快照逐字节相等 + KV 锚对账 |
| 最小玩法场景过到 level 2 | 两线十实体布局 | test-z：1→2 矿机线 tick 界 + 事件恰一次；2→3 切割线 2×10⁴ 拍内；同动作双局终态哈希相等 |
| v2 分块地图（S26-2） | mapgen.lua 纯 Lua over procgen | test-e 11 用例：整数距离 KAT/种子守护/预定义五斑钉值/黄金 chunk KAT/同 seed 相等+跨 seed 否定（方法=双 seed 各 3×3 全量 encode 比对）/patch_at 内外重叠三态/负坐标 floor/零 layout bump/缓存哨兵/色斑先序/PERF chunkgen |
| v2 存档信封（S26-2） | savegame.lua codec 往返 | test-f 8 用例：往返逐字节+roundtrip 双保险/脏 KV 守卫/派生态不进档/A·B·C 三局闭环（①-⑤逐条）/幂等导入 created=0/篛改一字节拒 CHECKSUM/mapgen_v 不符拒/PERF save·load |
| PERF 基线 | selftest 普通 Lua 口径 | `PERF shapez.tick1k_ns`（两线八实体）+ `chunkgen_ns` + `save_ms/load_ms`（实测值见交付回执） |
| v3 长局压测（S26-3） | bot 台 + `sz.demolish` + 五指标仪表盘 | test-g 6 用例（双档）：决策流 KAT 恰等 + 比例钉值 / 残留键墓碑恰等 + attr 余键 0 / 同 seed 双局重放逐值 + 跨 seed 否定 / 周期存读档闭环 B≡A 全锚 + 终态逐字节 / 吞吐往返逐字节 + id 保真 / PERF 八行落点；full 档实测入 §2.7 巡检记录表 |

门禁：`cargo run -q --example selftest -- --mod packages/mods/shapez --packages packages/libs`（本 mod 52 用例 + 四包 103 用例）不回退 pkg 199/0、agent 79/0、superagent 183/0、egg 158/0、--lib 112/0；manifest 双绿（shapez + agent-seed）；零 `crates/**` 改动；docs-consistency + docs-ver 双门禁。full 档（显式命令，不进默认门禁）：`SHAPEZ_STRESS=full cargo run -q --example selftest -- --mod packages/mods/shapez --packages packages/libs --filter test-g`（预算实测见 §2.7 巡检记录注记）。

## 6. 设计依据（为什么这样落）

**三要素（§1.7）**：① 数据形状 = shortkey 规范形串 + 4 entity schema + `sz.path.*`/`sz.anchor.*` KV 键形；不变量 = 间距 ≥ 0.63 定点版、shortkey 层序 canonical、重放逐锚 hash 相等、本级 quest completed 恰一次。② API 面 = mod 内 5 模块 + 消费 `pkg.entity`（spawn/get/set/ids/define）、`pkg.codec`（encode/decode 持久化与快照）、`pkg.procgen`（hash 锚定）、`pkg.quest`（关卡实例）；**新宿主面 = 零**。③ 性能预算 = v1 单拍 ≤100µs @ selftest 口径（实测 tick1k_ns 入回执；包加载无新增面）。

**R1–R7 落点**：R1 浮点→定点（§2 钉值 + KAT）；R2 数组（象限 0..3 表内保留、层 1 基显式长度）；R3 字符串（byte 索引 `sub`、层拆分手写、拼接序 KAT 钉死）；R4 整数截断（全 `//`，无 floor/trunc 混用）；R5 无类继承（table+显式委托，superagent 方言）；R6 确定性（无 os.time/clock、seed 显式落 data、v1 无随机消费面——procgen.run 地形属 v2）；R7 GPL-3（行为参照非抄写，源仓不入检出，产物内部测试）。

**反方案否决**：物品上带不作实体（10⁴ tick × 实体化 = 属性袋写入放大，KV `sz.path.*` 批量态更省且天然可序列化）；处理器不做双槽独立方向（v1 场景为直线产线，串行口吞吐 ≥ 处理器周期上限，语义差已登记 §4）。

**v2 设计依据（S26-2，登记在案不编号）**：① 整数椭圆（R1'）：源 `nextRange(0.9,1.1)` 浮点半轴弃用——浮点跨机不可钉值；rx/ry 各自 ±1 整数抖动，判据 `(dx·ry)²+(dy·rx)²≤(rx·ry)²` 与源 scale-椭圆几何同构，黄金值自建。② seed 唯真源：chunk 不进存档（源 BaseMap `{seed}` 同构）；`sz.seed` 显式（R6' 缺省也是钉死常量，非熵）；`sz.mapgen_v` 进档拒漂移（R5'）——生成器升级后旧档 fail-closed 而非静默重铺。③ kv 回写归属：`quest.*` 键由 mod 逐键回写（备选 quest.open/advance 重放重建因事件多、时序敏感被否）；quest.keys() 是包自报的唯一权威枚举面，回写是其逆映射，三重对账（复导出逐字节含 quest 键）兜底漂移。④ def 按块运行期注册：chunk 上下文（cx/cy）只能走 def.params（registry.run 不传流名不收逐调参数）——计划字面 `run("shapez-chunk",…)` 调整为 `run(def.<cx>.<cy>,…)`，D4 运行期 def 不跨存档无碍（产物落 KV，读档靠 seed 惰性重生）。⑤ 存档 k 拍前零带：路径态是唯一不可重建易变态；剧本约束使其与「path 不进档」同时成立（§2.6），viewport/audio/balancer 出本轮待裁。

**v3 设计依据（S26-3，登记在案不编号）**：① 逐拍流隔离序钟：决策流名 `bot.<世界拍>`——续跑局以 KV `sz.tick` 对齐流序（save/load 后同流同决策，B≡A 的序钟基础）；决策表第一抽纯 f(seed,tick)（KAT 可钉），目标面吃世界态 + 同流后续抽（执行确定性）。② 闭环轨零销毁（§4 简化 12）：importing「保 id」语义实测逼出——洞位档 import 重排 id 后 RECONCILE 逐字节必败；多存点周期存读档的等价以「每存拍世界致密」为充要，零销毁轨是最小实现（备选「洞位 id 入档」= 改 codec 契约，否）。③ 稳态小世界 + 高水位 KV 分离：tick 热路径受 `entity.ids` O(seq) 水位扫描与 belts 路径序 O(P²) 支配（巡检记录④），万级压力由 churn 段（KV 面）与吞吐台（codec 面）承担，tick 段 caps 有界——压测台分离两项压力而非单一大世界硬壣。④ 阈值判定口径注记：S19 过线（两判据实测成立）与 S20 未过线的口径差异（export 不含 write/fs；基线口径待 P-K 对齐）随读数入册，复验对齐后重测；测得新信号 belts O(P²) 路径序非三线阈值，登记待排不自行转正（§1.7 载体归属不确定时停手）。⑤ 升级曲线裁剪（计划 §0）：bot 压测固定 tuning，升级改常量 = 给重放契约加第二个 regime，五项任务零收益。
