# Yggdrasill · sim-kernel — 原生模拟内核（三层引擎 Tier 1）

> **id** kernel/sim-kernel · **模块** ① ygg 内核（`crates/ygg-sim`） · **触发** 改模拟内核（tick / ECS 状态 / 快照 / 回放 / 确定性策略 / 热核面）之前
> **读不到代价** hard-refusal（确定性破坏 = 回放字节不一致 = agent 侧可复算性全失，属不可逆缺陷）

> **上游**：[../README.md](../README.md) · **相关**：[../packages/README.md](../packages/README.md)（Tier 2 壳纪律）· [state-machine.md](state-machine.md) · [lua-api.md](lua-api.md)（宿主面注册范式，同 fs 桥）

## 0. 终局形态：三层引擎（本文件是 Tier 1 的契约）

| 层 | 载体 | 职责 | 约束 |
| --- | --- | --- | --- |
| **Tier 1 原生模拟内核** | `crates/ygg-sim`（本文件） | tick 循环、ECS 列式状态、快照/回放/state hash、noise/DSP/几何/物理原语 | 无模拟天花板；确定性是结构不变量 |
| **Tier 2 沙箱规则层** | `packages/libs/**`（壳）+ mod（Lua） | 游戏规则、组合、剧情胶水 | **永久沙箱**：帽/燃料/无 IO/无时钟 |
| **Tier 3 内容与数据** | schema、数据文件、生成产物 | 不变的内容 | 纯数据 |

**判定三问**（逐能力裁决归属）：逐实体逐帧的热逻辑 → Tier 1；事件驱动的规则与组合 → Tier 2；不变的内容 → Tier 3。

**下沉铁律**（每个 Tier 2→Tier 1 下沉面必须全过）：① 契约先行（本文件或包契约先行改定再实现）；② **批式 API**——bytes/数组进出、单帧单调用，**禁止细粒度逐项 getter**（mlua 边界穿越 ~µs，细粒度调用是负收益）；③ before/after 表（同数据集/同机/三次取中位，原则二）；④ 七包 API 兼容——`pkg-selftest` 全量不回退。

**五不变量**：mod 永在笼内；数据契约先行；确定性是结构不是纪律；能力出口走证据触发（AGENTS.md §1.7）；agent 内省面（快照/回放/state hash/查询）是一等公民。

**设计依据（为什么直接立三层，不再等演进数据）**：模拟上限——Lua 解释域实时上限 ~10² 活跃实体@60fps，距 AAA 级模拟（10⁴⁺/帧）差 2 个数量级，语言不可弥补，层级可；确定性回放是 agent 游戏的核心卖点，只能做成结构不能靠纪律；参照 Factorio（原生引擎+Lua mod+确定性联机）同款分工已被大规模验证。反方案：全 Lua 包=上限不足（保留为 Tier 2）；七包重写为 Rust=杀掉笼内免编译生态（拒绝）。

## 1. 边界（这个内核不做什么）

- 不做渲染/表现——viewer 投影域，永远在内核之外。
- 不内嵌任何游戏语义（生命/伤害/捕捉率/难度）——那是 Tier 2。
- 不自带 IO 策略——落盘经宿主 fs 桥/产物面（同 fs 契约）。
- v1 不预造物理/寻路引擎——随 P3-4/P4 用例需求立项（防投机性建造）。

## 2. 数据形状

| 对象 | 形状 | 说明 |
| --- | --- | --- |
| `SimState` | `{ tick: u64, world: ArchetypeStore, resources: KV<u64> }` | 模拟态全量；**模拟态禁浮点**（定点政策见 I2） |
| 组件 | 列式 SoA，按 archetype 分桶；组件 schema 与 Tier 2 属性袋同源形状 | 数据形状单源：包契约即 schema 来源 |
| `Snapshot` | 确定性编码全量字节（复用 codec 编码序） | restore 后 state hash 必须与原运行一致 |
| `ReplayLog` | `{ seed, inputs: [(tick, input)] }` | 输入日志；回放 = restore + 逐 tick 重放 |

## 3. 不变量

