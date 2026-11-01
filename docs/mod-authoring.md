# Yggdrasill Mod 作者指南（M0）

> 面向 Mod 作者的创作期踩坑清单转正——每一条都来自 packages/mods/test-adventure
> （《灯下谜径》）实写作证，语义以内核源码为准（host_api.rs / driver.rs /
> stdlib/fsm.lua / stdlib/narrative.lua）。类型补全请配合
> [lua-stubs](../lua-stubs/README.md)（LuaLS 注解面）接入 IDE。

## 1. 实体句柄模式：id 落 kv，勿存 local

`ygg.entity.spawn()` 返回整数 id。**这个 id 必须立刻落 `ygg.kv`**：

```lua
-- ✅ 正确：spawn 加 kv 门防重复，id 落 kv 持久化
if ygg.kv.get("hero.id") == nil then
  local hero = ygg.entity.spawn()
  ygg.entity.set_attribute(hero, "hp", 10)
  ygg.kv.set("hero.id", hero)
end
local hero = ygg.kv.get("hero.id")   -- handler 内一律从 kv 取回
```

```lua
-- ❌ 错误：id 存顶层 local
local hero = ygg.entity.spawn()      -- 顶层：装载期执行
```

原因（两条，均为存档语义的必然推论）：

1. **读档 = Mod chunk 重装载 + 状态覆写**。存档只携带实体表 / KV /
   待决选择，不携带 Lua 全局与 local——读档后 Mod 顶层代码重跑一遍，
   旧 local 里的 id 与覆写后的 store 不再对应；而顶层重跑还会**再次
   spawn 造成重复实体**，所以 spawn 必须加 kv 门（见上例）。
2. **kv 是唯一跨存读档存活的 Mod 侧记忆**。handler 内要用实体，从
   `ygg.kv.get` 取回 id——存档内实体 id 稳定，取回即同一实体。

同理适用于一切「装载期算出、运行期要再用」的值：要么落 kv，要么像
资源快照那样只读不引用状态（见 §5）。

## 2. choice 是 handler 的最后一个输出（终结语义）

`ygg.choice(prompt, options)` 以哨兵错误**终止当前 handler 的执行**——
choice 之后同 handler 内的任何代码都不会运行。这不是风格问题，是内核
「choice-as-state：挂起即数据」的终结语义。

**反例（emit-then-choice，会静默丢事件）**：

```lua
-- ❌ 错误：先 emit 再 choice
ygg.narrative.node("node:crossroad", function()
  ygg.emit("chapter.advance", "to_road")   -- 先入队的派生事件……
  ygg.choice("去哪儿？", { "地窖", "客房" }) -- ……被这里终结丢弃
end)
```

丢弃机理：choice 触发转换层把 `pending_choice` 置位后，同一 handler
先入队 FIFO 的派生事件全部计 `eventsDropped`——「chapter.advance」永远
不会派发，状态机停在旧态（《灯下谜径》初版实证踩中）。

**运行期警告（迭代 D 起）**：驱动不再让你靠肉眼踩坑——handler 在
`ygg.choice` 之前调用过 `ygg.emit` 时，本次派发信封会新增**加性可选键**
`warnings`（单元素字符串数组，非空才出现），文案含事件名与丢弃条数，如：

```json
{"ok":true,"event":"event.go","warnings":["事件 `event.go` 的 handler 在 ygg.choice 之前产生了 2 条派生事件（ygg.emit），已按语义丢弃（choice 须为 handler 最后一个输出调用）"],...}
```

两点边界：① 只计 `ygg.emit`——`ygg.narrate` 在 choice 之前的输出**不丢**
（直进叙述缓冲，信封 `narrative` 可见），把它计入警告会与信封自相矛盾；
② 每次 dispatch 至多一条警告（多个 handler 都踩坑也只报首条）。终端
示例 `cargo run --example adventure` 会以 `⚠ ` 前缀直接打印。

**正确做法**：
把 emit 挪到选择回应 handler（`ygg.choice_selected`）里——那里没有
choice 终结，FIFO 完整排空：

```lua
-- ✅ 正确：choice 只收尾；章节推进在 choice_selected handler 里 emit
ygg.narrative.node("node:tavern", function()
  ygg.narrate(TEXTS.scenes.tavern)
  ygg.kv.set("stage", "tavern")            -- 记住待回应点
  ygg.choice(TEXTS.prompts.tavern, { ... }) -- 必须是最后一个输出
end)

ygg.on("ygg.choice_selected", function(index)
  ygg.emit("chapter.advance", "to_road")   -- 无 choice 终结，安全
  ygg.narrative["goto"]("node:crossroad")
end)
```

多个选择点可共用一个 `ygg.choice_selected` handler，用 kv 旗（如
`stage`）区分当前待回应点。**「choice 后 emit」静态检测已由 ygg-lint
R1 规则承载**（M1 交付：`cargo run -p ygg-lint -- <mod.lua>` 直接可跑，
管线的 lint 门亦经该规则拦截）——运行期警告（见上）作为第二保险保留。

## 3. fsm 语义：声明序 / when / initial 落盘

`ygg.fsm.define(spec)` 一机一事件；状态持久化在 kv 键 `"fsm." .. name`
下，随存档往返。规则逐条：

- **声明序先到先得**：按 `transitions` 声明序（ipairs 严格序）逐边尝试，
  首个 `from` 匹配且条件成立的边胜出；**无匹配 = 静默 no-op**（不报错）。
- **when 条件边（可选）**：边上可声明 `when = function(payload) return
  条件 end`（truthy 即通过；无 `when` 视为恒真）。同一 `from` 的多出边
  靠 when 分派——

  ```lua
  transitions = {
    { from = "road", to = "cellar", when = function(p) return p == 1 end },
    { from = "road", to = "inn" },  -- 无 when = 兜底边
  },
  ```

  注意 when 在**定义期**校验：不是 function 也不是 nil 时，define 当场
  报错（ModLoad，文案带 `transitions[i].when`）。
