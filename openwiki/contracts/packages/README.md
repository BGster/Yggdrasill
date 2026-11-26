---
type: "Reference"
title: "packages 能力契约（索引与命名规则）"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-e8a9c9a52e4a7131f226de1b
    resource: repo://.pi/tools/manifest-verify.sh
  - id: openwiki-source-1816b4326e35a15b7cd252ff
    resource: repo://crates/ygg-sandbox/examples/pack.rs
  - id: openwiki-source-f5131ab478fa718e924dbf48
    resource: repo://crates/ygg-sandbox/src/inject.rs
  - id: openwiki-source-fef267a23f756637e121d083
    resource: repo://crates/ygg-sandbox/src/sandbox.rs
  - id: openwiki-source-9e553a7d516edb7720e6365c
    resource: repo://packages/libs/assetgen/pkg.json
  - id: openwiki-source-a74302e276dd8f7c220d4ffe
    resource: repo://packages/libs/codec/pkg.json
  - id: openwiki-source-7e104178dc65c6625b2c2050
    resource: repo://packages/libs/entity/init.lua
  - id: openwiki-source-7e16e66d75bcd267f6045913
    resource: repo://packages/libs/entity/pkg.json
  - id: openwiki-source-a0892a98ca143451ee73aebb
    resource: repo://packages/libs/procgen/pkg.json
  - id: openwiki-source-ee42ff4c10e774f5d7d2cfc4
    resource: repo://packages/libs/role-skill/pkg.json
  - id: openwiki-source-c872e8c01e8c61dc1b56328f
    resource: repo://packages/libs/textframe/pkg.json
  - id: openwiki-source-23e2f860a4e0501c0d83957c
    resource: repo://packages/mods/pkg-selftest/mod.lua
  - id: openwiki-source-ef5fcdd287f9052b0178d215
    resource: repo://packages/mods/pkg-selftest/pack.json
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# packages 能力契约（索引与命名规则）

> **id** packages/README · **模块** ⑤ packages · 通用能力层 · **触发** 新增或修改任何 package 或其契约之前
> **读不到代价** hard-refusal（不带契约 / 自测 / 性能数字的能力提交当场应被拒，纪律见 [../../governance.md](../../governance.md) §6）
> **replaces**：DESIGN §4 / `docs/design/packages/README.md`（搬运基线 `3a2536f`）
> **上游**：[../../architecture/README.md](../../architecture/README.md)（契约索引）· **相关**：[../mod/mod-package.md](../mod/mod-package.md)（多文件纯度范式）· [../kernel/lua-api.md](../kernel/lua-api.md)（`ygg.*` 是包唯一可用的宿主面）· [../kernel/state-surface.md](../kernel/state-surface.md)（KV 契约）· **判据**：[../../governance.md](../../governance.md)

## 1. 定位与判据

**三层引擎中的位置（终局形态，契约见 [../kernel/sim-kernel.md](../kernel/sim-kernel.md)）**：本树是 **Tier 2 沙箱规则层的能力契约**——对外 API 形状永久稳定（mod 兼容面）；内部实现随实测逐面下沉到 Tier 1 原生模拟内核（`crates/ygg-sim`）成为兼容壳。下沉铁律：契约先行、批式 API、before/after 表、pkg-selftest 不回退——详见 sim-kernel.md §0。

这里是八份**能力契约**的入口。包是「换个游戏还成立」的那一层：id 分配、属性袋、序列化、任务流转、对话调度、角色与技能引用、程序化生成、素材字节、字符网格。进包之前先过三问：

| 判据 | 归宿 |
| --- | --- |
| 换个游戏还成立？（数据形状与不变量） | **package**（本树） |
| 这是「这个游戏」的定义？（语义与规则） | **mod**（见 [../mod/mod-package.md](../mod/mod-package.md)） |
| 改它要改代码吗？不该（是内容） | **数据文件**（`pack.json` 的 `entries`，经 `ygg.data` 读） |

三问之外的硬闸：**每个包 ≥2 个不同游戏的用点**，用点取自四个复现项目（PokeRogue / CDDA / DCSS / Saltline）。只有一个用点的能力属于该游戏的业务，不是通用能力——这是本树拒绝新包的第一条理由。包**不写游戏规则**：生命归零算死、委托要不要审批、难度怎么算，一律留 mod。能力出口（内核能力 vs 包 vs 本游戏业务）的宪法条款在 `AGENTS.md` §1.7。

