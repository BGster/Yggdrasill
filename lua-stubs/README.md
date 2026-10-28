# lua-stubs —— `ygg` 全局 Lua 类型 stub（LuaLS 注解面）

迭代 C2 产物：让独立 Lua 工具链（lua-language-server / IDE）认识 Yggdrasill
Mod 的 `ygg` 全局 API——消除 `undefined-global` 告警、提供补全与形参提示
（含 `ygg.narrative["goto"]` 保留字括号形态）。

## 文件

| 文件 | 说明 |
| --- | --- |
| `ygg.lua` | `---@class` / `---@field` 注解声明（`---@meta` 纯类型文件，不参与运行时） |

## 接入 LuaLS

Mod 工程根放 `.luarc.json`（仓库根已可直用）：

```json
{
  "workspace.library": { "lua-stubs": true },
  "runtime.version": "Lua 5.4"
}
```

独立 Mod 目录使用：把 `lua-stubs/` 目录拷入工程后同法引用 `workspace.library`。

## 覆盖范围

以内核源码为权威（host_api.rs / resources.rs / inject.rs / sandbox.rs /
stdlib/fsm.lua / stdlib/narrative.lua），M0 精确集全量声明：

- `ygg.on / emit / narrate / choice`（choice 终结语义见 stub 内注记）；
- `ygg.kv.get / set`；`ygg.entity.spawn / get_attribute / set_attribute /
  has_tag / add_tag / remove_tag`；
- `ygg.data(id)`（单函数形态，与内核实际 API 一致）；
- `ygg.fsm.define`（含 B1 `when` 条件边）；
- `ygg.narrative.node / ygg.narrative["goto"]`（括号形态已注解）；
- `ygg.engine_version` / `ygg.stdlib_version`（均为字符串字段，两值同源相等）、
  `ygg.ticks()`。

**值域**：内核 `core::Value` 四标量 `integer | number | boolean | string`
（`YggScalar` 别名）；table / function 不能作为 KV、属性或事件 payload。