- **initial 首触发落盘**：机器第一次收到事件且 kv 键缺席时，**先写
  initial 再参与匹配**（幂等——键已存在则不动）。因此「机器已被触发过」
  在 KV 面可见（`ygg.kv.get("fsm.door")` 不再是 nil）。
- **未触发不落盘**：从未收到过事件的机器不写任何 KV——排查「为什么没有
  `fsm.xxx` 键」时先确认事件真的派发过（无匹配的事件也会落 initial，
  纯静默期不落）。

## 4. 事件 payload 规则

- **事件路径 payload 必填且必须是 JSON 标量**：整数 / 浮点 / 布尔 /
  字符串。空串、嵌套对象 / 数组 / null 均为 VALIDATION 错误信封
  （core Value 只有四标量）。例：`dispatch("chapter.advance", "\"to_road\"")`
  或 `dispatch("ygg.choice_selected", "1")`。
- **保留命令忽略 payload**：`save / load / state / log / drain` 五个名字
  保留为驱动命令，payload 传空串即可。事件名不得与这五个保留名重名
  （`ygg.choice_selected` 等不受限）。
- **空 payload 放宽与否**是已记录的 open 问题，当前不放宽——写事件
  派发时养成恒传标量的习惯（无语义时传 `0` / `"0"` 均可）。

## 5. 资源包结构与 pack 工具

### 目录结构

```text
packages/mods/<你的mod>/
├── mod.lua          Mod 本体（装载期注册 handler 与叙事节点）
├── pack.json        打包声明（pack 工具的输入，见下）
├── manifest.json    资源清单（pack 工具生成，勿手改哈希）
├── data/*.json      资源数据（全部文案 / 数值调参，经 schema 校验）
└── schemas/*.json   资源 schema（2020-12 锚；装载方按 id 注册分发校验）
```

`pack.json` 声明条目元数据，**哈希与字节数由工具现算**（改一个字节的
文案不再需要手算 sha256）：

```json
{
  "schemaVersion": 1,
  "entries": [
    { "id": "ygg:scene/texts", "kind": "scene-texts",
      "schema": "https://yggdrasill.dev/schemas/v1/scene-texts.json",
      "path": "data/scenes.json" }
  ],
  "schemaFiles": ["schemas/scene-texts.json"]
}
```

### seed 模板与项目实例的分层

同一个仓库里可以有两份同构的 Mod，但**它们不是同一份东西**：

| 位置 | 角色 |
| --- | --- |
| `packages/mods/agent-seed/` | **种子模板**：通用元工作流，对外分发；`data/playbook.json` 只装通用纪律 |
| `.ygg/mods/agent-seed/` | **本项目实例**：`data/` 装本仓库自己的规则与配置 |

- **骨架面**（`mod.lua` + `src/**/*.lua` + `schemas/`，以及 `pack.json` 的 `code` 段）必须逐字节一致——
  「同一套 harness」不允许有两种机制行为，也不允许两份 `code.modules` 声明不同的模块集合。
  用 `scripts/sync-seed-mod.sh` 同步（`--from project` 反向回流；脚本会检查两侧 `code` 段一致），
  脚本自动重算两侧 `manifest.json`；CI 有一道骨架守卫（`mod.lua` / `src/` / `schemas/` / `code` 段
  四查 + schemas 与 `packages/schema/schemas/` 同源），单侧改动会在合入前失败。
- **实例面**（`data/`，以及 `pack.json` 的 `agent` / `inboxRoutes`）必然不同（playbook 装的是各自项目的
  规则、前缀集必须与各自 scenes 的实际用词一致），任何同步都会毁掉其中一侧，所以脚本不碰它们、
  CI 也不校验。
- 拿模板开新项目：复制到目标仓 `.ygg/mods/` → 填 `data/playbook.json` 的项目章节 →
  改 `data/config.json` → 跑 `pack` / `ygg-lint` / `playtest` 三件套。步骤见
  `packages/mods/agent-seed/README.md`。

### 生成 manifest

```bash
cargo run -p ygg-sandbox --example pack -- packages/mods/<你的mod>
```

工具行为：对每个 `entries[].path` 从磁盘实际字节现算 sha256（小写 hex）
与 byteLength，生成 / 覆写 `manifest.json`；`schemaFiles` 逐个校验存在性
（漏带 schema 文件是装载期 `UnknownSchema` 的常见根因，打包期提前拦截）。
可读报错覆盖：pack.json 缺失 / 非法 JSON / 字段缺失、id 或 path 重复
声明、path 越界（`..` 或绝对路径）、数据文件缺失。

### 多文件 Mod（`code` 段）

> **已有单文件 Mod 想拆分**：分阶段步骤、每阶段验收口径、`§` 分区到模块的映射表、
> 两类硬骨头（全局函数 / 闭包上值）与派发序对照方法，见
> [`migration-single-to-multi.md`](migration-single-to-multi.md)。

单文件与多文件走**同一条装载管线**：没有 `code` 段时模块表为空，`require`
照样存在，但对任何名字都给可读拒绝
（`sandbox denied: module 'x.y' is not available …`）——这就是既有行为，
**现有单文件 Mod 零迁移**，行为逐字节不变。`mod.lua` 超过几百行、或想按职责
切分之后，再写 `pack.json` 的可选 `code` 段。

拆文件之前先记住那条不变量：**模块 chunk 只做纯导出（定义函数与常量），注册 /
写状态 / 输出全部集中在入口 `mod.lua`**——所以「文件怎么拆」不影响运行时行为，
重构保行为。细则见下文「副作用边界」。

```json
{
  "schemaVersion": 1,
  "entries": [ ... ],
  "schemaFiles": [ ... ],
  "code": { "entry": "mod.lua", "modules": ["src/util.lua", "src/deep/leaf.lua"] }
}
```

#### 声明面：`entry` 不是模块

