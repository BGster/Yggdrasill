---
type: "Reference"
title: "role-skill — 角色、技能引用与属性合成"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-c9ffd97102395dfea6ace07a
    resource: repo://packages/libs/role-skill/actors.lua
  - id: openwiki-source-e6ffe8d952cf4bd303ed3131
    resource: repo://packages/libs/role-skill/compose.lua
  - id: openwiki-source-810872ef7f8f3a5acb286d8a
    resource: repo://packages/libs/role-skill/counters.lua
  - id: openwiki-source-55e423ccea99bba1ca9b993e
    resource: repo://packages/libs/role-skill/init.lua
  - id: openwiki-source-b6ee4e314d4ca4be5620c380
    resource: repo://packages/libs/role-skill/store.lua
  - id: openwiki-source-2a5ab8f521b133bbbd67c1da
    resource: repo://packages/libs/role-skill/test/pkg-role-skill.lua
  - id: openwiki-source-68813646512ac0240d87170c
    resource: repo://packages/libs/role-skill/window.lua
  - id: openwiki-source-b2bd54854f6065023162029d
    resource: repo://packages/mods/pkg-selftest/tests/pkg-role-skill-smoke.lua
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# role-skill — 角色、技能引用与属性合成

> **id** packages/role-skill · **模块** ⑤ packages · 通用能力层 · **触发** 改角色授予 / 技能引用 / 属性合成或计数面之前
> **读不到代价** silent-drift（属性加成被写回属性袋后，「卸下技能」需要撤销历史，两个游戏的算法悄悄分叉）
> **replaces**：DESIGN §4 role-skill（搬运基线 `3a2536f`）
> **上游**：[README.md](README.md) · **相关**：[entity.md](entity.md)（属性袋真源）· [codec.md](codec.md)（`role` 前缀随存档导出）· [quest.md](quest.md)（同构的扁平键纪律）· [../kernel/lua-api.md](../kernel/lua-api.md)

## 1. 边界（这个包不做什么）

- **不定义角色体系**：职业、门派、阶级——数据。
- **不定义技能效果与数值**：伤害、消耗、冷却怎么算——mod。
- **不复制属性存储**：属性袋的唯一真源是 [entity.md](entity.md)；本包只在读取时**合成**，从不写回。
- **不做战斗结算**：命中判定、伤害减免属 mod 的 handler。
- **不定义经验曲线**：升级条件由 mod 的规则决定，本包只提供「授予 / 撤销 / 查询」。
- **不建继承树**：角色与技能都是实体 + 引用集。

## 2. 数据形状

| 字段 | 类型 | 必填 | 缺省 | 持久化位置 |
| --- | --- | --- | --- | --- |
| `roles.<name>` | table `{ requires_tags, grants_tags, attrs_spec }` | 是 | — | `role.defs`（载荷 = 一张表的 `codec.encode` **字符串**） |
| `skills.<name>` | table `{ requires, charges?, attrs_spec }` | 是 | — | `role.skills`（同上） |
| `actors.<id>` | table `{ role, skills = { name = { rank, charges?, capacity? } }, counters = { event = n } }` | 是 | `{}` | `role.actor.<id>`（同上；键集恰好 `role` / `skills` / `counters`） |
| `window.<id>` | **字符串数组**（旧 → 新，长度 ≤ `WINDOW_CAP`） | 否 | `{}` | `role.window.<id>`（同上；**独立键族**） |
| `index` | **升序稠密数组**（actor 的实体 id） | 是 | — | `role.index`（同上） |

`attrs_spec = { <attr 名> = <number 加成> }`；`requires = { tags = { … }, role = "…" }`（只校验**引用面**能否建立）。

**载荷一律是字符串。** 宿主 `ygg.kv` 只收四个标量（表会被 `value_from_lua` 拒成 `unsupported payload type`），所以每个键的值都是 `codec.encode` 的确定性 JSON 文本，读回时 `codec.decode`。全部 KV 流量收口在 `store.lua` 一个模块里（换编码口径只改一处）。

