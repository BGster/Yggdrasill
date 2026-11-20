# Yggdrasill · packages 能力契约（索引与命名规则）

> **id** packages/README · **模块** ⑤ packages · 通用能力层 · **触发** 新增或修改任何 package 或其契约之前
> **读不到代价** hard-refusal（不带契约 / 自测 / 性能数字的能力提交当场应被拒，纪律见 `AGENTS.md` §1.7）

> **上游**：[../README.md](../README.md)（design 树入口）· **相关**：[../mod/mod-package.md](../mod/mod-package.md)（多文件纯度范式）· [../kernel/lua-api.md](../kernel/lua-api.md)（`ygg.*` 是包唯一可用的宿主面）· [../kernel/state-surface.md](../kernel/state-surface.md)（KV 契约）· **判据**：[../../wiki-governance.md](../../wiki-governance.md)

## 1. 定位与判据

这里是六份**能力契约**的入口。包是「换个游戏还成立」的那一层：id 分配、属性袋、序列化、任务流转、对话调度、角色与技能引用、程序化生成。进包之前先过三问：

| 判据 | 归宿 |
| --- | --- |
| 换个游戏还成立？（数据形状与不变量） | **package**（本树） |
| 这是「这个游戏」的定义？（语义与规则） | **mod**（见 [../mod/mod-package.md](../mod/mod-package.md)） |
| 改它要改代码吗？不该（是内容） | **数据文件**（`pack.json` 的 `entries`，经 `ygg.data` 读） |

三问之外的硬闸：**每个包 ≥2 个不同游戏的用点**，用点取自四个复现项目（PokeRogue / CDDA / DCSS / Saltline）。只有一个用点的能力属于该游戏的业务，不是通用能力——这是本树拒绝新包的第一条理由。包**不写游戏规则**：生命归零算死、委托要不要审批、难度怎么算，一律留 mod。能力出口（内核能力 vs 包 vs 本游戏业务）的宪法条款在 `AGENTS.md` §1.7。

## 2. 包清单与状态表

| 包 | 契约 | 依赖 | 状态 | 用点覆盖 |
| --- | --- | --- | --- | --- |
| entity | [entity.md](entity.md) | 无（薄包 `ygg.entity.*`） | 契约已定 · 代码 P2 待实现 | 3（PokeRogue / CDDA / DCSS） |
| codec | [codec.md](codec.md) | entity | 契约已定 · 代码 P2 待实现 | 3（CDDA / PokeRogue / DCSS） |
| quest | [quest.md](quest.md) | entity · codec | 契约已定 · 代码 P2 待实现 | 3（PokeRogue / CDDA / DCSS） |
| dialogue | [dialogue.md](dialogue.md) | quest | 契约已定 · 代码 P2 待实现 | 3（PokeRogue / CDDA / Saltline） |
| role-skill | [role-skill.md](role-skill.md) | entity · codec | 契约已定 · 代码 P2 待实现 | 3（PokeRogue / DCSS / CDDA） |
| procgen | [procgen.md](procgen.md) | 无（只依赖确定性种子） | 契约已定 · 代码 P2 待实现 | 3（CDDA / DCSS / Saltline） |
| assetgen（规划项） | `packages/assetgen.md`（P2.5 立项时创建，届时本行改为链接） | procgen（共享 rng 契约） | 规划 | CDDA / DCSS / Saltline |

最后一行是**规划项，不是链接**：P2.5 的硬要求「全部由 seed 驱动、同 seed 同产物字节一致」使它和 [procgen.md](procgen.md) 共享同一份 rng 契约，先建 `assetgen.md` 会迫使 rng 契约写两遍。P0 只登记，立项时创建文件并把本行改成链接。

## 3. 依赖序

`entity → codec → quest → dialogue → role-skill`，`procgen` 独立、可提前。每条边都是**数据形状**依赖（不是编译依赖）：