- `entry`：入口文件名（宿主按它读 Mod 源，缺省回落 `mod.lua`）。入口**不登记**
  为模块——同一份源码有两个装载身份只会让缓存与错误定位产生歧义；把它同时写进
  `modules` 是打包期错误。
- `modules`：**逐条枚举**能被 `require` 的模块文件，**无 glob 展开**
  （路径含 `* ? [ ] { }` 直接拒绝）。显式空数组 `["modules": []]` = 「只有入口」。
- 段存在即**全量校验**：缺 `entry` / 缺 `modules` / 类型不符一律拒绝——不接受
  「写了个 `code` 但字段缺一半」这种半声明。

#### 名字与路径：点分名 ↔ 相对路径

- `src/util.lua` ↔ `require("src.util")`：模块名是点分标识符串，路径是包内相对
  路径加 `.lua`；两者是双射，`path_of` / `name_of_path` 是唯一实现
  （`crates/ygg-modules`），打包、装载、静态检查三处共用同一份判定。
- 每段字符集 `[A-Za-z0-9_]`，单段 ≤ 64 字节，整名 ≤ 256 字节。
- **文件名段不得带点**：`src/a.b.lua` 与 `src/a/b.lua` 都会映射到 `src.a.b`，
  缓存 key 不再唯一，同名不同体就成了「装载序决定谁生效」的不确定性缺陷。
  `name_of_path` 因此直接拒绝段内 `.`（`AmbiguousSegment`）；`..` / 绝对路径 /
  反斜杠 / 非 `.lua` 同批拒绝——装载期不做任何路径拼接，路径穿越面不存在。
- `require` 的入参必须是**规范名**：`require("src/util")` 是非法名
  （`ygg-lint --pack` 报 `YGG-MOD-NAME`，并给出正确写法）。

#### require 语义

- 模块 chunk 与入口共享同一 Lua 状态与 globals：模块顶层 `local` 不跨模块可见，
  **跨模块共享只经返回值**。
- 按规范名缓存：同一模块重复 `require` **只执行一次**，返回同一对象（table 身份
  一致）。模块 `return nil` 或无返回值 → 缓存 `true`，同样只执行一次。
- 循环依赖报**完整链条**：`circular require: src.a -> src.b -> src.a`；装载深度
  显式判栈（上限 64），不靠 Lua / Rust 栈溢出兜底。
- **`require` 在任何时机都可用**：装载期（入口顶层、模块体）与运行期（事件
  handler 内部）一视同仁。`host.seal()` 只封 `ygg.on`（运行期注册禁令），不顺手
  封 `require`——「注册必须在装载期」与「模块必须在装载期拉齐」是两件事，把后者
  当前者的代价是懒加载直接不成立。所以**纯库可以懒加载**（示例见「副作用边界」）：
  第一次用到时装载，之后跨事件命中同一份缓存。
- 代价是模块顶层必须是纯导出面（下一节）：懒加载放开的是**时机**，不是**约束**
  ——handler 里 `require` 到的模块照样不许在顶层注册。
- 模块装载消耗**独立燃料预算**（`LoadBudget`），不挤占单转换预算：模块再多，
  每次转换的可用步数不变。
- 读档重建缓存：模块内的 `local` / 计数器属「代码的加载产物」，不是状态——读档
  后每个模块重新执行一次、返回对象重建，**状态必须落 kv**（§1）。

#### 副作用边界：入口装配，模块纯导出

**不变量（一句话）**：模块 chunk 只做纯导出（定义函数与常量），不做任何有副作用的
事；注册 / 写状态 / 输出全部集中在入口 `mod.lua`。

**为什么**：`ygg.on` 的**调用序就是 handler 的派发序**。模块顶层若能自由注册，
「文件怎么拆」「先 `require` 谁」就直接决定「谁先被派发」——把一个大文件拆成三个
会改变运行时行为，重构不保行为。收紧之后派发序只有一个来源：**入口里那几行显式
调用的先后**（`M.install(deps)` / `M.register()` …），与 `require` 顺序、与
`pack.json` 的 `modules` 声明序都无关。

补一条容易踩的：**`M.register()` 也是注册面**。典型例子是 `agent.summary`
（只读摘要动作）——它在 render 模块里，但必须由入口在 `M.install(deps)` 之后显式调用
`render.register()` 完成注册，不能写在 `src/render.lua` 的模块顶层（`YGG-MOD-EFFECT`
会直接拒绝）。派发序上把它当作「入口的第一行 install」来排。

禁止面与允许面（清单唯一实现是 `crates/ygg-modules/src/lib.rs` 的
`pub mod effects`，内核影子表与 lint 都只读它，不各自维护一份）：

| 面 | 禁（模块 chunk 顶层碰即拒绝） | 允（模块顶层可用） | 清单常量 |
| --- | --- | --- | --- |
| 注册 | `ygg.on` / `ygg.fsm.define` / `ygg.narrative.node` | — | `BANNED_CALLS` / `BANNED_TABLE_MEMBERS` |
| 写状态 | `ygg.kv.set` / `ygg.entity.spawn` / `ygg.entity.set_attribute` / `ygg.entity.add_tag` / `ygg.entity.remove_tag` | — | `BANNED_TABLE_MEMBERS` |
| 输出 | `ygg.narrate` / `ygg.emit` / `ygg.choice` / `ygg.meta` / `ygg.narrative["goto"]` | — | `BANNED_CALLS` / `BANNED_TABLE_MEMBERS` |
| 纯读 | — | `ygg.data` / `ygg.ticks` / `ygg.kv.get` / `ygg.entity.has_tag` / `ygg.entity.get_attribute` | `PURE_READ_CALLS` / `PURE_READ_TABLE_MEMBERS` |

- 清单是**穷举式禁止面**，不是白名单：未列出的名字（含 `ygg.stdlib_version` /
  `ygg.engine_version` 这类值字段）在模块顶层照旧可用。`fsm` / `narrative` 是
  stdlib 在装载期用 Lua 挂出来的子表，按「子表 + 成员」处理，不当成 `ygg` 直系函数。
