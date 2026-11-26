---
type: "Reference"
title: "TLA+ 模型移植（黄金轨道对账台）"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-73577cecfc0fd0ac7e47400c
    resource: repo://packages/mods/tla-models/data/traces/ewd998.json
  - id: openwiki-source-ed190db059f205b3b9b34ec2
    resource: repo://packages/mods/tla-models/data/traces/gol.json
  - id: openwiki-source-101e11913bc35cffe640fb4d
    resource: repo://packages/mods/tla-models/data/traces/twophase.json
  - id: openwiki-source-94c2fa3309be3abad136de02
    resource: repo://packages/mods/tla-models/mod.lua
  - id: openwiki-source-ed3b7dc9b19e36b38c8f9bf5
    resource: repo://packages/mods/tla-models/pack.json
  - id: openwiki-source-a34051db80240cb0e5c251cc
    resource: repo://packages/mods/tla-models/src/ewd998.lua
  - id: openwiki-source-a578bb6bfb1992ffc10e254d
    resource: repo://packages/mods/tla-models/src/gol.lua
  - id: openwiki-source-7ee805264d031360b83a3114
    resource: repo://packages/mods/tla-models/src/inv.lua
  - id: openwiki-source-3a5f89b8eb43512f012ddaf5
    resource: repo://packages/mods/tla-models/src/klotski.lua
  - id: openwiki-source-b24f819ef42bd318415afee0
    resource: repo://packages/mods/tla-models/src/replay.lua
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# TLA+ 模型移植（黄金轨道对账台）

> **id** mod/tla-models · **模块** ③ mod 载体 · **触发** 改 `packages/mods/tla-models/**` 任一行之前
> **读不到代价** silent-drift（映射口径 / 哈希链口径 / 不变量语义漂移，黄金轨道断言失效）
> **replaces**：DESIGN §3.x tla-models 节（搬运基线 `3a2536f`）
> **上游**：[mod-package.md](mod-package.md) · **相关**：[../packages/entity.md](../packages/entity.md) · [../packages/codec.md](../packages/codec.md) · [../packages/procgen.md](../packages/procgen.md) · [testing-strategy.md](testing-strategy.md)

## 1. 形态与轨道驱动

headless 模型台（事件直发驱动），实测零 `crates/ygg-*` 改动。**注意口径**：这里跑的不是「内核的 TLA 规格」，而是拿 TLA+ 教科书模型压测内核面（entity / KV / tick / 快照哈希）——它们用外部形式规范反向校验引擎语义。**模型只做「解释器 + 轨道对账」，不做全量状态枚举**——枚举是 TLC 的活，做完验收就膨胀了。

- `reset(model)` 建规范初态（`Init` 的非确定域一律钉死，见 §4）。
- `step(model, action)` 执行一次 TLA 动作：`\E` 非确定性显式化为**动作选择输入**（与游戏输入同位）；**stutter = 无动作的显式步型**（`[][Next]_v` 的 `v'=v` 分支，哈希不变）。
- **每拍** `procgen.hash(snapshot())` 落哈希链（`tla.hash.<model>.<tick>`），每 4 拍落 KV 锚 `tla.anchor.<model>.<tick>`。哈希是**纯状态指纹**（不掺 tick / 前值——掺链会让 stutter 锚漂移）；「链」= 逐拍指纹序列。
- `inv.check(model)` 返回违例清单（空 = 绿）；不变量注册表在 `src/inv.lua`（断言抛错 fail-closed 记违例，不伪装绿）。
- **双局同轨重放**：逐锚相等 + 终态快照逐字节相等（快照表深相等 + `codec.encode` 字节相等双半断言）。
- `auto_tick`：`f(seed, tick, state)` 纯自选步（`procgen` 命名流派生 `name.."#"..tick`），从启用动作集按 `next32 % n` 取一；双局同 seed 逐锚必等。

