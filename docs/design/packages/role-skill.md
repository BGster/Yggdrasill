# Yggdrasill · packages/role-skill — 角色、技能引用与属性合成

> **id** packages/role-skill · **模块** ⑤ packages · 通用能力层 · **触发** 改角色授予 / 技能引用 / 属性合成或计数面之前
> **读不到代价** silent-drift（属性加成被写回属性袋后，「卸下技能」需要撤销历史，两个游戏的算法悄悄分叉）

> **上游**：[../README.md](../README.md) · **相关**：[README.md](README.md)（模板与命名规则）· [entity.md](entity.md)（属性袋真源）· [codec.md](codec.md)（`role` 前缀随存档导出）· [../kernel/lua-api.md](../kernel/lua-api.md)（`ygg.entity.*`）

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
| `roles.<name>` | table `{ requires_tags, grants_tags, attrs_spec }` | 是 | — | `ygg.kv` 键 `role.defs` |
| `skills.<name>` | table `{ requires, charges?, attrs_spec }` | 是 | — | `ygg.kv` 键 `role.skills` |
| `actors.<id>` | table `{ role, skills = { name = { rank, charges } }, counters = { event = n } }` | 是 | `{}` | `ygg.kv` 键 `role.actor.<id>` |
| `index` | table（actor 的实体 id 集合） | 是 | — | `ygg.kv` 键 `role.index` |

`attrs_spec` = `{ <attr 名> = <number 加成> }`；`requires` = `{ tags = { … }, role = "…" }`（只校验**引用面**能否建立）。

## 3. 不变量

- **I1 薄层**：角色与技能状态只存**引用**（`role` 名、`skills` 名）与**计数**（`rank` / `charges` / `counters`），不复制实体属性袋。
- **I2 合成而非写回**：`role.effective(id, attr)` = 实体基础值 + 各 `attrs_spec` 加成（合成规则可替换，见 API 面 `role.compose`）。**任何路径都不得把加成写进 `ygg.entity.set_attribute`**——否则卸下技能要撤销历史，而历史没有单调撤销。
- **I3 授予幂等**：`role.grant(id, role_name)` 连调两次结果相同，事件只发一次；`revoke` 对未授予的角色返回 `false` 而不报错。
- **I4 引用建立即事件**：技能获取 = ref 建立 + `role.skill_gained`；失去 = `role.skill_lost`。事件名固定，不随技能名变化。
- **I5 计数器只增**：`counters` 由订阅的事件自增，不递减、不清零（清零是 mod 的语义：例如「每回合重置」由 mod 在回合事件里自己 `role.reset_counters`）。
- **I6 无时钟**：冷却与持续时间不得用 `ygg.ticks()`（有 API 无推进者，恒 0，见 [../kernel/state-surface.md](../kernel/state-surface.md) §2.8.1）——用事件计数或回合事件表达。
- **I7 校验只看引用面**：`grant` 只校验 `requires_tags` / `requires.role` 是否满足（tag 与引用）；等级够不够、能不能学，是 mod 的 handler。
- **I8 顶层纯度**：包模块顶层只有纯导出，定义与注册一律在 `M.install(deps)`。

## 4. API 面

| 函数 | 签名 | 返回 | 副作用类别 |
| --- | --- | --- | --- |
| `role.define` | `(name, spec)` | `true` | 写（`role.defs`）；异 spec 重复定义报错 |
| `role.skill_define` | `(name, spec)` | `true` | 写（`role.skills`） |
| `role.grant` | `(id, role_name)` | `boolean` | 写 + 事件 `role.granted`（幂等） |
| `role.revoke` | `(id, role_name)` | `boolean` | 写 + 事件 `role.revoked` |
| `role.has` | `(id, role_name)` | `boolean` | 纯读 |
| `role.grant_skill` | `(id, skill_name, rank?)` | `boolean` | 写 + 事件 `role.skill_gained` |
| `role.revoke_skill` | `(id, skill_name)` | `boolean` | 写 + 事件 `role.skill_lost` |
| `role.charge` | `(id, skill_name)` | `number`（剩余） | 写；消耗一格，0 时不减负 |
| `role.recharge` | `(id, skill_name, n?)` | `number` | 写；恢复（上限由 `charges` 声明） |
| `role.effective` | `(id, attr)` | `number` | 纯读；基础值 + 合成加成 |
| `role.compose` | `(fn)` | `true` | 写；替换合成规则（缺省「同 attr 相加」），参数为 `(base, list_of_bonuses)` |
| `role.count` | `(id, event)` | `number` | 纯读 |
| `role.reset_counters` | `(id, event?)` | `boolean` | 写；显式清零（由 mod 决定时机） |