**索引是数组而不是集合。** `{ [id] = true }` 存成 JSON 会变成对象键，读回来所有整数键都是字符串（`"1" ~= 1`），成员判定随即失效；数组保留整数，写入时保持升序 ⇒ 字节稳定。**记录不可删**：KV 没有 delete、也没有遍历面，所以 `revoke` / `revoke_skill` 只清引用面，既不删记录也不从 `role.index` 摘除——`role.index` 因此是**只增**的。

**`capacity` 是实例级容量。** 有效容量 = 槽位 `capacity` 优先，其次声明的 `charges`，两者皆无则该技能不参与充能（`set_capacity`）。声明面是**静态数据**（同 spec 同值），`capacity` 是**实例状态**（同一技能在不同 actor 上可以不同）——两者分开存，不靠「给每个档位起一个新技能名」编码（PokeRogue 的 `<move>+u<k>` 后缀正是那种临时形态）。

**窗口是独立键族**，不并进 actor 记录：① 记录键集保持「恰好 `role` / `skills` / `counters`」；② 窗口每次练习都写，混进记录会让每次 `charge` 重写整条 ring（写放大 ≈ 100×）；③ 一个 actor 一条流，第二个流没有用点。**窗口也不可从 `counters` 派生**：「最近 n 次」所需的历史已被只增计数器销毁；`window_push` 只追加与回绕，从不删键。

### 事件载荷（`ygg.emit` 只收标量，形状必须定形）

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `role.granted` | `"<id>\|<role>"` | 授予后持有该角色（幂等：已持有不发） |
| `role.revoked` | `"<id>\|<role>"` | 原本持有、现已撤销 |
| `role.skill_gained` | `"<id>\|<skill>\|<rank>"` | 首次建立槽位；升级只改 `rank`、不发事件 |
| `role.skill_lost` | `"<id>\|<skill>"` | 槽位被清；记录本身保留 |

交一张表会在真内核里被硬拒成 `unsupported payload type: table`，而普通 Lua 的 mock 面吃表，掩蔽就一直测不出来。

## 3. 不变量

- **I1 薄层**：角色与技能状态只存**引用**（`role` 名、`skills` 名）与**计数**（`rank` / `charges` / `counters`），不复制实体属性袋。
- **I2 合成而非写回**：`role.effective(id, attr)` = 实体基础值 + 各 `attrs_spec` 加成。**任何路径都不得把加成写进属性袋**——否则卸下技能要撤销历史，而历史没有单调撤销。加成列表顺序是契约：`[角色加成, 技能按名字升序…]`，授予顺序不影响结果（否则两个游戏的同一套技能会因装配顺序不同算出不同加成）。`rank` **不参与合成**：「按等级缩放」用 `role.compose` 写规则，包不猜等级曲线。基础值缺失记 **0**（不是 `nil`）；非数字基础值报 `-ATTR`。
- **I3 授予幂等**：`grant` 连调两次结果相同，事件只发一次；`revoke` 对未授予的角色返回 `false` 而不报错。
- **I4 引用建立即事件**：技能获取 = ref 建立 + `role.skill_gained`；失去 = `role.skill_lost`。事件名固定，不随技能名变化。
- **I5 计数器只增**：`counters` 由订阅的事件自增，不递减、不清零（清零是 mod 的语义：`role.reset_counters` 由 mod 在回合事件里调）；handler 只做 `+1`，id 解析不出或实体没有记录时直接返回（不写不发）——记录由「被授予过什么」建立，不该被一条统计事件凭空创建。
- **I6 无时钟**：冷却与持续时间不得用宿主时间面（有 API 无推进者，恒 0）——用事件计数或回合事件表达。订阅面在**安装期**装一次：同一宿主面重复 `install` 不重复注册（否则一次事件 +2，症状是「冷却永远差一倍」）。
- **I7 校验只看引用面**：`grant` 只校验 `requires_tags` / `requires.role` 是否满足；等级够不够、能不能学，是 mod 的 handler。
- **I8 顶层纯度**：包模块顶层只有纯导出，定义与注册一律在 `M.install(deps)`。
- **I9 兼容增量**：既有 14 个导出签名与语义**只增不改**——新面只加导出与可选参数，既有调用路径逐字节同结果（`slot.capacity` 只由 `set_capacity` / `recharge` 第 4 参写入，`grant_skill` 建槽位仍只写 `rank` / `charges`）。错误码表不扩（仍那 7 个）。
- **I10 窗口与计数器分工**：`counters` 只增，表达冷却；`window` 是**有界近期流**（FIFO 回绕），表达「最近 n 次」。两者**不互推**；`window_push` 不写 `counters`，`reset_counters` 不影响窗口。窗口按**插入序**成立，不碰任何时间面。

