---
type: "Reference"
title: "包模型 — pack.json、manifest 与装载序"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T07:10:30.723Z
sources:
  - id: openwiki-source-c0d5856e80be2560c5caa5c8
    resource: repo://crates/ygg-host/src/image.rs
  - id: openwiki-source-2525bb44114bad58534d56ee
    resource: repo://crates/ygg-host/src/proto.rs
  - id: openwiki-source-1816b4326e35a15b7cd252ff
    resource: repo://crates/ygg-sandbox/examples/pack.rs
  - id: openwiki-source-f4f3c3407ada59508fa211cb
    resource: repo://crates/ygg-sandbox/examples/selftest.rs
  - id: openwiki-source-3ce02e4845420a1c74760ca3
    resource: repo://crates/ygg-sandbox/src/modload.rs
  - id: openwiki-source-49f141683426787b179190a3
    resource: repo://crates/ygg-sandbox/src/modules.rs
  - id: openwiki-source-6556f80f4ad5bd14105c33c6
    resource: repo://crates/ygg-sandbox/src/pack.rs
  - id: openwiki-source-2c5400885f18104f706a3405
    resource: repo://packs/example/pack.json
  - id: openwiki-source-d0bfaf08595e0b573250de8c
    resource: repo://packs/example/test/turn.lua
generated: { by: "pi", at: "2026-11-26T07:10:30.723Z" }
---

# 包模型 — pack.json、manifest 与装载序

> **id** `contracts/kernel/packaging` · **模块** kernel · **触发** 新建/改包、加模块或数据文件、排查「改了一行 Lua 就装不起来」
> **读不到代价** `hard-refusal`（漏跑 manifest 重生成 → 装载直接 `E_PACK`，且报错指向字节数而不是「你忘了重算哈希」）

## 1. 两份文件、两个作者

| 文件 | 谁写 | 作用 |
| --- | --- | --- |
| `pack.json` | **人** | 声明面：这个包是什么、有哪些代码与数据 |
| `manifest.json` | **工具** | 锁面：每个文件的字节数与 sha256；装载期逐条核对 |

`manifest.json` 永远不要手改——它是 `cargo run -p ygg-sandbox --example pack -- <包目录>` 的产物。

## 2. `pack.json` v2 字段

```json
{
  "schemaVersion": 2,
  "id": "example",
  "version": "0.1.0",
  "capabilities": ["world"],
  "entry": "src/main.lua",
  "modules": ["src/ai.lua", "src/board.lua", "…"],
  "data": [{ "id": "ygg:example/board", "kind": "board", "schema": "https://…/board.json", "path": "data/board.json" }],
  "schemaFiles": ["schemas/board.json"]
}
```

| 字段 | 语义 |
| --- | --- |
| `schemaVersion` | `2` = v2 包（有 `entry`/`modules`/`capabilities`）；`1` 是旧单文件形态 |
| `id` | 包身份；宿主握手时与 `pack_id` 对账 |
| `version` | 包自己的语义版本，宿主可展示，内核不解释 |
| `capabilities` | 声明这个包要用哪些内核能力（今日只做声明与 schema 校验，**尚无装载期拒绝**，见 §7） |
| `entry` | 入口 chunk 路径（相对包根） |
| `modules` | 可 `require` 的其余模块（不含 entry） |
| `data[]` | 数据条目元数据；`id/kind/schema/path` 由人写，哈希由工具补 |
| `schemaFiles` | 随包分发的 JSON Schema 清单；**不进 manifest**，装载方按 id 注册 |

## 3. `manifest.json` 线格式

```json
{ "schemaVersion": 1,
  "code":   [ { "path": "src/ai.lua", "byteLength": 3634, "sha256": "39b4…" } ],
  "entries":[ { "id": "ygg:example/board", "kind": "board", "schema": "https://…", "path": "data/board.json", "byteLength": 272, "sha256": "ab4e…" } ] }
```