## 5. 性能预算

| 指标 | 阈值（Goal 原则二） | 本包取值与测量方法 |
| --- | --- | --- |
| 包加载耗时 | `< 10ms` | 目标 `≤ 1ms`（纯 Lua，`install` 不预读定义）。测法同 [entity.md](entity.md) §5。 |
| 实体按 id 查找 / 更新 | `O(1)` | **强约束**：`role.actor.<id>` 一次 KV 直取；`effective` 只在该实体自己的技能集上求和（宽度 = 技能数，与实体总数无关）。测法：实体数 1 / 100 / 10k 三档耗时不随 N 增长（≤ 1.2×）。 |
| 序列化吞吐（实体 / 秒，双向） | 附基准集与耗时 | **不适用**：`role` 前缀由 [codec.md](codec.md) §5 基准集覆盖。 |
| 10k 实体常驻内存 | 给出数字 | 目标 1k actor（各 1 角色 + 6 技能）`≤ 1 MiB`；硬帽 `16 MiB`。口径同 [entity.md](entity.md) §5。 |
| 泄漏检测 | 导入导出 100 轮内存不增长 | 100 轮 `grant_skill → revoke_skill` 后 `role.index` 条目数不变、技能集尺寸回零，内存差 `≤ 1%`。 |

## 6. 用点计划

| 复现项目 | 用点 | 该包提供什么 | 阶段 |
| --- | --- | --- | --- |
| PokeRogue | 技能 / 特性 / PP | `charges` 直接承载 PP；特性 = `attrs_spec` 加成，合成而非写回 | P3 |
| DCSS | 法术 / 技能 | 技能等级（`rank`）+ 法术记忆集；法术强度经 `effective` 读出 | P3 |
| CDDA | 变异 | 变异体即「角色 + 若干 ref + 加成」——与技能同构，不需要第二套系统 | P3 |

## 7. 自测与验收

- **用例名前缀**：`pkg.role-skill.`（例 `pkg.role-skill.grant-idempotent`、`pkg.role-skill.compose-no-writeback`）。
- **断言面**：I2 用「授予技能后 `ygg.entity.get_attribute` 的基础值不变、`effective` 变化」的对照断言（这是本包最重要的一条）；I3 / I5 / I7 各一条；I6 用「包内不出现 `ygg.ticks` 字面量」的静态用例。
- **计数**：计入 `cargo run -q --example selftest -- --mod .ygg/mods/agent-seed` 的 `cases passed=`（发现机制见 [README.md](README.md) §5 未决项）。
- **禁止宽松断言**：只断 `effective` 返回了数字、不断它等于基础值加各加成之和，视同没测 I2。

## 8. 设计依据

**为什么是「薄层」而不是「角色系统」。** 角色与技能在四个复现项目里都长成一棵树或一张表，但真正的机制只有三件：谁拥有哪个引用、引用带来多少加成、加成怎么合成。把这三件抽出来，剩下的（职业相克、技能树前置、变异副作用）全是规则，留在 mod 才能每个游戏写得不一样。

**反方案一：技能写进实体属性袋。** 最直接，也是最贵的：卸下技能要把属性减回去，而「减回去」需要知道当前值里有多少来自技能——要么存来源账本，要么加一次就再也减不干净。合成读法把这个问题变成不可能发生。

**反方案二：继承树（角色 → 职业 → 技能）。** 加一个「临时变形」或「装备赋予的技能」就要多重继承；Goal 原则一的组合优先正是为此。

**判据（①②③）。** ① 阻断——把加成写回属性袋；在包里判定升级条件或伤害公式；用 `ygg.ticks` 做冷却。② 症状——卸下技能后属性对不上；两个游戏的同一技能加成分属两套算法；计数器随回合无限增长。③ 确认——本文件被索引；`selftest` 含 `pkg.role-skill.*`；§5 五个指标有数字。