## 4. API 面

| 函数 | 签名 | 返回 | 副作用类别 |
| --- | --- | --- | --- |
| `role.install` | `(deps)` | 包表 | 写（安装期一次）：`deps = { host?, ygg?, entity?, codec?, events?, window_cap? }`；`host` 与 `ygg` 两个键都收；缺 `entity` / `codec` 即 `-DEPS` 硬失败，不降级去猜别的存储；`window_cap` 是正整数（缺省 100），非法报 `-INSTALL` |
| `role.define` | `(name, spec)` | `true` | 写（`role.defs`）；同 spec 幂等（比较规范化后的编码字节）、异 spec 报 `-DEF` |
| `role.skill_define` | `(name, spec)` | `true` | 写（`role.skills`）；同样的幂等口径 |
| `role.grant` | `(id, role_name)` | `boolean` | 写 + `role.granted`（幂等：已持有 ⇒ 零写、零事件） |
| `role.revoke` | `(id, role_name)` | `boolean` | 写 + `role.revoked`；**技能集不动** |
| `role.has` | `(id, role_name)` | `boolean` | 纯读 |
| `role.grant_skill` | `(id, skill_name, rank?)` | `boolean` | 写 + `role.skill_gained`；已有槽位只更新 `rank`（缺省保持原值）且**不发事件** |
| `role.revoke_skill` | `(id, skill_name)` | `boolean` | 写 + `role.skill_lost`；记录保留 |
| `role.charge` | `(id, skill_name)` | `number`（剩余） | 写；消耗一格，0 时不减负；**未声明 `charges` ⇒ 返回 0 且不写** |
| `role.recharge` | `(id, skill_name, n?, cap?)` | `number` | 写；上限取**有效容量**；未声明且无 `capacity` ⇒ 返回 0 且不写；第 4 参 `cap` = **持久化**的新容量（≡ `set_capacity` 后填充），不引第二真源 |
| `role.effective` | `(id, attr)` | `number` | 纯读；基础值 + 合成加成。**任何路径都不写属性袋** |
| `role.compose` | `(fn)` | `true` | 写（模块级规则）；替换合成规则（缺省「同 attr 相加」），参数 `(base, list_of_bonuses)`；非函数报 `-COMPOSE`（`nil` = 恢复缺省） |
| `role.count` | `(id, event)` | `number` | 纯读；缺省 0 |
| `role.reset_counters` | `(id, event?)` | `boolean` | 写；显式清零。无记录视为已清零（`true` 且不写） |
| `role.skills_of` | `(id)` | **升序名字数组**（`1..k` 稠密） | 纯读（1 次 `kv.get`）；按 `<` 插入排序（沙箱没有 `table.sort`），`rank = 0` 的槽位也在 |
| `role.skill_state` | `(id, skill_name)` | 槽位**副本** `{ rank, charges?, capacity? }` 或 `nil` | 纯读（1 次 `kv.get`）；**不并入**声明的 `charges`（声明面是 mod 的数据，包内没有 defs 读面） |
| `role.set_capacity` | `(id, skill_name, cap)` | `cap` | 写；**槽位不存在 ⇒ `-UNKNOWN`**（fail-closed）。缩容夹紧 `charges = min(charges, cap)`；放大**不补满**；`charges` 缺失 ⇒ 初始化为 `cap` |
| `role.window_push` | `(id, label)` | ring 长度 | 写（1 get + 1 set）；`label` 非空字符串（否则 `-DEF`），**不要求**已定义 / 已授予（流是标签面） |
| `role.window` | `(id, label, n)` | 最近 `n` 条里等于 `label` 的**条数** | 纯读（1 次 `kv.get`）；`n = 0` / 无键 / 标签不在 ⇒ `0`；`n` 非整数或负数 ⇒ `-DEF`；**`n > WINDOW_CAP` ⇒ `-DEF`**（不静默截断） |