- **函数体内完全合法**：`function M.install(deps) ygg.on(…) end` 是这套边界的标准
  出口——禁的只是「模块 chunk 顶层」，不是「模块文件」。
- **入口 chunk 不受约束**：入口本来就是装配清单，注册 / 初始化是它的职责。单文件
  Mod（无 `code` 段）因此逐字节不变——它只有入口。
- **两道防线、一份清单**：运行时由 `ygg-sandbox/src/modules.rs` 的 `ShadowBanned`
  在模块 chunk 执行期间把禁止面原位换成拒绝桩（`Drop` 恢复，模块抛错也恢复，失败
  的模块不入缓存）；静态由 `ygg-lint --pack` 的 `YGG-MOD-EFFECT` 在作者侧提前报
  同一件事（顶层**取引用**也算违规，哪怕只是 `local f = ygg.on`）。lint 的已知局限
  ——别名逃逸（`local y = ygg; y.on(…)`）与动态下标静态不可判、顶层立刻执行的
  `pcall(function() … end)` 分不出——一律由运行时影子表兜底：那时别名拿到的也是桩。

模块（只导出）：

```lua
-- src/door.lua —— 模块 chunk：只定义，不注册
local TEXTS = ygg.data("ygg:scene/texts") -- 纯读面：顶层快照资源是允许的

local door = {}

function door.title()
  return TEXTS.scenes.begin
end

-- 注册面做成函数：函数体内的注册 / 写状态 / 输出都合法
function door.install(deps)
  ygg.narrative.node("node:door", function()
    ygg.narrate(door.title())
    ygg.choice(TEXTS.prompts.door, { TEXTS.options.enter, TEXTS.options.leave })
  end)
  ygg.on("ygg.choice_selected", deps.on_choice)
end

return door
```

入口（装配）：

```lua
-- mod.lua —— 入口：唯一的装配点（顶层注册是它的职责）
local door = require("src.door")
local choices = require("src.choices")

-- 派发序 = 下面两行的调用序（与上面 require 的顺序无关）：
door.install({ on_choice = choices.select })
choices.install()
```

handler 内懒加载纯库（这条能力是本次收紧一起拿回来的——模块纯了，`require` 才不必
和注册时机绑在一起）：

```lua
-- mod.lua：装载期不 require 这个库，第一次用到时才装载
ygg.on("agent.summary", function()
  local report = require("src.report") -- 运行期首次装载；之后跨事件命中缓存
  ygg.narrate(report.line())
end)
```

```lua
-- src/report.lua —— 照样是纯导出面（懒加载不放宽边界）
local report = {}

function report.line()
  return "需求 " .. tostring(ygg.kv.get("req.count"))
end

return report
```

#### 迁移：旧写法（模块顶层注册）怎么改

改造前模块可以在自己的顶层 `ygg.on(…)`，那时「`require` 序 = 注册序 = 派发序」。
现在这种模块**装载期就红**（不是静默改变派发序）——它必须改，改法只有一种形状：

```lua
-- 旧（改造前合法，现在拒绝）
ygg.on("probe.order", function()
  ygg.narrate("sneaky")
end)
return shared

-- 新（注册进函数体，入口显式调用）
function shared.install()
  ygg.on("probe.order", function()
    ygg.narrate("sneaky")
  end)
end
return shared
```

入口补一行按原注册序的调用（`shared.install()`）。**为什么值得改**：拆文件、挪
`require` 顺序、把函数搬到另一个模块，都不再改运行时行为；handler 里能懒加载纯库；
注册点收敛到入口一处，读入口即知派发序。

#### 错误示例（两种形态，照文案改）

静态检查（`ygg-lint --pack`，有 error 即退出码 1）：

```text
error[YGG-MOD-EFFECT]: 模块顶层调用了有副作用的 host API `ygg.on`：模块 chunk 在装载期执行，注册 / 写状态 / 输出会随 require 一起发生。注册序就是派发序：模块自己注册，等于让「文件怎么拆」决定「运行时行为」，拆文件或调整 require 顺序都会改派发序，重构不再安全
   ┌─ src/shared.lua:21:1
   │
21 │ ygg.on("probe.order", function() ygg.narrate("sneaky") end)
   │ ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ 模块顶层只留纯导出（函数定义与常量）：把这段搬进模块函数 `function M.install(deps) … ygg.on(…) … end`，由入口（pack.json 的 code.entry）require 后显式调用 `M.install(deps)`——函数体内调用完全合法。顶层仍允许纯读面（ygg.data / ygg.ticks / ygg.kv.get / ygg.entity.has_tag / ygg.entity.get_attribute）
```

（同一段建议还会以 `= fix:` 行重复一遍，此处略。）运行期由影子表拦下，装载即失败
（`ModLoad` 形态）：

```text
mod load failed: runtime error: src/shared.lua:21: 模块顶层不能调用 `ygg.on`
  模块顶层只做纯导出（定义函数与常量），不注册、不写状态、不产生输出；
  装配集中在入口 mod.lua：
    1) 模块里写 function M.install(deps) ... ygg.on(...) ... end（在函数体内调用是允许的）
    2) 入口 require 该模块并调用 M.install(deps)
  原因：handler 的注册序就是派发序。让模块自己注册，等于让「文件怎么拆」决定「运行时行为」，
  重构就不再安全。模块纯了以后，require 才能在 handler 里按需调用。
  见 docs/mod-authoring.md §5 多文件 Mod
```

- 拒绝覆盖**整条依赖链**：中间层模块（不是入口、也不是第一个被 `require` 的）写错
  一样装载期红，报错指向**模块自己**的文件与行号。
- 影子表 `Drop` 恢复：拒绝之后同一次运行里 `kv` / `narrate` 照常可用（桩不残留）；
  失败的模块不写缓存，再 `require` 得同一个可读拒绝，不会留下半个模块。

#### 内容寻址：改模块必须重跑 `pack`

