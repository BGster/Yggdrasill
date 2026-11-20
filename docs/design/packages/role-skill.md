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
| `roles.<name>` | table `{ requires_tags, grants_tags, attrs_spec }` | 是 | — | `ygg.kv` 键 `role.defs`（载荷 = 一张表的 `codec.encode` **字符串**） |
| `skills.<name>` | table `{ requires, charges?, attrs_spec }` | 是 | — | `ygg.kv` 键 `role.skills`（同上） |
| `actors.<id>` | table `{ role, skills = { name = { rank, charges } }, counters = { event = n } }` | 是 | `{}` | `ygg.kv` 键 `role.actor.<id>`（同上；键集恰好 `role` / `skills` / `counters`） |
| `index` | **升序稠密数组**（actor 的实体 id） | 是 | — | `ygg.kv` 键 `role.index`（同上） |

`attrs_spec` = `{ <attr 名> = <number 加成> }`；`requires` = `{ tags = { … }, role = "…" }`（只校验**引用面**能否建立）。

**载荷一律是字符串（D1）。** 宿主 `ygg.kv` 只收四个标量（`crates/ygg-core/src/entity.rs` 的 `Value`；表会被 `value_from_lua` 拒成 `unsupported payload type`），所以四个键的值都是 `codec.encode` 的确定性 JSON 文本，读回时 `codec.decode`。全部 KV 流量收口在 `store.lua` 一个模块里（换编码口径只改一处）。

**索引是数组而不是集合（D2）。** `{ [id] = true }` 存成 JSON 会变成对象键，读回来所有整数键都是字符串（`"1" ~= 1`），成员判定随即失效；数组保留整数，写入时保持升序 ⇒ 字节稳定。**记录不可删**：KV 没有 delete、也没有遍历面（[../kernel/state-surface.md](../kernel/state-surface.md) §2.8.1 是同一事实），所以 `revoke` / `revoke_skill` 只清引用面，既不删记录也不从 `role.index` 摘除——`role.index` 因此是**只增**的。

## 3. 不变量

- **I1 薄层**：角色与技能状态只存**引用**（`role` 名、`skills` 名）与**计数**（`rank` / `charges` / `counters`），不复制实体属性袋。
- **I2 合成而非写回**：`role.effective(id, attr)` = 实体基础值 + 各 `attrs_spec` 加成（合成规则可替换，见 API 面 `role.compose`）。**任何路径都不得把加成写进属性袋**——否则卸下技能要撤销历史，而历史没有单调撤销。
  - 加成列表的顺序是契约：`[角色加成, 技能按名字升序…]`。授予顺序不影响结果——否则两个游戏的同一套技能会因装配顺序不同算出不同加成。
  - `rank` **不参与合成**（D8）：加成只取 `attrs_spec[attr]`；「按等级缩放」是 mod 用 `role.compose` 写的规则，包不猜等级曲线（线性 / 指数 / 分段在四个复现里差得太远）。
  - 基础值缺失记 **0**（不是 `nil`）；非数字的基础值报 `-ATTR`——静默把字符串当 0 加会让装配错误变成「数值偏小」，最难归因。
- **I3 授予幂等**：`role.grant(id, role_name)` 连调两次结果相同，事件只发一次；`revoke` 对未授予的角色返回 `false` 而不报错。
- **I4 引用建立即事件**：技能获取 = ref 建立 + `role.skill_gained`；失去 = `role.skill_lost`。事件名固定，不随技能名变化。
- **I5 计数器只增**：`counters` 由订阅的事件自增，不递减、不清零（清零是 mod 的语义：例如「每回合重置」由 mod 在回合事件里自己 `role.reset_counters`）。handler 只做 `+1`；id 解析不出、或实体没有记录时直接返回（不写不发）——记录由「被授予过什么」建立，不该被一条统计事件凭空创建。
- **I6 无时钟**：冷却与持续时间不得用宿主的时间面（有 API 无推进者，恒 0，见 [../kernel/state-surface.md](../kernel/state-surface.md) §2.8.1）——用事件计数或回合事件表达。订阅面在**安装期**装一次：同一宿主面重复 `install` 不重复注册（否则一次事件 +2，症状是「冷却永远差一倍」）。
- **I7 校验只看引用面**：`grant` 只校验 `requires_tags` / `requires.role` 是否满足（tag 与引用）；等级够不够、能不能学，是 mod 的 handler。
- **I8 顶层纯度**：包模块顶层只有纯导出，定义与注册一律在 `M.install(deps)`。

## 4. API 面