## 2. 包清单与状态表（2026-11-26 实测）

| 包 | 契约 | 依赖 | 用例（实测） | 用点覆盖 |
| --- | --- | --- | --- | --- |
| entity | [entity.md](entity.md) | 无（薄包 `ygg.entity.*`） | 29 | 4（PokeRogue / CDDA / DCSS / Saltline） |
| codec | [codec.md](codec.md) | entity | 21 | 4（CDDA / PokeRogue / DCSS / Saltline） |
| quest | [quest.md](quest.md) | entity · codec | 24 | 4（PokeRogue / CDDA / DCSS / Saltline） |
| dialogue | [dialogue.md](dialogue.md) | quest | 26 | 3（PokeRogue / CDDA / Saltline） |
| role-skill | [role-skill.md](role-skill.md) | entity · codec | 28 | 4（PokeRogue / DCSS / CDDA / Saltline） |
| procgen | [procgen.md](procgen.md) | codec（仅摘要面） | 25 | 4（CDDA / DCSS / Saltline / PokeRogue） |
| assetgen | [assetgen.md](assetgen.md) | procgen（**仅 rng 面**） | 28 | 3（Saltline / CDDA·导出面 / PokeRogue；DCSS 字符渲染不消费像素未兑现用点，P3-3 G-6 裁定 2026-11-21） |
| textframe | [textframe.md](textframe.md) | 无（纯导出，零依赖） | 18 | 3（Saltline 实证替换 / CDDA 指认 / DCSS 指认；PokeRogue 图形化无字符网格，见契约 §6） |

合计 **199**（八行相加自洽——任一侧漂移就对不上）。复算命令：`--filter pkg-<name>` 逐包一次，全量一次，见 §5。

## 3. 依赖序

`entity → codec → quest → dialogue → role-skill`，`procgen → codec`（仅摘要面），`assetgen → procgen`（仅 rng 面）。每条边都是**数据形状**依赖（不是编译依赖）：

- `codec` 依赖 `entity`：bundle 里的记录形状 = entity §2 的属性袋形状；序列化不能自己发明第二份实体表示。
- `quest` 依赖 `entity` + `codec`：委托方与承接方是实体 ref；任务实例随 KV 前缀由 codec 点名导出。
- `dialogue` 依赖 `quest`：任务推进靠 `quest.*` 事件驱动对白，对话包消费事件，不反向调用任务状态机。
- `role-skill` 依赖 `entity` + `codec`：角色/技能只存引用与计数，属性袋归 entity，存档归 codec。
- `procgen` 依赖 `codec`（**仅摘要面**）：`procgen.hash` 借 codec I1 的确定性编码口径，不自己写第二份编码器；PRNG 本身只依赖确定性种子。它是唯一「产物不落 KV、defs 也不落 KV」的包——`defs` 由安装期注入 + 运行期注册表。
- `assetgen` 依赖 `procgen`（**仅 rng 面**）：只用 `procgen.rng` / `procgen.state` / `procgen.next32`，不调 `define` / `run` / `hash`。共享的是 rng 契约，不是生成算法：procgen 产 `cells`，assetgen 产 `pixels` / `pcm` / 顶点表。
- `textframe` **无依赖**：字符网格只碰整数下标与字符串拼接。它是依赖图上唯一的孤点（零入边零出边），有意的——[textframe.md](textframe.md) §3 I8。

## 4. 契约章节模板（八节同构，缺节即未写完）

```text
# 标题 — 一句定位
> **id** packages/<name> · **模块** ⑤ packages · 通用能力层 · **触发** <改动这个包之前>
> **读不到代价** <irreversible | hard-refusal | silent-drift | none>（<一句为什么>）

## 1. 边界（这个包不做什么）
## 2. 数据形状   （表：字段 | 类型 | 必填 | 缺省 | 持久化位置）
## 3. 不变量     （I1..In，每条都是可断言的单句）
## 4. API 面     （表：函数 | 签名 | 返回 | 副作用类别）
## 5. 性能预算   （五个指标 + 本包取值 + 测量方法 + 基准集）
## 6. 用点计划   （表：复现项目 | 用点 | 该包提供什么 | 阶段）
## 7. 自测与验收 （用例名前缀、断言面、进哪条命令的计数）
## 8. 设计依据   （为什么这样切 / 反方案 / ①②③ 判据）
```

