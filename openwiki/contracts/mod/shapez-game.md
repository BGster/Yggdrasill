---
type: "Reference"
title: "shapez v1 核心循环（示例 Mod）"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-2ab16b0c37abba22ac8f5f64
    resource: repo://packages/mods/shapez/mod.lua
  - id: openwiki-source-e308a16824bc2fd47c443906
    resource: repo://packages/mods/shapez/pack.json
  - id: openwiki-source-3d2124bd428d41c3dac93c91
    resource: repo://packages/mods/shapez/src/belts.lua
  - id: openwiki-source-834de638f599bd7a30b4ed82
    resource: repo://packages/mods/shapez/src/bot.lua
  - id: openwiki-source-70149f5d637a268c78fea75d
    resource: repo://packages/mods/shapez/src/hub.lua
  - id: openwiki-source-418b259509afa6bfb10af753
    resource: repo://packages/mods/shapez/src/mapgen.lua
  - id: openwiki-source-09207ed69ef1a89dfd2425e8
    resource: repo://packages/mods/shapez/src/savegame.lua
  - id: openwiki-source-565d7085147379336fa0190f
    resource: repo://packages/mods/shapez/src/shapes.lua
  - id: openwiki-source-8872d27f8cf98c59c4e70665
    resource: repo://packages/mods/shapez/src/simtick.lua
  - id: openwiki-source-4325af450f48dd9270d0a9ef
    resource: repo://packages/mods/shapez/tests/test-b-belts.lua
  - id: openwiki-source-392c5ad98a80f951a60f3126
    resource: repo://packages/mods/shapez/tests/test-g-stress.lua
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# shapez v1 核心循环（示例 Mod）

> **id** mod/shapez-game · **模块** ③ mod 载体 · **触发** 改 `packages/mods/shapez/**` 任一行之前
> **读不到代价** silent-drift（定点口径 / 链判定 / 计数口径漂移，重放断言失效）
> **replaces**：DESIGN §3.x shapez 节（搬运基线 `3a2536f`）
> **上游**：[mod-package.md](mod-package.md) · **相关**：契约 [entity](../packages/entity.md) · [codec](../packages/codec.md) · [procgen](../packages/procgen.md) · [quest](../packages/quest.md)

## 1. 它证明什么

shapez 是**游戏 Mod 的活证据**：一个 8 模块的多文件 Mod，只用 `pkg.*` 包面与 `ygg.*` 沙箱面，**零 `crates/ygg-*` 改动**，就把一部真实游戏的核心循环压进内核契约。它同时是「多文件纯度」（§[mod-package](mod-package.md) §4）与「确定性重放」两条门禁的现场样本。

**核心循环**：矿机产形 → 传送带定点推进 → 切割 / 堆叠 / 旋转 → hub 交付 → 过级。

| 事件 | 语义 |
| --- | --- |
| `event.start` | `simtick.setup`：schema 注册 + 定点常量 + hub 装配；载荷串 = 地图种子（首局写 `sz.seed`，first-wins，缺省也是钉死常量 `shapez-v2`，**禁缺省熵**） |
| `sz.place.*` | `x,y,rot` 落格 + `sz.layout_v` bump；矿机必须落在资源斑内（v2 = 分块 procgen，放置触发懒生成） |
| `sz.tick` | 一拍 = 处理器 / 矿机（出料→投产 / 推进）→ 传送带（路径推进 + 末端弹射）→ `sim.step(1)` + `sz.tick` +1；每 N 拍落锚 `sz.anchor.<tick> = procgen.hash(snapshot())` |
| 交付 | hub 收任意**非空**形状；只有命中当前级目标（规范形串相等）才计数；满额过级 = `quest.advance`（本级恰一次）+ 新级实例 + 计数归零，末级进 freeplay |

**降级诚实账**：裸 `KernelFacade`（selftest / playtest）未挂 sim——首拍叙述一行 `[SIM-UNMOUNTED]`，KV 计拍照走。

## 2. 数据形状（它练的是哪几条形状契约）

- **shortkey 即规范形**（源 `serialize() == getHash()` 同位语义）：层自下而上以 `:` 相接；每层恰 8 字符 = 4 象限 ×（子形 `R/C/S/W` + 颜色 `r/g/b/y/p/w/u`）；象限序 `0=tr 1=br 2=bl 3=tl`；空象限 `--`；全空层非法；空项 = 空串 `""`（仅切割空半的传输占位，`is_valid("") == false`）。
- **Q32.32 定点**：raw = 值 × 2³²，全程 64 位整数 `//`，全部由 `belts.constants(tuning)` 从 `tuning.*Milli` 纯整数推导——`spacing_q = 2705829396`、`belt_step_q = 143165576`、`miner_step_q = 28633115`（151 拍 / 件）、cutter `35791394`（121 拍 / 件）、stacker `17895697`。
- **4 个 entity schema**：`sz.belt`(x/y/rot) · `sz.processor`(kind/in1/in2/progress/out1/out2) · `sz.hub` · `sz.miner`(shape/progress/out1)。计划里的「6–8 schema」是 v1–v3 合计口径：static 吸收进各实体 attrs，ejector / acceptor 吸收为 `in*` / `out*` 槽。
- **KV 键面（`sz.*` 单写者）**：`sz.tick` / `sz.level` / `sz.got` / `sz.layout_v` / `sz.setup` / `sz.hub_id` / `sz.level_quest` / `sz.path.<startid>`（`codec.encode` 的 `{s=spacing_q, i={{d,k}…}}`）/ `sz.anchor.<tick>`（16 位 hex）；v2 增 `sz.seed` / `sz.mapgen_v`（**进档**）与 `sz.chunk.<cx>.<cy>`（缓存，**不进档**）。

