# scripts/

仓库自带的**开发辅助脚本**（非内核、非包，不进装载管线）。全部只读或只产出构建产物；任何一个都不做「决策」。

| 脚本 | 作用 | 触发的门禁 |
| --- | --- | --- |
| `build-wasm.sh` | 构建 `crates/ygg-wasm/pkg/`（WASI SDK 探测 + wasm-bindgen + WASI 导入改写 + 可选 `wasm-opt`），末尾跑 node 冒烟 | 改 `crates/ygg-wasm`、`crates/ygg-host`、`crates/ygg-capi` 后 |
| `wasm-lane.mjs` | 无浏览器环境跑 50 tick + replay，逐帧对比 `packs/example/golden.json` | 同上；CI 不跑（需 wasm 产物） |
| `persist-matrix.sh` | 长跑矩阵：1000 / 10000 / 100000 tick，原生与 wasm 最终哈希对比；**退出码 2 = wasm 产物过期** | 改确定性面、改快照格式后 |
| `wiki-ver.sh` | `openwiki/ver.json` 账本生成/验真 + 链接与 id 闭包 | 改任一 `openwiki/**/*.md` 后 |
| `wiki-consistency.sh` | 索引闭包 + `AGENTS.md` 行数预算 + claims 证据闭包（`--attest` 核验某局的读签） | 改 `AGENTS.md`、改被 claims 引用的文件后 |

用法与输出口径看各脚本头部的注释；门禁清单见 `openwiki/operating/build.md`。