声明了 `code` 段后，`pack` 工具逐模块读盘现算 sha256 / byteLength，写进
`manifest.json` 的 **`code` 数组**（与 `entries` 同构但独立，按路径序写），并用
装载期同一套判定（集合一致 / 上限 / 内容寻址）再校一遍——**打包过了但装载不过**
这条路被堵死了。

```json
"code": [ { "path": "src/util.lua", "sha256": "<64 位小写 hex>", "byteLength": 123 } ]
```

- 装载前对账三层：声明 ↔ manifest 登记（**双向相等**）→ 字节内容
  （byteLength 预检先于 sha256）→ 登记规范名。任一不符即装载失败，错误文案带
  模块路径与期望值。
- 改了模块文件**必须重跑 `pack`**：字节数或哈希对不上，装载期直接拒绝。
  **这是设计，不是 bug**——它堵的是「改了模块却静默跑旧代码」这条最贵的失误。
- 声明与 manifest 双向相等：声明了没登记（忘了打包）/ 登记了没声明（改了声明没重打）
  都拒绝——不接受「半份模块表」（半份表只会换来一个看似无关的 `NotFound`）。

#### 错误定位：真实文件 + 真实行号

- 模块 chunk 名 = `@<包内相对路径>`（`@src/util.lua`；入口仍是 `@mod.lua`）：
  错误文本与 traceback 携带**真实文件与行号**，不是拼回单文件之后的偏移。
- 装载器自判的拒绝（非法名 / 未登记 / 循环 / 超深 / seal 之后）带**调用点**：
  形如 `src/a.lua:3: <原因>`，与模块 chunk 错误的 `path:line:` 前缀同形；未登记时
  还带 require 链。
- 模块 chunk 自身的错误按 Lua 原生错误链浮出（每层 `path:line:` + traceback 即
  链条），不再套一层包装——深层依赖的错误再包一层只会让文本平方级膨胀。
- 整包检查：`cargo run -p ygg-lint -- --pack <mod目录>` 逐模块**独立 parse**
  （**不拼接**：Lua 的 `local` 本就不跨 chunk，装载器也是一个模块一个 chunk，
  拼接会制造装载期不存在的共享作用域），并按 AST 扫 `require("…")` 字面量建
  依赖图，循环与深度用装载器同一份 `LoadStack` 判。错误面：`YGG-MOD-MISSING`
  （声明了读不到）/ `YGG-MOD-NAME`（名字或路径非法）/ `YGG-MOD-UNDECLARED`
  （require 了没登记）/ `YGG-MOD-CYCLE`（循环，链条与装载期同形）/ `YGG-MOD-LIMIT`
  （数量 / 深度 / 字节超限）/ `YGG-MOD-EFFECT`（模块顶层碰副作用面，见「副作用
  边界」，error 拦门）/ `YGG-MOD-UNUSED`（登记了从入口不可达，warning 不拦门）。
  有 error 时退出码 1；没有 `code` 段的包给一句「该 Mod 为单文件」提示，退出码 0。

#### 上限（装载前判）

| 维度 | 上限 | 常量（`crates/ygg-modules` / `pack`） |
| --- | --- | --- |
| 模块数（不含入口） | 128 | `MAX_MODULES` |
| 装载深度（同时存活的嵌套 require 层） | 64 | `MAX_DEPTH` |
| 单模块源码 | 1 MiB | `MAX_CODE_BYTES` |
| 整包模块源码合计 | 1 MiB | `MAX_TOTAL_CODE_BYTES` |

#### 已知缺口：入口装载期（Mod 顶层）的 `ygg.narrate` 会被静默丢弃

`ygg.narrate` 是**收集-回放**面：Lua 执行期只把调用追加进收集缓冲，回放发生在
handler 调用内（每次 handler 调用前清空缓冲、调用后按追加序回放进 sink）。因此：

- **入口顶层**调用 `ygg.narrate`：调用进了缓冲，但第一个 handler 调用前的清空会把
  它丢掉——**不回放、无告警**。入口装载期的叙事输出无效。
- 模块体不在此列：模块顶层调 `ygg.narrate` 直接是**可读拒绝**（副作用面，见「副作用
  边界」一节），不会跌进「静默丢弃」这个更差的行为。
- 同族一条已有注记：`ygg.narrative["goto"]`（`stdlib/narrative.lua`：「装载期调用
  会进收集缓冲、不被回放」）。`ygg.emit` 与 `ygg.narrate` 走的是同一份缓冲，装载期
  调用同样不会派发 / 不会回放，此前没有注记——这里补上：**叙事与派生事件都只有在
  handler 内执行才有效**。
- 入口装载期该做的是注册（`ygg.on` / `ygg.narrative.node` / `ygg.fsm.define`）与快照
  （`ygg.data`）；模块顶层只做纯导出。`ygg.kv.*` / `ygg.entity.*` 是即席状态写，不经
  缓冲、入口装载期调用立即生效（所以 §1 的 spawn 门禁照样适用）。

#### KV 前缀归属（拆文件时先定这个）

一拆文件，原来「一个 chunk 里随便写」的 KV 平面就成了跨模块共享面。三条纪律：

1. **一个前缀只有一个写方**：跨模块写同一前缀 = 隐式耦合，拆文件的意义就没了。
   需要共享的派生值走模块返回值，或以只读函数导出。
2. **内核前缀只读**：`fsm.*` 由 stdlib 的 `ygg.fsm` 独占写（首触发写 initial，之后
   每次转换改写），Mod 侧不得 `kv.set("fsm.*")` 越权——那会和状态机直接打架。
3. **前缀不是对外 API**：它是同一 Mod 内部的约定；宿主不读 Mod 私有 KV（只读回执
   `meta.<metaKey>` 摘要面）。

现状前缀分布（以 agent-seed 为实例，全部为真实写入点）：