**返回值语义**：`grant` 返回**生效后的状态**（连调两次都是 `true`；I3 说的是状态不是变更），`revoke` / `revoke_skill` 返回**原本是否持有**（未持有 `false` 不报错）。这与 `entity.set` 的「返回是否变化」刻意不同——各包的契约把自己那条写清楚，调用方就不必猜。

**事件面恰好四个名字**：`pkg.role_skill.EVENTS` 是单源，订阅方读这张表，不猜名字。错误码面 7 个（`-DEPS` / `-INSTALL` / `-DEF` / `-ATTR` / `-COMPOSE` / `-UNKNOWN` 及环境面同义码）。

## 5. 性能预算

**口径声明**：下列实测值产在 **selftest（普通 Lua + mock KV）**，因为沙箱里既无 `os.clock` 也无 `collectgarbage`。所以内存那行是**包侧 Lua 堆上界**（含 mock KV 的表开销），不是宿主 Rust 堆读数。复算：`--filter pkg-role-skill`（三次取中位）。

| 指标 | 阈值 | 实测与测量方法 |
| --- | --- | --- |
| 包加载耗时 | `< 10ms` | 目标 `≤ 1ms`；**实测 0.913 ms**（清掉全部 `pkg.role_skill*` 后 `require` + `install({ codec })`，5 次取中位、批内三次取中位；S8 面 0.870–0.968 ms）。打印 `perf.role-skill.load_ms` |
| 实体按 id 查找 / 更新 | `O(1)` | **强约束，用结构证据而不是只看墙钟**：一次 `effective` 的 `kv.get` 次数在 N = 1 / 100 / 10k 三档**恒为 3**（记录 + 角色定义表 + 技能定义表），打印 `perf.role-skill.kv_ops_per_effective=3`；墙钟比 `t(10k)/t(1)` **实测 1.021×**（1.018–1.051），判据 `≤ 1.55×`（1.5 曾被实测 1.50 摸到边界，余量按红值 +3.3%）。打印 `perf.role-skill.time_ratio_10k_over_1` |
| 序列化吞吐 | 附基准集与耗时 | **不适用**：`role` 前缀由 [codec.md](codec.md) §5 基准集覆盖 |
| 10k 实体常驻内存 | 给出数字 | 1k actor（各 1 角色 + 6 技能）**实测载荷增量 300.8 KiB**（目标 `≤ 1 MiB`，硬帽 `16 MiB`；**差值口径**：基线含宿主与注入资产，先 `seed(3000, 1)` 预载 ≈3× 被测载荷、被测段铺 1000 条新记录，判据只量增量；`collectgarbage("count")` 差，**读前显式 `collect`**）。两面同值：仓内 selftest 面 / PokeRogue 注入面（952 技能定义计入基线）均 300.8 KiB，读数一致即解耦成立 |
| 泄漏检测 | 导入导出 100 轮内存不增长 | 100 轮 `grant_skill → revoke_skill` 后：技能集回零、`role.index` 条目数不变、KV 键数不变，内存差 **0.144%**（≤ 1%） |
| 读面 / 实例容量 / 窗口（S8 新增面） | 与既有 `charge` **同带**（`≤ 1.0×` / `≤ 1.5×` / `≤ 2.5×` / `≤ 4.0×`） | 同一 actor、`charge` 作基准（余量 1e9 ⇒ 恒走写路）：`skills_of` **0.573×**、`skill_state` **0.567×**（判据 `≤ 1.0×`）、`set_capacity` **1.003×**（`≤ 1.5×`）、`window`(n=100) **2.209×**（`≤ 2.5×`）、`window_push` **3.641×**（`≤ 4.0×`；`recharge` 2.083× 作参考）。**结构证据**：`skills_of` / `skill_state` / `window` 各 **1 次 `kv.get`**，`window_push` **1 get + 1 set**，`set_capacity` **2 get + 1 set**。窗口载荷 100 条 = **901 B**（判据 `≤ 1 KiB`），`window_cap=100` |