**事件面**：`event.start`（全模型钉初态）/ `tla.reset`（`model|all`）/ `tla.step`（`model|action`）/ `tla.auto`（`model|n`）/ `tla.check`（裁决叙事）/ `tla.summary`（meta 摘要）。错误全 fail-closed：守卫不满足 `YGG-MOD-TLA-GUARD`、未知动作 `YGG-MOD-TLA-ACTION`、CHOOSE 不唯一 `YGG-MOD-TLA-CHOOSE`、平移撞件 `YGG-MOD-TLA-BLOCKED`。

## 2. 数据形状

| schema | attrs | 模型 | 源变量 |
| --- | --- | --- | --- |
| `tla.cell` | x:number, y:number, alive:boolean | gol | `grid[Pos → BOOLEAN]`（16 格） |
| `tla.piece` | kind:string, ox:number, oy:number | klotski | `board`（十件集，cells 由 kind 偏移展开：v2/h2/sq/u） |
| `tla.node` | id:number, active:boolean, color:string, counter:number, pending:number | ewd998 | `active/color/counter/pending` |

**KV 键形（`tla.*` 单写者）**：`tla.tick.<m>` / `tla.hash.<m>.<tick>` / `tla.anchor.<m>.<tick>` / `tla.tp.p|c|x` / `tla.ewd.token.pos|q|color`。KV 无删除面：reset 只重置 `tla.tick` 并覆盖写，更短轨道的旧 tick 键不入对账读面（verify 只读 tick ≤ 当前）。

**轨道 JSON**（`data/traces/*.json`，schema `schemas/tla-trace.json`）：`{model, track, method:"KAT", source, note, init?, actions[], hashes[16hex], anchors{tick→16hex}, final}`——动作向量黄金轨道；entry id `ygg:tla/trace/<model>`。

**快照规范序**（哈希 KAT 面）：gol cells 按 (y,x)；klotski pieces 按 (ox,oy,kind)；ewd998 nodes 按 id + token 三键。

## 3. 模块图（mod.lua 装配，src/** 纯导出）

| 文件 | 职责 | 行数 |
| --- | --- | --- |
| `src/gol.lua` | GameOfLife N=4：8 邻域计票（每代一次 attrs 批读） | 200 |
| `src/twophase.lua` | TwoPhase 握手：KV 标量 + 交替守卫 + vBar 投影 | 112 |
| `src/klotski.lua` | SlidingPuzzles 4×5：件形偏移 + move/update + CHOOSE 唯一化 | 359 |
| `src/ewd998.lua` | EWD998 N=2：六动作 + Safra 归纳不变量 + 终止检测 | 380 |
| `src/replay.lua` | 轨道驱动：哈希链 / 锚 / 双局对账 / 篡改面 / auto | 279 |
| `src/inv.lua` | 不变量注册表（register / check / names / known） | 86 |

## 4. 映射表（`VARIABLE` / `Init` / `Next` / `Inv` → 引擎面）

| 源构造 | 引擎落点 | 备注 |
| --- | --- | --- |
| `VARIABLE` 全局小态 | KV 标量（`tla.tp.*`、`tla.ewd.token.*`） | 每身份态 → entity（三 schema） |
| `Init` 非确定域 | 钉规范初态 | gol 全灭（pattern 走 reset opts）；ewd 全 active / 全 white / token(0,0,black)；twophase (0,0,0)（`XInit: x≡0`） |
| `Next` 析取 | `step(action)` 显式动作 + `step_auto` 纯 `f(seed,tick,state)` | 守卫假 = 动作不使能 → GUARD 错（轨道选了即轨道错误） |
| `[][Next]_v` stutter | 显式步型（`action = nil/"stutter"`） | 哈希不变（纯状态指纹口径） |
| `Inv` / `TypeOK` | inv.lua 注册表，运行期断言 | 无类型系统 → TypeOK 全数移植（整数纪律手写 `is_integer`） |
| `CHOOSE` 唯一化 | 命中件数 ≠ 1 必报 `YGG-MOD-TLA-CHOOSE` | 不静默取第一个 |
| WF/SF 公平性与活性 | **裁剪，safety-only** | 活性不可由有限自检等价覆盖（[testing-strategy.md](testing-strategy.md) §3 同理） |
| TLA `%` vs Lua `%` | 右操作数为正且域无负输入时同值 | {0,1} 域 KAT 钉死（twophase cycle4） |

