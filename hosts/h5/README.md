# `hosts/h5` — 无构建步骤的第三方画面

一个**没有任何构建工具**的浏览器宿主：原生 ES module 直接 `import`，无打包器、无 TypeScript、
无 npm 依赖、无外链字体/CDN。它只做三件事——把 `ready` 的组件元数据摊成表、把 `delta.changed`
涂到格子、把点击/按键翻译成 `command` 帧；**一行规则逻辑都没有**（可达格、伤害、胜负全在内核的
Lua 里，被拒绝时原样显示内核回的 `E_RULE` 文案）。

## 起服务（一条命令，在仓库根执行）

```bash
python3 -m http.server 8080
# 打开 http://localhost:8080/hosts/h5/
```

前提：先构建 wasm 产物（`crates/ygg-wasm/pkg/` 已被 gitignore，不入库）：

```bash
./scripts/build-wasm.sh      # 末尾会自动跑 node scripts/wasm-lane.mjs --smoke
```

页面 import 的是仓库里**已构建**的两份文件，零复制、零别名：
`../../crates/ygg-wasm/pkg/ygg_wasm.js` 与 `../../crates/ygg-wasm/wasi-shim.js`。

## 四个交互步骤

1. **看启动态**：页面加载后状态栏为 `就绪：tick 0 …`，画布上是 10×5 棋盘与 10 个单位
   （蓝=team 0 在左、红=team 1 在右），右侧表格列出 `ready.components` 的四列
   （`pos / hp / team / moved`）与每个实体的当前值。
2. **走一步**：点棋盘上的空格 → 发 `command{"name":"move","args":{"id":…,"dx":…,"dy":…}}`
   → 回执 `delta` 就地更新画面 → 再发 `step{ticks:1,dt:16}` 推进一 tick。
   也可用方向键 / WASD 对当前选中单位走一格。**点墙或越界**时状态栏显示内核回的
   `[E_RULE] …` 文案，`tick` 与画面都不变（内核拒绝即零写入）。
3. **打一下**：点另一个单位 → 发 `command{"name":"attack","args":{"attacker":…,"target":…}}`，
   血量变化完全来自 `delta`（不整表重绘）。同队或不相邻同样由内核回 `E_RULE`。
4. **存档续跑**：点 `Snapshot` 下载 `snapshot.bin`；刷新页面后用 `Open snapshot` 选中该文件 →
   发 `open{bytes}` → 从同一 `tick`/`hash` 继续。`End turn` 发 `end_turn`，`Step` 单独推进 tick。

选中规则：默认选中第一个实体；点表格某行选中它；点棋盘上另一单位 = 对它 `attack`；
按住 `Shift` 点另一单位 = 只切换选中（不发命令）。

## `?trace=1` 看什么

```bash
http://localhost:8080/hosts/h5/?trace=1
```

页面底部出现 Trace 面板，并把每帧的 `delta.changed` 同时打到 console：条目形如
`[step] tick=3 changed=10` 后跟逐实体 `{eid, comps:[{name, val}]}`。用它核对
「`delta` 只含被写的组件」——例如单条 `move` 命令只写 1 个实体的 `pos/moved`，
而 `step` 一 tick 会写全部 10 个实体的 `pos/moved`，不含 `hp/team`。

## 文件

| 文件 | 职责 |
| --- | --- |
| `index.html` | 画布 + 状态栏 + 按钮，仅此而已 |
| `main.js` | 协议循环：握手 → 视图状态 → 点击/按键 → `command` / `step` / `snapshot` / `open` |
| `render.js` | 纯像素：格子底色与单位方块、组件表；不读规则、不做判断 |
| `pack.image.js` | **生成物**：`export const IMAGE = {...}`，由下面这条命令产出，勿手改 |

```bash
cargo run -q --example image -p ygg-host -- packs/example --out hosts/h5/pack.image.js
cargo run -q --example image -p ygg-host -- packs/example --check hosts/h5/pack.image.js   # 校验
```

`hosts/h5/*.js` 里出现的 `hp / attack / move / team` 只可能是三类：命令名（`"move"` / `"attack"`）、
协议字段名（组件 `hp/team`、`delta` 的列）、状态栏文案——没有第四类。