| 函数 | 签名 | 返回 | 副作用类别 |
| --- | --- | --- | --- |
| `role.install` | `(deps)` | 包表 | 写（安装期一次）：`deps = { host?, ygg?, entity?, codec?, events? }`（D3）。`host` 与 `ygg` 两个键都收（批次统一拼法是 `host`）；缺 `entity` / `codec` 即 `-DEPS` 硬失败，不降级去猜别的存储 |
| `role.define` | `(name, spec)` | `true` | 写（`role.defs`）；同 spec 幂等（比较规范化后的编码字节）、异 spec 报 `-DEF` |
| `role.skill_define` | `(name, spec)` | `true` | 写（`role.skills`）；同样的幂等口径 |
| `role.grant` | `(id, role_name)` | `boolean` | 写 + 事件 `role.granted`（幂等：已持有 ⇒ 零写、零事件） |
| `role.revoke` | `(id, role_name)` | `boolean` | 写 + 事件 `role.revoked`；**技能集不动**（技能是独立引用） |
| `role.has` | `(id, role_name)` | `boolean` | 纯读 |
| `role.grant_skill` | `(id, skill_name, rank?)` | `boolean` | 写 + 事件 `role.skill_gained`；已有槽位只更新 `rank`（缺省保持原值）且**不发事件** |
| `role.revoke_skill` | `(id, skill_name)` | `boolean` | 写 + 事件 `role.skill_lost`；记录保留 |
| `role.charge` | `(id, skill_name)` | `number`（剩余） | 写；消耗一格，0 时不减负；**未声明 `charges` ⇒ 返回 0 且不写**（D8） |
| `role.recharge` | `(id, skill_name, n?)` | `number` | 写；恢复（上限取**声明**的 `charges`，不是槽位当前值）；未声明 ⇒ 返回 0 且不写（D8） |
| `role.effective` | `(id, attr)` | `number` | 纯读；基础值 + 合成加成。**任何路径都不写属性袋**（I2） |
| `role.compose` | `(fn)` | `true` | 写（模块级规则）；替换合成规则（缺省「同 attr 相加」），参数为 `(base, list_of_bonuses)`；非函数报 `-COMPOSE`（`nil` = 恢复缺省） |
| `role.count` | `(id, event)` | `number` | 纯读；缺省 0 |
| `role.reset_counters` | `(id, event?)` | `boolean` | 写；显式清零（由 mod 决定时机）。无记录视为已清零（`true` 且不写） |

**返回值语义（D9）**：`grant` 返回**生效后的状态**（连调两次都是 `true`；I3 说的是状态不是变更），`revoke` / `revoke_skill` 返回**原本是否持有**（未持有 `false` 不报错）。这与 `entity.set` 的「返回是否变化」刻意不同——两个包各自的契约把自己那一条写清楚，调用方就不必猜。

**表下注（D8）**：① `rank` **不参与属性合成**——加成只取 `attrs_spec[attr]`，要「按等级缩放」用 `role.compose` 自定义规则；② `effective` 的基础值缺失记 **0**（不是 `nil`，不报错），非数字属性值报 `-ATTR`；③ 未声明 `charges` 的技能不参与充能语义（`charge` / `recharge` 返回 0、不写）。

**事件面恰好四个名字**：`role.granted` / `role.revoked` / `role.skill_gained` / `role.skill_lost`（`pkg.role_skill.EVENTS` 是单源）。订阅方读这张表，不猜名字。

## 5. 性能预算

**口径声明（先读再读数字）**：下列实测值产在 **selftest 面**（普通 Lua + mock KV），因为沙箱里既无 `os.clock` 也无 `collectgarbage`（`crates/ygg-sandbox/src/sandbox.rs` 的白名单测试钉死）。所以内存那行是**包侧 Lua 堆上界**（含 mock KV 的表开销），不是宿主 Rust 堆读数。复算：`cargo run -q --example selftest -p ygg-sandbox -- --mod packages/mods/pkg-selftest --packages packages/libs --filter pkg-role-skill`（三次取中位；`print` 面就是本表口径）。

| 指标 | 阈值（Goal 原则二） | 实测与测量方法 |
| --- | --- | --- |
| 包加载耗时 | `< 10ms` | 目标 `≤ 1ms`；**实测 0.742 ms**（清掉全部 `pkg.role_skill*` 后 `require` + `install({ codec })`，5 次取中位、批内三次取中位；单次 0.651–0.906 ms）。打印 `perf.role-skill.load_ms` |
| 实体按 id 查找 / 更新 | `O(1)` | **强约束**，用**结构证据**而不是只看墙钟：一次 `effective` 的 `kv.get` 次数在 N = 1 / 100 / 10k 三档**恒为 3**（记录 + 角色定义表 + 技能定义表），打印 `perf.role-skill.kv_ops_per_effective=3`；墙钟比 `t(10k)/t(1)` **实测 1.0×**（三次取中位 0.998–1.048），打印 `perf.role-skill.time_ratio_10k_over_1`。判据写 `≤ 1.5×`（1.2 会被 GC 脉冲打成假红，r1 在 [quest.md](quest.md) §5 的同类指标上实证过） |
| 序列化吞吐（实体 / 秒，双向） | 附基准集与耗时 | **不适用**：`role` 前缀由 [codec.md](codec.md) §5 基准集覆盖 |
| 10k 实体常驻内存 | 给出数字 | 1k actor（各 1 角色 + 6 技能）**实测 892.2 KiB**（目标 `≤ 1 MiB`，硬帽 `16 MiB`；`collectgarbage("count")` 差）。打印 `perf.role-skill.mem_1k_actors_kib` |
| 泄漏检测 | 导入导出 100 轮内存不增长 | 100 轮 `grant_skill → revoke_skill` 后：技能集回零、`role.index` 条目数不变、KV 键数不变，内存差 **0.026%**（≤ 1%）。打印 `perf.role-skill.leak_delta_pct` |