| 模型 | TLA 惯用法 | 不变量 | TLC 实测（源仓记录） |
| --- | --- | --- | --- |
| GameOfLife N=4 | 确定性 CA（唯一后继） | TypeOK | 65536/131072/1 |
| TwoPhase（MCTwoPhase 实例） | 交替同步 + 抽象参数实例化 | Inv（p,c∈{0,1}） | 4/5/4 |
| SlidingPuzzles（Klotski 4×5） | 组合谜题 + 目标反用 + CHOOSE 唯一化 | TypeOK + KlotskiGoal | 不全枚举（源 runtime 5s） |
| EWD998 N=2 | 分布式终止检测 + 令牌有界 | TypeOK + Inv（P0∧P1∨…∨P4）+ TerminationDetection | 4097/53271/14（源仓 AsyncTerminationDetection N=4 档） |

**落选登记**：MultiCarElevator（动作面最大，留后续轮）；ewd998 其余变体与 TLAPS 证明模块（不移植）。

## 5. 语义子集边界（风险清单，逐条可断）

1. **仅移植「有限态 + 显式 Next 析取 + 安全不变量」子集**；活性 / 公平性 / TLAPS 证明 / 无限态实时模型一概不选。
2. **`Init` 非确定域裁剪**（exploration ≠ replay）：钉规范初态。
3. **TypeOK 运行期加强两处**（TLA 集合嵌套表达不出、实体形状能）：gol 同格双实体；klotski 件 cells 两两不交。
4. **klotski 步数口径**：格步（TLC trace 语义）≠ 源注 81 步计数（多格滑步计一）。
5. **不做全量可达断言**：模型台是解释器 + 对账，不是模型检查器。
6. **哈希 canonical 序**：快照键序 + 实体 id 序（reset spawn 序）KAT 钉死防漂移。
7. **KlotskiGoal 保持原语义**（目标态 ∉ board 的反用）：2×2 方块抵达底中四格 = 违例，测试断言「初始绿」。
8. **probe / pass 反直觉语义逐字对齐**：probe 守卫 =「前轮不结论」才点火；pass 把黑节点色传给令牌（N=2 轨道 t6 令牌变黑，须第二轮探测达检测态）。

## 6. 黄金轨道（哈希对账口径）

**A 法（TLC `-dumptrace` 对账）不可达**：实测 2026-11-22 `tla2tools` 在 GitHub releases 与 Maven Central 两个 URL 均 HTTP 404。**走 B 法并如实声明**：

- **KAT 手算步表**：TwoPhase 4 态循环（00→10→11→01→00）；Life blinker / block / toad（Python 独立推导 + 手算双推导一致）；Klotski 开局三步（`u(1,3)→(1,4)`、`u(2,3)→(2,4)`、`h2(1,2)→(1,3)`，各步 free 过滤手工核验）；EWD998 N=2 十动作轨（Python 推导含每步 Inv，TD 首真 @t8）。
- **哈希冻结**：逐拍指纹 = `procgen.hash(snapshot())`（FNV-1a 64 over `codec.encode`，16 hex）真算落 `data/traces/*.json`；语义 = 回归锁（语义漂移即冻结值破）。测试同时断言：实现状态 == 手算 KAT 表（**语义正确性**）∧ 实现哈希 == 冻结值（**漂移检出**）——两线分离，缺一不可。
- **篡改红三型**：`replay.tamper` 的 hash1 / final / anchor 三变异面各断言对账必败，同参不篡改 = 绿（红例必须真红）。

## 7. 性能预算（实测改定）

