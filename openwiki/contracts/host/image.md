---
type: "Reference"
title: "包镜像 — 无文件系统宿主的装载面"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T07:10:30.723Z
sources:
  - id: openwiki-source-a07e566f3712cbd5931877e8
    resource: repo://crates/ygg-host/examples/image.rs
  - id: openwiki-source-c0d5856e80be2560c5caa5c8
    resource: repo://crates/ygg-host/src/image.rs
  - id: openwiki-source-3ce02e4845420a1c74760ca3
    resource: repo://crates/ygg-sandbox/src/modload.rs
  - id: openwiki-source-8e4bf269973ae240f415c5ef
    resource: repo://hosts/h5/pack.image.js
generated: { by: "pi", at: "2026-11-26T07:10:30.723Z" }
---

# 包镜像 — 无文件系统宿主的装载面

> **id** `contracts/host/image` · **模块** host · **触发** 给浏览器/C 宿主打包、改 `pack.json` 的 `modules`、改装载路径
> **读不到代价** `hard-refusal`（镜像与目录两条装载路径分叉 → 同一包在原生能跑、在宿主里报 `E_PACK`）

## 1. 为什么需要镜像

wasm 与 C 侧没有可读的「目录」，装载参数只能随字节一起过界。镜像就是这段 JSON：

```text
{"modSource": "<Lua 源码>",
 "manifest": "<manifest.json 文本；空串 = 无资源>",
 "schemas":  ["<schema 文本>", ...],
 "resources": {"<path>": "<base64>", ...},
 "modules":  {"<模块路径>": "<Lua 源码>", ...},
 "chunkPaths": {"<模块路径>": "<包内相对路径>", ...}}
```

镜像解析放在**内核侧**（`ygg-host::image`）而不是绑定层：解析里每一步都是「这个字段认不认」的判断，而绑定层的规矩是只搬字节。三条绑定共用这一份实现，才有 parity 可言。

## 2. 六个键的语义

| 键 | 缺省 | 语义 |
| --- | --- | --- |
| `modSource` | 必填 | 入口 chunk 的 Lua 源码（等价于 `pack.json` 的 `entry`） |
| `manifest` | 必填（可为空串） | 空串 = 走无资源路径，此时 `schemas` / `resources` **被忽略** |
| `schemas` | 可缺省 | 每个元素是 schema 文本；注册 id 取自文本里的 `$id`，注册序 = 数组序 |
| `resources` | 可缺省 | `path → base64`；按 path 字典序装载（与书写序无关） |
| `modules` | 可缺省 | `require` 的**全部**可见面；键是模块路径 |
| `chunkPaths` | 可缺省 | 只喂 traceback 诊断，不进内容寻址 |

`modules` 的键两种拼法等價：`src/util` 与 `src/util.lua` 指同一个模块，规范名只有一个（`pack.json.modules` 写后者），两种拼法撞到同一个模块即拒。后两个键缺省与空对象等价 —— 单文件包的装载参数与没有镜像功能时逐字节相同。

## 3. 内容寻址与握手闸

镜像不碰磁盘：`ModBundle` 在内存里成型后直接 `Loaded::from_bundle`。因此 `pack_hash` 是 **manifest 文本本身的摘要** —— 同一份镜像在任何宿主、任何平台上都得到同一串。这正是握手闸要的「A 包的存档读不进 B 包」：比对的是文本摘要，不是文件系统状态。

## 4. 往返一致性

`to_image(load(image))` 与 `to_image(load(to_image(load(image))))` 必须逐字节相同：

- 输出规范化：`serde_json::Map` 按键字典序，不依赖遍历顺序；
- `manifest` 等于 `MINIMAL_MANIFEST` 且无 `schemas` / `resources` 时写出空串，与 `decode_image` 的无资源路径一一对应；
- `modules` / `chunkPaths` 仅在非空时写出，键由 `path_of` 还原为模块相对路径（`src.util` → `src/util.lua`）。

这条不变量由 `crates/ygg-host/src/image.rs` 末尾的内联用例（往返、模块键归一、base64、缺字段报错）与 `hosts/h5/pack.image.js` 的 `--check` 一起钉住：生成物一旦过期，`--check` 非 0 退出。

## 5. 缺字段与坏字段的报错

镜像里没有「猜测填充」，每个字段缺失或类型不对都返回一条指名原因（`crates/ygg-host/src/image.rs`）：

| 症状 | 报错前缀 |
| --- | --- |
| 字节不是 UTF-8 / 不是合法 JSON | `镜像不是合法 UTF-8：` / `镜像不是合法 JSON：` |
| `modules` 的键不是模块路径 | `镜像的模块键 "…" 不是模块路径：` |
| `modules[...]` 不是字符串 | `镜像的 \`modules["…"]\` 不是字符串` |
| 两处键撞到同一个模块 | `镜像的 \`chunkPaths\` ...`（登记期判重） |
| `schemas[i]` 缺 `$id` | `镜像的 \`schemas[i]\` 缺字符串 \`$id\`（注册 id 取自 \`$id\`）` |
| `resources[path]` 不是 base64 | `镜像的 \`resources["…"]\` 不是 base64 字符串` |
| 装载期失败 | `装载失败：` + 内核 `LoadError` 原文 |

## 6. 生成与校验

```bash
cargo run -q --example image -p ygg-host -- packs/example --out hosts/h5/pack.image.js   # 生成
cargo run -q --example image -p ygg-host -- packs/example --check --out hosts/h5/pack.image.js  # 校验
```

`--out` 以 `.js` 结尾时写出一个可被 ES module 直接 import 的常量文件（首行是「生成物勿手改」）；否则写纯 JSON。`--check` 必须与 `--out` 联用。

## 7. 维护责任

- **触发**：改 `pack.json`（尤其 `modules` / `resources`）、改装载序、加镜像字段。
- **检查**：`cargo test -p ygg-host image`（内联用例）、`--example image ... --check`、`node scripts/wasm-lane.mjs`。