三条**已声明的边界**（不是缺陷，是形状的代价）：

- **`role.index` 是「一个键装整张索引」**：每首次为某 id 建立记录就重写整张数组，成本 O(当前条目数)，所以「建 1k actor」是 O(n²) 字节量级的操作（一次性装配可接受），而 `effective` 完全不受影响——上面那行 O(1) 说的是读面。
- **性能口径不测 `grant` 的索引编码成本**：内存与 O(1) 两行直接铺记录（用 `codec.encode` 写 KV），测的是「载荷常驻」与「读面形状」；把装配成本混进来会让这两条指标变成机器速度的函数。
- **`window` 每次读全载荷**：`window(id, label, n)` 是 1 次 `kv.get` + 整个 ring 的解码（100 条 ≈ 901 B）⇒ 2.209×；k 个标签要 k 次读（DCSS 25 技能 ≈ 2.2 ms/回合，原型 89 µs × 25）。要收敛成 1 次读需要一条「整窗口」读面（`window_of`），那会多一份不变量与用例——本轮不做。

## 6. 用点计划

| 复现项目 | 用点 | 该包提供什么 | 阶段 |
| --- | --- | --- | --- |
| PokeRogue | 技能 / 特性 / PP | `charges` 直接承载 PP；特性 = `attrs_spec` 加成，合成而非写回 | P3 |
| DCSS | 法术 / 技能 | 技能等级（`rank`）+ 法术记忆集（`#skills_of` 计数到 21 槽）；法术强度经 `effective` 读出 | P3 |
| CDDA | 变异 | 变异体即「角色 + 若干 ref + 加成」——与技能同构，不需要第二套系统 | P3 |

**S8 三面与移植仓的对应**（每面指向来源仓现状；三仓替换是后续小改）：

| 面 | 移植仓 | 现状（该仓 `file:line`） | 本包提供 | 阶段 |
| --- | --- | --- | --- | --- |
| 读面 ① | PokeRogue | `mod/src/domain/moves.lua:5-6`——没有「列某人全部技能」的读面，槽位身份只能落 `mv1..4` / `ppu1..4` 属性 | `skills_of` / `skill_state` | P4（替换待做） |
| 读面 ① | DCSS | `mod/src/domain/spells.lua:5`（记忆集落 `dcss.spells` 串）、`:17`（`MAX_SLOTS=21`）、`:64-85`（满槽挤最旧） | 同上：槽位数 = `#skills_of`；**置换顺序仍需 mod 侧有序表**（JSON 对象不保序，`skills_of` 只给升序名字） | P4（部分可替换） |
| 实例容量 ② | PokeRogue | `mod/src/domain/moves.lua:8`（档位用技能名后缀 `<move>+u<k>`，惰性 `skill_define`）、`:23-49` | `set_capacity` + `recharge` 第 4 参 | P4（替换待做） |
| 滑动窗口 ③ | DCSS | `mod/src/domain/skills.lua:4-6`（`role.count` 单调，做不出「最近 100 次」）、`:22`（`QUEUE_SIZE=100`）、`:184-195`（FIFO 挤最旧） | `window_push` / `window` | P4（替换待做） |

## 7. 自测与验收