文档头只写 `id` 与 `读不到代价`（加 `模块` / `触发`）——**不写 ver 头**：版本只在 `openwiki/ver.json`，见 [../../governance.md](../../governance.md) §7。

## 5. 载体与纪律

- **布局**：`packages/libs/<name>/pkg.json` + `init.lua` + 同目录子模块 + `test/`。`<name>` 必须匹配 `^[a-z][a-z0-9_-]*$`，它就是包 id（单源，不额外声明）。模块名 = `pkg.<name>`——mod 子模块已占用 `src.*`。**连字符规则**：id 里的 `-` 在模块名里折叠为 `_`（`role-skill` → `pkg.role_skill`），因为模块名段字符集只有 `[A-Za-z0-9_]`；文件名段同样不许连字符（`attr-v2.lua` 打包期即拒）。
- **`pkg.json`（必须存在）**：`{ schemaVersion: 1, name, version, deps?, contract }`。`name` 等于目录名；`version` 是**精确 pin**（mod 的 `pack.json` `packages[]` 与本包 `deps[]` 引用都是字符串相等）；`contract` 是仓库内相对路径且必须存在。它进 manifest 的 `packages` 段（内容寻址），但**不进**模块表（不是 Lua）。
- **纯度**：包模块顶层只允许纯导出（函数与常量），注册 / 写状态 / 输出一律在 `M.install(deps)` 或调用期——与 [../mod/mod-package.md](../mod/mod-package.md) 的 `mod.pure-export` 同一条纪律。包内除 `test/**` 外每个文件都必须被 manifest 的 `packages` 段纳管。
- **自测入册**：用例名前缀 `pkg.<name>.`，放 `<包根>/<本包>/test/*.lua`，由 `selftest` 收集，与 mod 自测进同一个 `cases passed=` 计数面。主件可把用例段按主题拆进兄弟件 `test/<主件名>/<theme>.lua`（`return function(C) … end` 注入，主件尾部装配）；采集面只看 `test/*.lua` 主件（非递归），计数守恒。
- **发现机制**：声明驱动，不扫目录——只有 `pack.json` 的 `packages` 段声明过的包才装载，包根由 `--packages` / `$YGG_PACKAGES_ROOT` / `<mod>/../../packages/libs` 解析（完整规则见 [../mod/mod-package.md](../mod/mod-package.md) §3.2.3）。
- **两个夹具**：`packages/libs/fixture/` 是**装载面活体夹具**（钉多文件 require、缓存身份、chunk 名、纯导出边界），显式豁免「≥2 用点」硬闸；`packages/mods/pkg-selftest/` 是**声明了 `packages` 段的 Mod**，唯一职责是让包自测能被收集执行（生产 mod `agent-seed` 不声明包，它的 `cases passed=` 永远看不到 `pkg.*`，这就是回归钉）。
- **计数口径（2026-11-26 实测，命令见下）**：全量 **199**；逐包 `--filter pkg-entity` → 29、`pkg-codec` → 21、`pkg-quest` → 24、`pkg-dialogue` → 26、`pkg-role-skill` → 28、`pkg-procgen` → 25、`pkg-assetgen` → 28、`pkg-textframe` → 18（相加 = 199）。沿革：S6 记的 191（entity 21）已随 S9 之后的用例增长到 199/29。`--filter` 过滤的是**文件名**，故每个包在 mod 侧 `tests/` 下都要有同名冒烟文件接住，否则单独跑会「没有可执行的测试文件」exit 1。
- **冒烟不计数**：`tests/pkg-*-smoke.lua` 是 file 级断言（`error` ⇒ `file FAIL` ⇒ `ok=0`），只钉「装载面 + 顶层纯度 + 导出面清单（+ 事件名集合、KV 前缀、摘要 / canon 行格式)」，不进 `cases passed=`。
- **`entries` 必须非空**：打包工具要求资源包至少一个数据条目，所以夹具带一条探针条目（`data/probe.json`），随后 `cargo run -p ygg-sandbox --example pack -- packages/mods/pkg-selftest --packages packages/libs` 重算 manifest，`sh .pi/tools/manifest-verify.sh --mod packages/mods/pkg-selftest` 复核 `pkg_drift=0`。
- **沙箱可装载面（包代码的硬边界）**：包在裸 `Sandbox::new()` 里只拿得到**第一级**面：

  | 级 | 面 | 出处 | 谁可以用 |
  | --- | --- | --- | --- |
  | 第一级 | 16 个 base 纯计算函数（`assert` / `error` / `getmetatable` / `ipairs` / `next` / `pairs` / `pcall` / `rawequal` / `rawget` / `rawlen` / `select` / `setmetatable` / `tonumber` / `tostring` / `type` / `xpcall`）+ 字符串方法（经 `getmetatable("").__index`）+ 受控 `require` + `ygg` | `crates/ygg-sandbox/src/sandbox.rs` 的 `BASE_WHITELIST` | **包与 mod 都可以** |
  | 第二级 | 15 个 `math` 成员（`abs` / `floor` / `ceil` / `sqrt` / `fmod` / `modf` / `tointeger` / `ult` / `min` / `max` / `random` / `huge` / `maxinteger` / `mininteger` / `type`）+ `ygg.ticks` | `crates/ygg-sandbox/src/inject.rs` 的 `install`（**只在 driver 路径**注入） | **只有 mod**；包不得依赖 |

  `string` / `table` / `utf8` / `coroutine` / `math` / `os` / `io` / `package` 与 `dofile` / `loadfile` / `load` / `print` / `rawset` / `collectgarbage` 全为 `nil`（`whitelisted_environment_exposes_only_the_allow_list` 逐名断言）。包侧越级的症状：按旧文献写 `table.insert` / `string.format`，或以为「包可以用 `math`」——裸沙箱装载期 `attempt to index a nil value (global 'math')`。见 [../kernel/lua-api.md](../kernel/lua-api.md) §3.1。

  两条门禁（改任何包代码后必跑）：

  ```sh
  cargo test -p ygg-sandbox --test packages_sandbox                # 裸沙箱装载 / 源码禁名列
  cargo run -q --example selftest -p ygg-sandbox -- \
    --mod packages/mods/pkg-selftest --packages packages/libs      # 生产面（Lua 计数）
  ```