两条**已声明的边界**（不是缺陷，是形状的代价）：

- **`role.index` 是「一个键装整张索引」**：每首次为某 id 建立记录就重写整张数组，成本 O(当前条目数)。所以「建 1k actor」是 O(n²) 字节量级的操作（一次性装配可以接受），而 `effective` 完全不受影响——上面那行 O(1) 说的是读面。
- **性能口径不测 `grant` 的索引编码成本**：内存与 O(1) 两行直接铺记录（用 `codec.encode` 写 KV），测的是「载荷常驻」与「读面形状」；把装配成本混进来会让这两条指标变成机器速度的函数。

## 6. 用点计划

| 复现项目 | 用点 | 该包提供什么 | 阶段 |
| --- | --- | --- | --- |
| PokeRogue | 技能 / 特性 / PP | `charges` 直接承载 PP；特性 = `attrs_spec` 加成，合成而非写回 | P3 |
| DCSS | 法术 / 技能 | 技能等级（`rank`）+ 法术记忆集；法术强度经 `effective` 读出 | P3 |
| CDDA | 变异 | 变异体即「角色 + 若干 ref + 加成」——与技能同构，不需要第二套系统 | P3 |

## 7. 自测与验收

- **用例名前缀**：`pkg.role-skill.`（例 `pkg.role-skill.grant-idempotent`、`pkg.role-skill.compose-no-writeback`）。
- **断言面**：I2 用「授予技能后基础值不变、`effective` 变化」的对照断言（这是本包最重要的一条）；I3 / I5 / I7 各一条；I6 用静态用例逐文件扫（`ygg.ticks` / `os.clock` / `os.time` / `math.random` / `set_attribute` / `entity.set(` 六个字面量，源文件清单取 manifest 的 `packages` 段，天然排除 `test/**`）。
- **用例清单（17 条）**：`kv-string-shape` / `grant-idempotent` / `revoke-absent-false` / `grant-unknown-role-errors` / `requires-tags-gate` / `skill-gain-once` / `charge-floor` / `recharge-cap` / `compose-no-writeback` / `compose-custom-order` / `effective-missing-attr-zero` / `counters-increment` / `counters-reset` / `static-no-clock` / `leak-100-rounds` / `perf-o1-structural` / `perf-load`。
- **计数**：17 条计入 `cargo run -q --example selftest -p ygg-sandbox -- --mod packages/mods/pkg-selftest --packages packages/libs --filter pkg-role-skill` 的 `cases passed=`。载体是 `packages/mods/pkg-selftest/`——生产 Mod（`agent-seed`）不声明包，它的 `cases passed=` 永远是 79/0，这是回归钉。
- **冒烟不计数**：`tests/pkg-role-skill-smoke.lua` 是 **file 级**断言（`error` ⇒ `file FAIL` ⇒ `ok=0`），不进 `cases passed=`；它接住 `--filter` 的 mod 侧过滤，并钉装载面、导出面清单、事件名集合与 KV 前缀。
- **禁止宽松断言**：只断 `effective` 返回了数字、不断它等于基础值加各加成之和，视同没测 I2。

## 8. 设计依据

**为什么是「薄层」而不是「角色系统」。** 角色与技能在四个复现项目里都长成一棵树或一张表，但真正的机制只有三件：谁拥有哪个引用、引用带来多少加成、加成怎么合成。把这三件抽出来，剩下的（职业相克、技能树前置、变异副作用）全是规则，留在 mod 才能每个游戏写得不一样。

**反方案一：技能写进实体属性袋。** 最直接，也是最贵的：卸下技能要把属性减回去，而「减回去」需要知道当前值里有多少来自技能——要么存来源账本，要么加一次就再也减不干净。合成读法把这个问题变成不可能发生。

**反方案二：继承树（角色 → 职业 → 技能）。** 加一个「临时变形」或「装备赋予的技能」就要多重继承；Goal 原则一的组合优先正是为此。

**判据（①②③）。** ① 阻断——把加成写回属性袋；在包里判定升级条件或伤害公式；用 `ygg.ticks` 做冷却。② 症状——卸下技能后属性对不上；两个游戏的同一技能加成分属两套算法；计数器随回合无限增长。③ 确认——本文件被索引；`selftest` 含 `pkg.role-skill.*`；§5 五个指标有数字。