- **I1 确定性**：同 seed + 同 ReplayLog → 逐 tick 状态字节一致（跨 tick 数、跨进程）。
- **I2 定点政策**：模拟数值默认 i64 定点（Q32.32 或按面声明）；浮点禁入模拟态；`x86_64`/`aarch64` 语义由整数运算保证（跨平台矩阵见 §5）。
- **I3 回放闭环**：`restore(snapshot) + replay(log)` → 每 tick `state hash` 与原始运行逐值一致。
- **I4 容量线**：10⁴ 活跃实体单 tick ≤ 16.6ms（目标线；实测表进 §5，未达即回炉）。
- **I5 隔离**：模拟态不读时钟/外部随机源——随机数仅由 seed 流派生（与 procgen rng 契约同源单份）。
- **I6 兼容**：任何下沉不得改变 Tier 2 包的对外 API——`pkg-selftest` 全量用例是回归钉。

## 4. API 面（批式铁律的落地形状）

| 面 | 签名形状 | 副作用 |
| --- | --- | --- |
| `sim.step(dt)` | 推进一 tick（系统注册制，系统列表是数据） | 变更模拟态 |
| `sim.snapshot()` → bytes / `sim.restore(bytes)` | 全量存取 | 纯 |
| `sim.hash()` → u64 | 规范化序列化序的哈希 | 纯 |
| bulk entity ops | `spawn_batch/despawn_batch/insert_batch`（批量字节进出） | 变更模拟态 |
| noise/DSP/几何原语面 | `eval(buffer_in) -> buffer_out` 批式 | 纯 |

宿主注册范式与 fs 桥同款：库侧 `driver.rs` 注入（可改），CLI 激活触点在 `egg.rs`（宿主面——**改动走用户授权账，与 U1 六行同笔**）。

## 5. 性能预算与验收

| 指标 | 阈值 | 测法 |
| --- | --- | --- |
| tick 10⁴ 活跃实体 | ≤ 16.6ms | 三次取中位，基准集入册 |
| snapshot/restore 10⁵ 实体 | ≤ 100ms | 同上 |
| 回放一致 | 逐 tick hash 一致 | replay 用例（字节对比） |
| Tier 2 兼容 | `pkg-selftest` 153/0 不回退 | 每次下沉后全量 |
| 跨平台 | x86_64 ↔ aarch64 digest 一致 | 无 CI 期**显式声明为论证级**（整数语义由语言规范保证）；有 CI 即补实测 |

## 6. 用点计划

| 复现项目 | 用点 | 该内核提供什么 | 阶段 |
| --- | --- | --- | --- |
| Saltline（P3-4） | 实时帧内碰撞/投影/弹道 | 批式 step + 几何原语面 | P3-4 帧率不达标即启用 |
| superagent game（P4） | 人物/道具/任务状态机 | bulk 状态面 + 快照/回放 | P4 |
| CDDA/DCSS（P3） | 大实体量回合制 | bulk 面性能余量 | P3（Lua 壳先行） |

## 7. 自测与验收

- replay 字节一致用例（restore + replay 逐 tick hash）；
- 定点政策静态检查（模拟 crate 禁浮点字面量，同 assetgen 扫描范式）；
- 七包 API 兼容回归（`pkg-selftest` 全量）；
- 字面三件套全绿；
- 每个下沉面一表：before/after（同数据集/同机/三次中位）。

## 8. 设计依据

- **为什么三层**：模拟上限差 2 个数量级只能靠层级补；确定性回放是 agent 游戏核心卖点，必须结构化；Factorio 先例（原生引擎+Lua mod+确定性）已验证该分工。
- **为什么 v1 收窄**：tick/ECS 存储/快照/hash 是所有后续面的公共底座，先立底座；物理/寻路等重系统随用例立项——防投机性引擎建造。
- **反方案**：全 Lua 包（上限不足，保留为 Tier 2 角色）；七包重写 Rust（杀掉笼内免编译生态，拒绝）；等 P3 数据再定（被用户裁决否决——架构直接承诺，数字用于排序下沉面而非决定是否下沉）。
- **①②③ 判据**（每个下沉面）：① 被阻动作=该面性能预算不达或需宿主资源；② 症状=对比表数字；③ 确认=digest/帧率/吞吐实测。
