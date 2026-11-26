---
type: "Reference"
title: "开发纪律 — 分支、门禁、零债与提交"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T07:10:30.723Z
sources:
  - id: openwiki-source-e05e3253cc972e8bd548b5cf
    resource: repo://.github/workflows/check.yml
  - id: openwiki-source-8037e2358a2c4f9b2c722a11
    resource: repo://AGENTS.md
  - id: openwiki-source-2696e2f7febd691f8cbc479e
    resource: repo://crates/ygg-host/tests/replay_tamper.rs
  - id: openwiki-source-73a02c1005c0a3609db5e3de
    resource: repo://crates/ygg-sandbox/src/host_api/world/mod.rs
  - id: openwiki-source-88bd460316cc9f4d5e371ae1
    resource: repo://scripts/wiki-consistency.sh
  - id: openwiki-source-219a851abf50e3acdd3f032c
    resource: repo://scripts/wiki-ver.sh
generated: { by: "pi", at: "2026-11-26T07:10:30.723Z" }
---

# 开发纪律 — 分支、门禁、零债与提交

> **id** `operating/dev-workflow` · **模块** 操作 · **触发** 开工、提交、发现缺陷、想「先放着以后做」
> **读不到代价** `silent-drift`（纪律松一格，三个月后就要花一周把「临时」清掉）

## 1. 分支与合并

| 规则 | 违规症状 | 检查 |
| --- | --- | --- |
| 功能分支在独立 worktree 里长（`~/tmp/worktrees/<repo>`），主检出保持干净 | 主检出被切分支、构建产物污染 | `git -C <主检出> status --short` 只有未跟踪的本地目录 |
| 一个迭代一条分支（如 `agent/kernel-sdk`），**不直接提交到 `develop`/`master`** | 主干历史里出现「临时」提交 | `git log --oneline origin/develop -5` 里没有本迭代的提交 |
| 合并到主干由人决定，且用 `--no-ff` 保留合并节点 | 主干看不出批次边界 | 合并提交存在且父有两个 |

## 2. 提交粒度

- 一个提交 = 一件说得清的事（「世界事务化」「删掉第二套实体模型」「参考包 + 金值」）；
- 提交里同时包含代码、测试与**受影响的契约页**——文档滞后一个提交就等于没写；
- 提交信息首行是「做了什么」，不是「改了哪些文件」；正文写**为什么**与**代价**（迁移数字、性能变化、被否掉的替代方案）；
- 一个提交必须能独立通过门禁；不通过就不许提交，不许「下一个提交修」。

## 3. 提交前门禁（与 CI 等价，除浏览器与 10 万 tick）

```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
cargo run -q --example selftest -p ygg-sandbox -- --mod packs/example
cargo run -q --example golden -p ygg-sandbox -- packs/example --check packs/example/golden.json
cargo run -q --example image -p ygg-host -- packs/example --out hosts/h5/pack.image.js --check
git ls-files '*.rs' '*.lua' | xargs wc -l | awk '$1>800 && $2!="total"'   # 单文件 ≤800 软、>1200 硬禁
bash scripts/wiki-ver.sh --check && bash scripts/wiki-consistency.sh   # 文档面（改过页面时）
```

改过 wasm 相关代码时再加两步：`bash scripts/build-wasm.sh`（需 `WASI_SDK_PATH`）与 `node scripts/wasm-lane.mjs`——CI 的 `wasm` job 也跑这两步，先在本地跑能省一轮来回。

`.rs` 单文件超 800 行是**信号**：先问「是不是混了两个职责」，再用门面拆分（`mod.rs` 收口 + 子模块各管一件事），不要按行数机械切分。超 1200 行是**硬禁**。

## 4. 零债

发现的问题只有两个合法去处：

1. **现在实现**（含测试与契约更新）；
2. **当场转成排期项**：写进台账，带验收标准与编号。

不合法：「以后再说」「待优化」「遗留」「TODO 注释」。台账项只有 `scheduled` / `done` 两态，且必须能被下一个人直接开工。

违规症状：代码里出现 `// TODO`、报告里出现「后续可优化」而没有编号。检查：`grep -rnE "TODO|FIXME|待优化" crates/ scripts/ packs/`。

## 5. 注释与测试

- **注释只写「为什么」与「不许碰的边界」**，不写「这行在做什么」；公开项用 `///` 说清契约与失败码；
- 测试名是契约名（`tampered_hash_is_located_at_its_frame`），失败时不用读实现就知道是哪条契约破了；
- **测试服从设计**：设计改了先改测试再改实现；不许为了绿化而放宽断言、删用例、加 `#[ignore]`；
- 断言失败信息要带上下文（哪个 tick、哪个 eid、期望 vs 实际）。

## 6. 手术权

核验或联调中发现的**个位数行级**缺陷（一行取值、一个路径、一处签名）可以直接修并提交，提交信息里注明「为什么是手术而非重构」。超过这个尺度就按正常迭代走。

## 7. 不写什么

- 不写「计划文档」当交付物：能落成代码或契约页的就落，落不了就进台账；
- 不写兼容层/转发页（死链比转发页更容易发现）；
- 不写「将来支持 X」的抽象：现在没有第二个实现者，抽象只是负债。

## 8. 维护责任

- **触发**：任何提交、任何「先放着」的念头、任何门禁红。
- **检查**：§3 四条命令 + `git log --oneline` 每条提交都能对上「一件事」。