| 前缀 | 键形 | 写方 |
| --- | --- | --- |
| `fsm.*` | `fsm.<机器名>` | stdlib `ygg.fsm`（Mod 只读） |
| `settings.*` | `settings.<配置路径>` | `config.set` 动作（动态覆盖 > `data/config.json` 静态默认） |
| `req.*` | `req.<i>.<字段>` + `req.count` / `req.last` / `req.conflicts` | 需求表 |
| `prop.*` | `prop.<i>.<字段>` + `prop.count` | 提案表 |
| `lane.*` | `lane.<k>.<字段>` + `lane.seq` | 泳道台账（`lane.claim` / `lane.release`） |
| `ui.*` | `ui.page` / `ui.page.stage` | 纯展示层（不进摘要、不影响派活，也不改任何业务动作语义） |
| `reject.*` | `reject.count` / `reject.last` | 拒绝留痕（`flow.refuse` 的唯一写入点）：只做只读查询，**不是**纪律闸 |
| `task.*` / `verify.*` / `inbox.*`，裸键 `ending` | 各自单一用途 | 各自的单一写入点 |
| `review.*` | `review.lesson` / `review.score` / `review.evolution.{artifact,candidates,classes}` | `review.note`（单一写入点；进化声明三段缺省即 `""` / `0/0` / `""`） |

**这是文档约定，装载期不校验前缀归属**——没有新增校验面，别指望内核替你拦跨模块写。

进化声明是**两层**的（同一份口径、两个前缀）：`config.set review.evolution.artifact|candidates|classes=<值>`
写运行期覆盖层 `settings.review.evolution.*`（`settings.*` 的单一写方仍是 `config.set`），收口时
`review.note` 把它落到 `review.evolution.*`。种子只校**形状与完整性**——`ev` 的 `artifacts/*.md` 路径、
`c` 的两个非负整数、`cls` 的六类计数；`screened > 0` 时 `ev` 与 `cls` 都必填（`c=0/0` 可只带 `ev`），
且 `cls` 和必须等于 `screened`、保留类之和必须等于 `kept`；
沙箱没有文件系统（无 `io`/`os`），所以**读不到 artifact 内容**，也无法判断复核是否独立执行。

补一句关于「状态」的边界：KV 本就不进状态哈希（哈希只覆盖实体面），所以别把 `ui.*`
当成「可以随便写」的例外——它照样入存档，要响应存读档。

#### 示例 fixture

`crates/ygg-sandbox/tests/fixtures/multi-script/` 是一个**真跑**的三层依赖 Mod
（入口 → `src.handlers` → `src.util`），外加「三处 require 同一模块」与一个
`return nil` 的模块。它演示的正是本节的写法：每个模块只导出 `register()` /
`register_order()`，入口按 `util → shared → handlers` 的顺序显式调用它们——
**派发序 = 这几行的 call 序**，而 `pack.json` 的声明序与 `require` 序都故意不同
（改了 `require` 序，派发序不动）。它的 `README.md` 列了依赖形状、装载期观测面
与逐文件的内容寻址归属，被 `tests/modules_e2e.rs` /
`tests/modules_negative.rs` / `tests/modules_lifecycle.rs` / `tests/modules_pure_export.rs`
直接装载——**要照抄一个多文件 Mod 的结构，抄它**。

#### 热载与续局的签名口径

宿主监视的文件清单 = 五件套（`mod.lua` + `manifest.json` + `pack.json`）+ 配置的**实际
解析结果**（`pack.json` 未声明 `ygg:agent/config` 时回落 `data/config.json`）+ `entries` /
`schemaFiles` + **`code.entry` 与 `code.modules` 的每个路径**。签名本身是
`路径 + 字节数 + mtime`（只回答「变没变」，不回答「内容是否相同」）——所以续局不必为
每次读盘重算全量哈希；**内容精确性由 manifest 的 sha256 在装载时承担**。两者分工的
结果：模块文件一改，签名就变 → 热载触发重装载 → 内容寻址当场校验。缺了「纳管监视」
这半边，改 `src/*.lua` 而入口没动就会静默跑旧代码（无任何报错）——这是本设计最不能
省的一步。

监视的是「解析结果」而不是「声明」，原因同一条：只看声明的话，没声明配置项的 Mod
改 `data/config.json`（如 `drive.maxSteps`）签名不变，新阈值永不生效且无告警。

读盘 + 声明对账 + 构造驱动**只有一条路径**（`crates/ygg-sandbox/src/modload.rs` 的
`ModBundle::load`）：走查（`--example playtest`）与实跑（`--example agent`）吃同一份
装载语义，差异只来自事件驱动策略。`driver` / `sandbox` / `modules` 依旧零 IO，
装载器拿到的只是 `ModuleSet` 字节表。

### host 契约段（`agent` / `inboxRoutes`）

通用 Mod 工具（`cargo run --example agent`、`--example playtest`）**不含任何 Mod 语义字面量与
回退默认值**（事件名 / 叙事前缀 / 摘要键名 / 入口载荷 / inbox 字段名与幂等标记均属于契约）：
它们从被装载的 Mod 自己的 `pack.json` 读契约。这两段对这两个工具是**必填**（缺失即报错退出 /
`ok=false` 报告），写法见 `.ygg/mods/agent-seed/README.md` 的「零、host 契约」章节：

```json
{
  "agent": {
    "entryEvent": "<新局入口事件名>",
    "choiceEvent": "<玩家选择事件名；须与内核保留路由 ygg.choice_selected 同名，否则失去序号域校验>",
    "summaryEvent": "<Mod 自报摘要的只读动作名>",
    "endingPrefix": "<结局叙事行前缀>",
    "metaKey": "<回执信封 meta 里摘要的键名>",
    "rejectionPrefixes": ["<拒绝叙事行前缀>", "<至少一条>"],
    "replayPrefix": "<幂等重放叙事行前缀>",
    "defaultPayload": "<入口事件首次派发的载荷（JSON 标量文本）>"
  },
  "inboxRoutes": {
    "externalIdField": "<inbox 记录里幂等键的字段名>",
    "payloadField": "<inbox 记录里预成形载荷的字段名>",
    "extMarker": "<幂等键拼到载荷末尾的后缀标记>",
    "<kind>": {
      "action": "<内核动作名>",
      "fields": ["<段1>", "<段2>", "<幂等键字段名>"],
      "write": { "flag": "<宿主 CLI 旗标名>", "shape": "named|payload" }
    }
  }
}
```