- `codec` 依赖 `entity`：bundle 里的记录形状 = entity §2 的属性袋形状；序列化不能自己发明第二份实体表示。
- `quest` 依赖 `entity` + `codec`：委托方与承接方是实体 ref；任务实例随 KV 前缀由 codec 点名导出。
- `dialogue` 依赖 `quest`：任务推进靠 `quest.*` 事件驱动对白，对话包消费事件，不反向调用任务状态机。
- `role-skill` 依赖 `entity` + `codec`：角色/技能只存引用与计数，属性袋归 entity，存档归 codec。
- `procgen` 只依赖确定性种子：不依赖其它包，可最先落地；它是唯一一个「产物不落 KV」的包。

## 4. 契约章节模板

六份契约**逐节同构**，缺节即视为未写完：

```text
# Yggdrasill · packages/<name> — <一句定位>
> **id** packages/<name> · **模块** ⑤ packages · 通用能力层 · **触发** <改动这个包之前>
> **读不到代价** <irreversible | hard-refusal | silent-drift | none>（<一句为什么>）

## 1. 边界（这个包不做什么）
## 2. 数据形状        （表：字段 | 类型 | 必填 | 缺省 | 持久化位置）
## 3. 不变量          （I1..In，每条都是可断言的单句）
## 4. API 面          （表：函数 | 签名 | 返回 | 副作用类别）
## 5. 性能预算        （五个指标 + 本包取值 + 测量方法 + 基准集）
## 6. 用点计划        （表：复现项目 | 用点 | 该包提供什么 | 阶段）
## 7. 自测与验收      （用例名前缀、断言面、进哪条命令的计数）
## 8. 设计依据        （为什么这样切 / 反方案 / ①②③ 判据）
```

文档头**只写** `id` 与 `读不到代价` 两个固定头，**不写 ver 头**：`scripts/docs-ver.sh` 只扫 `docs/context/`（`files=22` 与 `find docs/context -name '*.md'` 逐一吻合），design 树的 `ver` 没有刷新机制，写了就是一个永不更新的字段。

## 5. 载体与纪律