- **性能数字**：每包 §5 五个指标必须给数字口径与测量方法（同数据集 / 同机器 / 跑三次取中位）；首实现填实测值，偏差回写契约。
- **不新增存储层**：实体持久化只有一条路——mod KV 命名空间 + codec。每个包的 KV 前缀在它自己的 §2「持久化位置」列声明，前缀单写者（对齐 `mod-package.md` 的 `mod.kv-prefix-owner`）。
- **载体判据**：新能力落 `crates/ygg-*/` 还是 `packages/libs/<name>/`，按 `AGENTS.md` §1.7 判——本文件是能力的规格，不是工作流规则的第二副本。

## 6. 设计依据

**为什么「包」是这么一个单位。** 判据是数据形状的稳定性，不是代码量。实体 id、属性袋、任务步骤、对话事件、技能引用、种子序列——这些形状在换游戏时**不变**，变的只是填进去的名字与数值；伤害公式、委托审批、难度曲线在换游戏时**必然**变，写进包就是把第一个游戏的规则变成所有游戏的限制。

**反方案一：继承树。** 生物 / 物品 / 技能 / 任务各建一棵类树，一旦出现「既是物品又能施放」或「任务本身是一件可交易物」，多重继承就来了。**反方案二：每个游戏自带一套实体/任务/对话系统。** 代价可数：四份 `next_id`、四份序列化、四份任务状态机、四份 agent 对话管道（含四份凭据处理）。Goal 原则给的是**组合优先于派生**：一切都是实体 + 属性袋，类型用 schema 表达成数据，新增类型 = 加数据不改包。

**判据（①②③）。** ① 阻断——一个能力提交没选载体：该进内核（需要 Rust 语义）的写成了包，或是本游戏业务却塞进了 `packages/libs`；包内写进了游戏规则。② 症状——同一数据形状在每个游戏里各造一遍；一个包只有一个游戏在用；某个游戏在自己的 mod 里重写实体管理 / 序列化 / 任务流转 / 对话调度。③ 确认——契约存在且被 [../../architecture/README.md](../../architecture/README.md) 索引，用例计入 `selftest` 的 `cases passed=`，§5 性能表有数字。