- `inboxRoutes` 的三个保留键**必填**，但**允许一个 kind 都不声明**（= 本 Mod 不支持异步 inbox，
  宿主的 `--requirement` 等写命令会明确拒绝，退出码 3）。三个保留键与 kind **同层**，kind 名不得与它们同名。
- `agent.rejectionPrefixes` 必须写 **Mod 真实在用的**叙事前缀（≥1 条）：宿主靠它判定「派发未生效」，
  写错了就把被拒当成成功。`defaultPayload` 允许空串，但必须显式声明。
- `pack` 工具对这两段做**可选前置校验**：声明了就按运行期同一套规则解析，写坏了打包期即拦下；
  没声明不报错（冒险类 Mod 不吃 host 契约）。
- 只有吃契约的通用工具需要它；**绑定特定 Mod 的应用**（`clients/godot`、`packages/client-web`、
  `examples/adventure.rs`…）各自服务固定 Mod，字面量归属见 `guide.show harness` 的 `literal-ownership` 规则。

### host 契约段（`settlement` / `drive`，在 `data/config.json`）

宿主的结算口径与自动推进阈值同样**必填、无默认值**（缺段 / 缺字段 / 非正整数 = 装载期失败）：

```json
{
  "settlement": {
    "requireEvents": true,
    "requireNoFailedSteps": true,
    "requireEnding": true,
    "requireInboxConsumed": true
  },
  "drive": { "maxSteps": 64, "stallLimit": 12, "stepsPerRequirement": 16 }
}
```

- `settlement` 的四项各自可关（关掉不参与结算 `ok`，但仍记入 `settlement.json` 的 `checks`）。
- `drive` 三项必须是正整数（`maxSteps` / `stallLimit` / `stepsPerRequirement`）。
- 这两段属于 `agent-config` schema 的**可选属性**（内核不解释，只做形状校验）；缺段时内核装载照过，
  但 `--example agent` 会拒绝启动——口径来自 Mod，宿主不猜。
- 注意区分：这里的 `settlement` 是**宿主配置段**（`--settle` 的判定口径）。种子 Mod **不提供**
  `settlement.*` 动作族——那属于项目实例的自选面；种子的局末进化声明走 `review.note` 可选段
  （`ev=` / `c=` / `cls=`），不新增动作、不新增状态。

### 装载期会发生什么

资源在内核装载时按 manifest 逐条**内容寻址校验**（byteLength 预检 →
sha256 → JSON 解析 → schema 分发校验），任何一条不过即装载失败（错误
文案带文件路径与违规定位）。Mod 顶层即可 `ygg.data(id)` 快照资源——
装载期快照进 local，handler 闭包消费（资源只读，不受读档影响）。

## 6. 存读档语义：资源不入档，restore 须重供资源

- `save` 存档 = 引擎版本 + 实体表 + KV + 待决选择。**资源包不在存档内**
  ——资源是数据不是状态：读档不改资源、存档不携带资源。
- **读档（restore）必须重新供给资源三件套**（manifest + schemas 文本 +
  数据字节）：宿主以 `restore_with_resources` 重建时，先按装载管线校验
  资源包，再整体覆写状态面。宿主侧忘了重供资源是集成期常见错误——
  内核侧报的是「资源包缺失」而非「存档损坏」，别被文案带偏。
- 引擎版本门：存档由异版引擎写出时读档拒绝（VALIDATION，文案含双版本）。
- 存读档往返后：实体 id 稳定（kv 取回即同一实体）、KV 原样、待决选择
  原样挂起（读档回包 `pendingChoice` 直接可渲染提示面）。

## 7. ygg.choice_selected 域校验（A2 语义）

存在待决选择时，驱动在 `ygg.choice_selected` 进入内核转换**之前**做序号
域校验：payload 必须是整数且在 `1..=选项数` 内。

- 越界（如 2 个选项传 `9`）或非整数（`1.5` / `"2"` / 非 JSON）→
  可读 VALIDATION：`选项不存在：有效范围 1..2`——玩家视角文案，不进
  转换、不耗燃料、**不改任何状态**，待决选择原样保留，随后合法序号
  照常工作。
- 没有待决选择时收到该事件不触发域校验（无对象可校验），按普通事件
  进入转换——handler 内自行兜底（未知 stage 静默同 fsm no-op 口径）。

## 8. 排障备忘

- handler 报错文本已携带 **mod 侧位置**（chunk 命名 `@mod.lua`）：
  形如 `mod.lua:3: boom`，配合行号即可定位 Mod 源（Lua 会剥掉文件型
  chunk 名的 `@` 前缀，`@` 是记号不是名字本体）。
- **host 回调拒绝同样可定位**（迭代 D 实证收口）：如 `ygg.kv.set("x", 0/0)`
  被内核门拒绝时，mlua 在 Rust 回调返回错误的瞬间捕获当时 Lua 栈
  （`CallbackError.traceback`），错误文本形如：

  ```text
  lua handler for event `event.bad` failed: runtime error: kv value for key `x` is non-finite
  stack traceback:
  \t[C]: in field 'set'
  \tmod.lua:3: in function <mod.lua:2>
  ```

  消息首行（`runtime error: ...`）是桥侧 Rust 文案（描述拒什么），Mod 侧
  调用点在末尾 traceback 的 `mod.lua:<行号>` 帧里——两层合读即可定位。
  `ygg.entity.set_attribute` 等其余 host 回调同法。
- 运行期警告：choice 前置 emit 丢弃经信封 `warnings` 键浮出（见 §2，
  不再静默）。