- **布局**（P1 已落地）：`packages/libs/<name>/pkg.json` + `init.lua` + 同目录子模块 + `test/`。`<name>` 必须匹配 `^[a-z][a-z0-9_-]*$`，它就是包 id（单源，不额外声明）。模块名 = `pkg.<name>`——mod 子模块已占用 `src.*`（`.ygg/mods/agent-seed/src/**` 以 `require("src.core.util")` 形式互相引用）。**连字符规则**：id 里的 `-` 在模块名里折叠为 `_`（`role-skill` → `pkg.role_skill`）——模块名段字符集只有 `[A-Za-z0-9_]`，`pkg.role-skill` 会被 `normalize` 拒掉，折叠是唯一可行映射。同理，**文件名段也不许有连字符**：`attr-v2.lua` 是不可装载的模块路径，打包期即可读拒绝。
- **`pkg.json`（包自描述，必须存在）**：`{ schemaVersion: 1, name, version, deps?, contract }`。`name` 必须等于目录名；`version` 是**精确 pin**（被 mod 的 `pack.json` `packages[]` 与本包 `deps[]` 引用时都是字符串相等）；`deps` 可缺省（`[{id, version}]`）；`contract` 是仓库内相对路径，必须存在（打包期校验）。它与包内 Lua 文件一样进 manifest 的 `packages` 段（它决定解析结果，必须内容寻址），但**不进**模块表（不是 Lua）。
- **纯度**：包模块顶层只允许纯导出（函数与常量），注册 / 写状态 / 输出一律在 `M.install(deps)` 或调用期——与 [../mod/mod-package.md](../mod/mod-package.md) 的 `mod.pure-export` 同一条纪律。**无未声明文件**：包内除 `test/**` 外的每个文件都必须被宿主 manifest 的 `packages` 段纳管（已落地；其它文件打包期即拒）。
- **自测入册**（已落地，条件式）：用例名前缀 `pkg.<name>.`，放在 `<包根>/<本包>/test/*.lua`，由 `cargo run -q --example selftest -- --mod <mod> --packages <包根>` 收集，与 mod 自测进入**同一个** `__SELFTEST` 计数面（`cases passed=` 里一并计）。于是条件式成立——声明的包计入该命令的计数，不声明包的 mod 跑 0 条，生产 mod（`agent-seed`）的 `cases passed=` 不因本机制变化。包自测自带计数垫片，不依赖消费它的 mod 的 harness。
- **发现机制（P1 已定）**：声明驱动，不靠目录扫描——只有 `pack.json` 的 `packages` 段声明过的包才会被装载，包根由 `--packages` / `$YGG_PACKAGES_ROOT` / `<mod>/../../packages/libs` 解析。完整规则见 [../mod/mod-package.md](../mod/mod-package.md) §3.2.3。
- **装载面夹具（`fixture`）**：`packages/libs/fixture/` 是**装载面活体夹具**（`pkg.json` + `init.lua` + `words.lua` + `test/pkg-fixture.lua`），**不是能力包**，显式豁免第 ② 节「≥2 个不同游戏用点」硬闸——那道闸针对能力包；夹具的作用是钉住装载契约（多文件 require、缓存身份、chunk 名、纯导出边界），不是提供能力。P2 的六个能力包（entity / codec / quest / dialogue / role-skill / procgen）不受此豁免影响。
- **性能数字**：每包 §5 五个指标必须给数字口径与测量方法（同数据集 / 同机器 / 跑三次取中位）；P0 期填**目标值**，P2 首实现填实测值，偏差回写契约。
- **不新增存储层**：实体持久化只有一条路——mod KV 命名空间 + codec。每个包的 KV 前缀在它自己的 §2「持久化位置」列声明，前缀单写者（对齐 `mod-package.md` 的 `mod.kv-prefix-owner`）。
- **前向契约的清偿义务**：P0 期这些契约**没有代码消费者**，按 [../../wiki-governance.md](../../wiki-governance.md) 的判据它们是前向契约（Goal 反例 7 又硬性要求路径就是 `docs/design/packages/<name>.md`）。清偿动作写死在 P2：每包首实现时 `packages/libs/<name>/init.lua` 的头注释写 `docs/design/packages/<name>.md`，指针类语义自那一刻成立。**能力出口**：新能力落 `crates/ygg-*/` 还是 `packages/libs/<name>/`，按 `AGENTS.md` §1.7 判——本文件是能力的规格，不是工作流规则的第二副本。

## 6. 设计依据

**为什么「包」是这么一个单位。** 判据是数据形状的稳定性，不是代码量。实体 id、属性袋、任务步骤、对话事件、技能引用、种子序列——这些形状在换游戏时**不变**，变的只是填进去的名字与数值；所以它们能被一次写对、反复使用。反过来，伤害公式、委托审批、难度曲线在换游戏时**必然**变，写进包就是把第一个游戏的规则变成所有游戏的限制。

**反方案一：继承树。** 生物 / 物品 / 技能 / 任务各建一棵类树，看着整齐，但一旦出现「既是物品又能施放」或「任务本身是一件可交易物」，多重继承就来了。**反方案二：每个游戏自带一套实体/任务/对话系统。** 这是现状，代价可数：四份 `next_id`、四份序列化、四份任务状态机、四份 agent 对话管道（含四份凭据处理）。Goal 反例 2 判定这种「复现里自造基建」为未完成——不是复现做得差，是包没覆盖到位。Goal 原则一给的是**组合优先于派生**：一切都是实体 + 属性袋，类型用 schema 表达成数据，新增类型 = 加数据不改包。

**判据（①②③）。** ① 阻断——一个能力提交没选载体：该进内核（需要 Rust 语义）的写成了包，或是本游戏业务却塞进了 `packages/libs`；包内写进了游戏规则。② 症状——同一数据形状在每个游戏里各造一遍；一个包只有一个游戏在用；某个游戏在自己的 mod 里重写实体管理 / 序列化 / 任务流转 / 对话调度。③ 确认——契约存在且被 [../README.md](../README.md) 索引，用例计入 `selftest` 的 `cases passed=`，§5 性能表有数字。