## 3. 模块图（`mod.lua` 装配，`src/**` 纯导出）

| 文件 | 职责 |
| --- | --- |
| `src/shapes.lua` | shortkey 编解码 + 切割 / 堆叠 / 旋转纯函数 |
| `src/belts.lua` | 路径链重建 + 定点推进 + KV 持久化 |
| `src/processors.lua` | miner / cutter / stacker / rotater / trash |
| `src/hub.lua` | 交付计数 + 过级 + quest 实例 |
| `src/simtick.lua` | tick 信封 + 落点裁决 + 快照 / 锚 |
| `src/mapgen.lua`（v2） | 分块 procgen + `patch_at` + `sz.seed` / `sz.mapgen_v` 守护 |
| `src/savegame.lua`（v2） | 存档信封 save / load + 三重对账 |
| `src/bot.lua`（v3） | 长局压测台（决策流 + churn + 吞吐台） |

## 4. 验收映射（本页要 1 分钟能核对的东西）

| 验收 | 实现面 | 用例 |
| --- | --- | --- |
| 形状 KAT（黄金值自建） | shapes 纯函数 | test-a 10 例（往返 / 切割源序 / 旋转闭包 / 堆叠 gap / 层数上限 / 错误码） |
| 10⁴ tick 重放（I3） | simtick 锚 + `procgen.hash` | test-b：双局 20 锚逐值相等 + 终态快照逐字节 + KV 锚对账 |
| 最小玩法过到 level 2 | 两线十实体 | test-z：1→2 矿机线 tick 界 + 事件恰一次；2→3 切割线 2×10⁴ 拍内；同动作双局终态哈希相等 |
| v2 分块地图 | mapgen（纯 Lua over procgen） | test-e 11 例（整数距离 KAT / 种子守护 / 五斑钉值 / 黄金 chunk KAT / 同 seed 相等 + 跨 seed 否定 / `patch_at` 三态 / 负坐标 floor / 零 layout bump / 缓存哨兵 / 色斑先序 / PERF chunkgen） |
| v2 存档信封 | savegame（codec 往返） | test-f 8 例（往返逐字节 + roundtrip 双保险 / 脏 KV 守卫 / 派生态不进档 / A·B·C 三局闭环 / 幂等导入 `created=0` / 篡改一字节拒 / `mapgen_v` 不符拒 / PERF save·load） |
| v3 长局压测 | bot 台 + `sz.demolish` + 五指标 | test-g 6 例（双档：决策流 KAT 恰等 + 比例钉值 / 残留键墓碑恰等 / 同 seed 双局逐值 + 跨 seed 否定 / 周期存读档 B≡A 全锚 / 吞吐往返逐字节 + id 保真 / PERF 八行） |

**门禁命令**：

```bash
cargo run -q --example selftest -- --mod packages/mods/shapez --packages packages/libs
# 本 mod 52 用例 + 四包 103 用例；不得回退 pkg 199/0、agent 79/0、superagent 183/0、egg 158/0、--lib 112/0
# full 档（显式，不进默认门禁）：SHAPEZ_STRESS=full … --filter test-g
```

外加 manifest 双绿（shapez + agent-seed）、**零 `crates/**` 改动**、文档一致性 + 版本门禁。

## 5. 简化登记（逐条可断，selftest 钉住）

链判定放宽（后带不逆风即接，无曲线段）· 无压缩堆叠分支 · 布局重建不迁状态（按 startid 命中才保留路径态）· 单口串行出料（每拍每台至多一件，出料仅在空闲拍）· 矿机出料不受 progress 门（progress 是产量累加器）· 处理器开工即算产物 · 静态斑退役（v2 换 procgen 分块）· 无种子退役（v2 `sz.seed` 显式、`sz.mapgen_v` 钉版本）· 形斑单色元（混色留 v3）· 存档最小面（mb 级长档不在本轮，带内运输件不进档）· 拆带弃件（v3）· 闭环轨零销毁（`importing` 保 id = 致密重排）· 仪表盘只在 selftest 普通 Lua 面成立（bot 本体零探针，clock / gc 注入制）。

## 6. 设计依据（为什么这样落）

**数据形状**：shortkey 规范形串 + 4 entity schema + `sz.path.*` / `sz.anchor.*`；不变量 = 间距 ≥ 0.63 定点版、层序 canonical、重放逐锚 hash 相等、本级 quest completed 恰一次。**API 面**：mod 内模块 + 消费 `pkg.entity`（spawn / get / set / ids / define）、`pkg.codec`、`pkg.procgen`、`pkg.quest`——**新宿主面 = 零**。**性能预算**：v1 单拍 ≤ 100 µs @ selftest 口径。

**被否的反方案**：物品上带不作实体（10⁴ tick × 实体化 = 属性袋写入放大，KV `sz.path.*` 批量态更省且天然可序列化）；处理器不做双槽独立方向（v1 场景为直线产线，串行口吞吐 ≥ 处理器周期上限）。

**三条约束的直接后果**（迁移到本仓后仍成立）：① 逐拍流隔离序钟——决策流名 `bot.<世界拍>`，续跑局以 `sz.tick` 对齐流序，是 B≡A 的序钟基础；② 闭环轨零销毁——`importing` 的「保 id」= 致密重排，洞位 id 不在存读档契约内，故 `closed` 模式全程禁 demolish；③ 稳态小世界 + 高水位 KV 分离——tick 热路径受 `entity.ids` 水位扫描与 belts 路径序 O(P²) 支配，万级压力由 churn 段与吞吐台承担。
