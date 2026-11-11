# fixture：multi-script（r4 测试面）

一个**真跑**的多文件 Mod（不是空壳），被 `tests/modules_e2e.rs` /
`tests/modules_negative.rs` / `tests/modules_lifecycle.rs` /
`tests/modules_pure_export.rs` 与 `examples/egg.rs` 的测试直接装载。

后者的用法最容易误导未来的读者：`tests/modules_pure_export.rs` 把它复制到临时
目录后**只改入口**（调换 require 序 / install 序），断言「模块体执行序变了、
handler 派发序不变」。所以入口里那几行 `M.register()` 的**写法与顺序**是测试
契约的一部分：重排它们会同时弄坏本 fixture 与那份用例。

## 依赖形状

```text
mod.lua（入口，不是模块）
├─ require("src.handlers")   ──┬─ require("src.util")     ← 叶子，返回 table
│                              └─ require("src.shared")   ← 叶子，返回 table
├─ require("src.util")        （与 handlers 里那次同一份）
├─ require("src.shared")      （与 handlers 里那次同一份：三处 require 一个模块）
├─ require("src.silent")      ← 无返回值 → 缓存 true
└─ require("src.silent")      （重复 require 不得再执行）
```

三层依赖（入口 → `src.handlers` → `src.util`）+ 一处重复 require + 一个返回
`nil` 的模块——正是 r4 要钉住的那几个面。

## 装载期观测面（测试断言的对象）

| 全局名 | 由谁写 | 语义 |
| --- | --- | --- |
| `load_trace` | 每个模块体首行 | 模块体执行序（`>handlers>util>shared>silent`） |
| `util_execs` / `shared_runs` / `silent_runs` | 各模块体 | 执行次数（重复装载 / 重复 require 的显形面） |
| `util.count` / `shared.count` | `util.bump` / `shared.bump` | 模块内状态（存读档后必须归零：不留旧对象） |

事件面：`probe.*` 是给测试的观测面（`trace` / `identity` / `nil` / `table` /
`bump` / `stats` / `order`）；`event.start` + `ygg.choice_selected` 是给
playtest 的推进面（走到【结局）。

`pack.json` 的 `code.modules` 声明序（util → handlers → silent → shared）
**故意不等于** require 序——装载序由依赖图决定，不由声明序决定。

## 文件

| 文件 | 是否内容寻址 | 说明 |
| --- | --- | --- |
| `pack.json` | 否（声明面） | `entries` / `schemaFiles` / `code` / `agent` / `inboxRoutes` |
| `manifest.json` | 否（清单本身） | `entries` 与 `code` 的 sha256 + byteLength |
| `mod.lua` | 否（入口） | 宿主作为 Mod 源读取，不登记为模块 |
| `src/*.lua` | 是（`code`） | 可 `require` 的模块 |
| `data/scenes.json` | 是（`entries`） | 资源 |
| `schemas/scene-texts.json` | 否（按 `$id` 注册） | 资源 schema |
| `data/config.json` | 否（宿主侧） | `drive` / `settlement`（agent 宿主直读，非内核资源） |

改任何一个模块文件后必须重算 `manifest.json`（测试侧由
`tests/support/mod.rs::reseal` 现算；手工改要跑打包工具）。