计划原估「单拍 ≤100µs、10³ 拍重放 ≤10ms」；实测（普通 Lua 口径，`os.clock` 1k 拍均摊）：

| 键 | 实测 | 达标 |
| --- | --- | --- |
| `tla.gol_tick1k_ns` | 332,163 ns/拍 | 阈 ≤1ms ✓（计划 100µs ✗） |
| `tla.klotski_tick1k_ns` | 315,953 ns/拍 | 阈 ≤1ms ✓（计划 100µs ✗） |
| `tla.ewd998_tick1k_ns` | 93,868 ns/拍 | 计划 100µs ✓ |

**改定依据**：gol 16 格 × 每格一实体、klotski 10 件——每拍下限 ≈30 次 `pkg.entity` 主机往返（attrs 批读 + 属性写），单次 ≈7µs（mock KV + schema 校验 + alive 门），故 100µs / 10ms-1k 在「实体每格一存」映射下不可达（10³ 拍 ≈ 0.32s）。已做优化：每代一次 attrs 批读、reset 期 ids 缓存、不变量面恒新鲜扫描。阈值改定 ≤1ms/拍（≈3× 余量，防机器抖动红门）。

## 8. 跨游戏用点（≥2）

1. **任何模态移植的黄金回放参照法**：轨道 JSON（动作向量 + 冻结哈希 + 终态快照）+ 双局逐锚对账 + 篡改红三型——把外部形式规范（TLA / Alloy / 有限状态机）落成 mod 时直接复用 `src/replay.lua` 口径。
2. **红例双例样板**：每模型「不变量绿轨 + 非法态红 + 守卫红 + 对照绿」的用例结构，以及「KAT 语义表与冻结哈希分离断言」的两线法（shapez 的 KAT 是单线法的另一形态）。
3. **CHOOSE 唯一化的 fail-closed 先例**：`YGG-MOD-TLA-CHOOSE` 对「集合选择遇非唯一」报错不静默。

## 9. 验收与门禁

| 验收 | 判据 |
| --- | --- |
| 文件行数 ≤500 | 最大 380（`src/ewd998.lua`） |
| 包面消费 + 零 crates | 消费 entity / procgen / codec；`git diff --stat develop` 零 `crates/**` |
| 用例 | test-0 6 + a 12 + b 8 + c 12 + d 12 + z 8 = 58（含红例三型） |

`cargo run -q --example selftest -- --mod packages/mods/tla-models --packages packages/libs` → 本 mod 58 + 四包 73 = **131/0**，不回退 pkg 199 / agent 79 / shapez 155 / superagent 183 / egg 158 / `--lib` 114；manifest 双绿（`pack` 复算 diff-clean + `manifest-verify.sh` + `sync-seed-mod.sh --check`）；零 `crates/**` 改动。

## 10. 设计依据

**三要素**：① 数据形状 = 3 实体 schema + `tla.*` KV 键形 + 轨道 JSON 形；不变量 = TypeOK 三族 + Safra Inv + KlotskiGoal + TerminationDetection + 双局逐锚重放相等。② API 面 = mod 内 6 模块，消费 `pkg.entity` / `pkg.procgen` / `pkg.codec`，**新宿主面 = 零**。③ 性能预算 = §7 实测改定（PERF print 行原文可复现）。

**反方案否决**：① 哈希掺链（`h_t = hash(h_{t-1} ‖ snap)`）——stutter 步哈希必变，`[][Next]_v` 的 `v'=v` 分支失去「同态同哈希」对账面，弃；纯状态指纹 + 锚序列承担「链」语义。② 外部写不缓存失效面（gol shadow grid 写透缓存）——不变量面必须看见外部锻造（红例可见性），缓存只护 step/snapshot 热路径、TypeOK 恒新鲜扫描。③ 全量状态枚举对账（复刻 TLC）——枚举即验收膨胀。

维护责任 = 改 `packages/mods/tla-models/**` 的执行者；触发 = 任一相关 commit；检查 = §9 门禁。