- `ygg.narrative["goto"]` 跳未知节点 = 可读报错（不静默）；fsm 无匹配
  = 静默 no-op（见 §3）——两类「找不到」行为不对称是既定设计。
- 装载期（Mod chunk 执行期）报错以 ModLoad 形态浮出，含 Lua 文本定位；
  运行期 handler 报错映射 VALIDATION 码（稳定码契约：按 `error.code`
  分支，勿解析 message）。

## 9. 验收表（宿主 / Mod 迁移，两份）

多文件 Mod 能力分两段：**宿主面**（解析 / 装载 / 打包 / 热载 / 静态检查 + 契约测试）
已交付；**Mod 迁移**（把 agent-seed 这种大单文件 Mod 真的拆成 `src/*.lua`）是后续
独立迭代，**不在本次范围**。两张表分开是因为验收主体不同：前者验「机制成立」，
后者验「某一份 Mod 拆得对不对」。

### 9.1 宿主验收（已交付，本次范围）

| 交付面 | 验收判据 | 证据 |
| --- | --- | --- |
| 解析面（`crates/ygg-modules`）+ 装载器（`modules.rs`）+ 装载期燃料（`fuel.rs` 的 `LoadBudget`）+ 副作用影子表（`ShadowBanned`） | 名字规范化与路径双射（`name_of_path(path_of(n)) == n`）；缓存与 table 身份；`nil` / 无返回值 → `true` 且只执行一次；循环报完整链条；深度显式判栈；`require` 装载期与 handler 内部均可用；模块顶层碰禁止面即可读拒绝（含调用点与真实文件行号），影子表 `Drop` 恢复；模块装载消耗独立预算（不挤占单转换预算） | `cargo test -p ygg-sandbox --test modules_unit --test modules_load --test modules_pure_export` |
| `pack.json` 的 `code` 段 + `manifest.json` 的 `code` 数组 + 热载监视清单 | 声明全量校验（路径规则 / 形状 / 规模）；三方对账（声明 ↔ 登记 ↔ 字节）；内容寻址（byteLength 先于 sha256）；`watched_files` 纳管每个模块与**配置解析结果** | `cargo test -p ygg-sandbox --test pack_code`、`cargo test --example agent` |
| 唯一装载口 `modload.rs` + `ygg-lint --pack` | `load_mod` 全仓单点（走查与实跑同一份装载语义）；lint 逐模块独立 parse（不拼接）+ AST 依赖图 + 与装载器同一份 `LoadStack` + 副作用面（`YGG-MOD-EFFECT`）；错误面 MISSING / NAME / UNDECLARED / UNUSED / CYCLE / LIMIT / EFFECT，有 error 退出码 1 | `cargo test -p ygg-sandbox --test modload_bundle`、`cargo run -p ygg-lint -- --pack crates/ygg-sandbox/tests/fixtures/multi-script` |
| 测试面：fixture（三层依赖 + 重复 require + `return nil`）+ 正/负/生命周期 | 每条拒绝路径断言错误文本的**关键片段**（模块路径 / 行号 / 完整链条 / 上限常量），不用 `is_err()` 糊过去；读档重建缓存、模块内状态归零、handler 不重复注册；128 模块整包装载 < 2s；重复装载注册序逐项相等 | `cargo test -p ygg-sandbox --test modules_e2e --test modules_negative --test modules_lifecycle` |

单文件零迁移的证据面（每次合并都跑）：`.ygg/mods/agent-seed` 与 `packages/mods/agent-seed`
两侧 `playtest` 都到结局、【结局 叙事行出现、`ygg-lint` 0 error。

### 9.2 Mod 迁移验收（后续独立迭代，不在本次范围）

把 agent-seed 从单文件拆成 `src/*.lua` 时，按这张表验：

| 判据 | 口径 |
| --- | --- |
| 骨架面两侧逐字节一致 | `scripts/sync-seed-mod.sh` 后 `diff -q .ygg/mods/agent-seed/mod.lua packages/mods/agent-seed/mod.lua` 无输出；`schemas/` 同法（CI 有骨架守卫） |
| 整包静态检查干净 | `cargo run -p ygg-lint -- --pack <两侧目录>` 均 0 error 0 warning，且无 `YGG-MOD-UNUSED`（每个登记模块从入口可达） |
| 上限内 | 模块数 ≤ 128、装载深度 ≤ 64、模块源码合计 ≤ 1 MiB |
| **派发序不变** | handler 派发序与拆分前逐项一致——派发序**只来自入口**：模块纯导出，`ygg.on` 由入口按原注册序显式调用（`M.install(deps)` / `M.register()`）；`require` 序与 `pack.json` 的 `modules` 声明序都不参与定序 |
| 模块纯导出 | 每个模块 chunk 顶层只有函数 / 常量定义与纯读（`ygg.data` 等）；注册 / 写状态 / 输出全部在模块函数体内、由入口调用；`ygg-lint --pack` 不得出现 `YGG-MOD-EFFECT` |
| KV 前缀单一写方 | 按 §5「KV 前缀归属」逐前缀核对；**无自动校验**，只能人工过 |
| 存读档 | 读档后模块缓存重建（每个模块各执行一次）、模块内 `local` / 计数归零、handler 不重复注册 |
| 内容寻址 | 每次改模块后重跑 `pack`，`manifest.json` 的 `code` 数组随之变化（不重跑就装载失败） |
| 双 Mod 三件套 | 两侧 `pack` + `ygg-lint` + `playtest` 全绿（playtest 必须到【结局） |

两个不能省的说明：

- **拆分最容易漏的回归是派发序**，不是语法：派发序变了不会报错，只会行为不同。
  它的唯一来源是入口那几行调用——所以拆分时先照原注册序排好入口的 `install()` 调用，
  其余（模块里怎么写、`require` 谁先）就都不影响派发。
- `YGG-MOD-UNUSED` 是 **warning，不拦门**（退出码 0）：拆分后忘删的死模块不会让门禁变红，
  只能人看报告。