- **用例名前缀**：`pkg.role-skill.`。
- **断言面**：I2 用「授予技能后基础值不变、`effective` 变化」的对照断言（本包最重要的一条）；I3 / I5 / I7 各一条；**I9 用对照断言**（`capacity-recharge-cap`：3 参 `recharge` 的返回值与载荷字节同前，4 参才出现 `capacity`）；**I10 用隔离断言**（`window-vs-counters`）；I6 用静态用例逐文件扫（`ygg.ticks` / `os.clock` / `os.time` / `math.random` / `set_attribute` / `entity.set(` 六个字面量，源文件清单取 manifest 的 `packages` 段，天然排除 `test/**`）。
- **用例清单（28 条）**：`kv-string-shape` / `grant-idempotent` / `revoke-absent-false` / `grant-unknown-role-errors` / `requires-tags-gate` / `skill-gain-once` / `charge-floor` / `recharge-cap` / `compose-no-writeback` / `compose-custom-order` / `effective-missing-attr-zero` / `counters-increment` / `counters-reset` / `static-no-clock` / `leak-100-rounds` / **S8 读面** `read-skills-of` / `read-skill-state` / **S8 容量** `capacity-grow` / `capacity-shrink` / `capacity-errors` / `capacity-recharge-cap` / **S8 窗口** `window-push-read` / `window-ring-wrap` / `window-n-over-cap` / `window-vs-counters` / **兼容面** `compat-export-face` / `perf-o1-structural` / `perf-load`。
- **计数**：**28** 条计入 `--filter pkg-role-skill` 的 `cases passed=`（S8 前 17 条，S8 加 11 条）；包侧全量见 [README.md](README.md) 的实测表。载体是 `packages/mods/pkg-selftest/`——生产 Mod（`agent-seed`）不声明包，它的计数永远是 79/0，这是回归钉。
- **冒烟不计数**：`tests/pkg-role-skill-smoke.lua` 是 **file 级**断言，不进 `cases passed=`；它接住 `--filter` 的 mod 侧过滤，并钉装载面、导出面清单（19 个导出）、事件名集合与 KV 前缀（4 条键名）。
- **真沙箱面不计入 `cases passed=`**：四个派生事件的**载荷形状**由 `cargo test -p ygg-sandbox --test packages_sandbox` 的用例 ① 钉住。普通 Lua 的 mock `ygg.emit` 吃表，所以表载荷在 `cases passed=` 这一面**测不出来**（本包四处表载荷就是这么活到 r2 才被冒烟抓住）。
- **禁止宽松断言**：只断 `effective` 返回了数字、不断它等于基础值加各加成之和，视同没测 I2。

## 8. 设计依据

**为什么是「薄层」而不是「角色系统」。** 角色与技能在四个复现项目里都长成一棵树或一张表，但真正的机制只有三件：谁拥有哪个引用、引用带来多少加成、加成怎么合成。把这三件抽出来，剩下的（职业相克、技能树前置、变异副作用）全是规则，留在 mod 才能每个游戏写得不一样。

**反方案一：技能写进实体属性袋**——最直接也最贵：卸下技能要把属性减回去，而「减回去」需要知道当前值里有多少来自技能，要么存来源账本，要么加一次就再也减不干净；合成读法把这个问题变成不可能发生。**反方案二：继承树（角色 → 职业 → 技能）**——加一个「临时变形」或「装备赋予的技能」就要多重继承；组合优先正是为此。

**判据（①②③）**。① 阻断——把加成写回属性袋；在包里判定升级条件或伤害公式；用 `ygg.ticks` 做冷却。② 症状——卸下技能后属性对不上；两个游戏的同一技能加成分属两套算法；计数器随回合无限增长。③ 确认——本页被索引；`selftest` 含 `pkg.role-skill.*`；§5 六行指标有数字。

维护责任 = 改 `packages/libs/role-skill/**` 的执行者；触发 = 任一相关 commit；检查 = `--filter pkg-role-skill` 计数 + `kv_ops_per_effective=3` + `packages_sandbox` 用例 ①。