- `code` 段存在 ⇔ 包声明了 `entry`/`modules`（单文件包没有这一段，旧包因此零 diff）；
- 装载期核对三件事：**集合一致**（声明与锁一一对应，多一个少一个都拒）、**字节数**、**sha256**；任一不符答 `E_PACK`；
- 规模上限由 `pack::assemble` 把住：总量 1 MiB、模块数 128、内容寻址按规范化路径（`src/a` 与 `src/a.lua` 等价，两种拼法同时出现判重复）；
- `pack_hash`（宿主握手用的那个）是 **manifest 文本** 的 sha256，不是包的版本号——manifest 变了，指纹就变。

## 4. 装载序（与 `crates/ygg-sandbox/src/modload.rs::build` 分支一一对应）

| 步 | 做什么 | 失败后果 |
| --- | --- | --- |
| ① | 资源表 `ResourceTable::build`（顺带解析 manifest）——**先于任何 Lua 状态** | 预算/寻址/校验失败零 Lua 副作用，答 `E_PACK` |
| ② | `Sandbox::new()`：stdlib 裁剪 + 内存上限 + 白名单环境 | 内存配额失败 |
| ③ | `set_sim(Sim::new(seed))`：世界出生即存在，`world.*` 没有「无世界」降级态 | — |
| ④ | `inject::install`：`math` 精确集 + `math.random` 绑世界流 | 必须在 ③ 之后（闭包拿的是有世界的槽） |
| ⑤ | `host_api::install`：`ygg.kv` / `ygg.codec` / `world.*` | — |
| ⑥ | `resources::install`：`ygg.data` 只读面 | 条目 id 未注册即在这里暴露 |
| ⑦ | `stdlib::install`：版本面（Mod 顶层可读） | — |
| ⑧ | 模块表注入（模块集 + chunk 覆盖表，同一时机） | 首次 `require` 的 traceback 起就是真路径 |
| ⑨ | 入口 chunk：`reset_budget()` 后 `load(..).call(())` | 返回值就是声明面，命令面在这里找；异常答 `E_PACK` |

**预算重置在 ⑨ 之前**：装载是第一个宿主入口，②–⑧ 期间跑的库代码已经花掉余额，入口必须从满额起算才可复现。

## 5. 包内 `require`

- 只查宿主注入的 `ModuleSet`，两个命名空间：`src.*`（包自己的 `modules`）与 `pkg.<id>.*`（写在 pack 依赖里的共享文件）；
- **不做路径拼接**、不读文件系统：`require("src.board")` 命中 `src/board.lua`，`require("board")` 不会猜；
- 模块 chunk 执行期间 `ygg` 面被遮罩（`ShadowBanned`）：模块顶层**不许写世界状态**，只许返回导出表；
- 未注册的名字一律拒绝（空模块表 → 任何 `require` 都被拒，不静默给 `nil`）。

## 6. selftest 约定

- v2 包：用例放 `<包>/test/*.lua`，与 `harness.lua` 同住；入口由 `pack.json` 的 `entry` 决定（v1 是 `tests/` + `mod.lua`）；
- 运行：`cargo run -q --example selftest -p ygg-sandbox -- --mod packs/example [--filter <子串>]`；
- 自检面是纯 Lua：没有断言库依赖，`harness.lua` 注入 `fresh/copy/check/eq/eq_list` 等；
- 参考包实测 12 例（`packs/example/test/turn.lua`）。

## 7. 已知缺口（已排期，不在本页解决）

`capabilities` 今日只是声明：没有任何装载期或调用期的 `E_CAPABILITY` 拒绝，越界用面只能靠规则自律。台账 `D-13` 跟踪。

## 8. 维护责任

- **触发**：加/删模块与数据文件（必须重跑 `example pack`）、改 `pack.json` 字段、改装载序、改 manifest 线格式。
- **检查**：`cargo run -p ygg-sandbox --example pack -- packs/example`（应零 diff）、`cargo run -q --example image -p ygg-host -- packs/example --check hosts/h5/pack.image.js`、`cargo test -p ygg-sandbox --test pack_code --test example_pack`。
