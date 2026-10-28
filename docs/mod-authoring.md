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

### 生成 manifest

```bash
cargo run -p ygg-sandbox --example pack -- packages/mods/<你的mod>
```

工具行为：对每个 `entries[].path` 从磁盘实际字节现算 sha256（小写 hex）
与 byteLength，生成 / 覆写 `manifest.json`；`schemaFiles` 逐个校验存在性
（漏带 schema 文件是装载期 `UnknownSchema` 的常见根因，打包期提前拦截）。
可读报错覆盖：pack.json 缺失 / 非法 JSON / 字段缺失、id 或 path 重复
声明、path 越界（`..` 或绝对路径）、数据文件缺失。

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
