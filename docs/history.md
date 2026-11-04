# 迭代记录（History）

> 只记「为什么这样落」与「留下了什么可判定的口子」；实现细节看代码与各 crate 头注。

## r-store · 宿主持久化端口下沉 + sqlite 可选后端

**分支** `feature/ygg-store`（base `38b7a88`）。目标：让宿主的第二个存储后端能接入，
而不必把「存档 / 事件日志 / inbox / 位移 / 重置」再复制一遍。

### 落成的四件事

1. **端口**：`crates/ygg-store`（第 7 个 workspace 成员，第二个**受控例外**——理由写在
   根 `Cargo.toml`，与 `ygg-modules` 同格式）。trait 只承接「状态事实」，
   约定调用方已持 run 锁，**trait 里没有任何锁原语**。
2. **JSON 后端**：从 `examples/agent.rs` 原样下沉，行为与文案不变。字节面用
   **合成探针 Mod** 冻结的 fixture 逐字节回归（`tests/fixtures/legacy-run/`）。
3. **sqlite 后端**：`<run-dir>/run.sqlite`，WAL + `synchronous=NORMAL`（与既有无 fsync
   的原子写在**耐久度上等价**，不是降低）+ `busy_timeout=5s`；`BEGIN IMMEDIATE` 单事务
   写 `step` + `snapshot`；reset 用 `generation` 归档（**旧代仍可查**，不删证据）。
4. **宿主接线**：`run.json` 的 `store` 字段是权威，`--store` 只是开局断言（不一致 →
   fail-closed 且零写入）；旧局无该字段 → 按产物推断为 `json`。

### 留下的可判定口子（下次可以据此复查）

| 口子 | 判定方式 |
| --- | --- |
| JSON 字节兼容（键序 / 尾随换行） | `cargo test -p ygg-store` 的 `golden_bytes`（逐字节，不做 JSON 语义比较） |
| 两后端语义等价 | `cargo test -p ygg-store` 的 `backends_agree_on_step_records`（`StepRecord` 逐字段相等） |
| sqlite 全或无 | `src/sqlite.rs` 的 crate 内单测（事务中途失败 / 写锁竞争两条注入） |
| 可选后端不污染 wasm/lint | CI `store-sqlite` job 的 `cargo tree` 守卫（出现 `rusqlite` 即红） |
| 不许开 `preserve_order` | 同上（`cargo tree -p ygg-store -e features` 含 `preserve_order` 即红） |

### 刻意没做的事（避免被误当成「漏了」）

- **不写 `autosave.json` 到 sqlite 局**：真加性 = 不产生多余文件。代价是 sqlite 局不能被
  只认文件的旧工具读——`--store json` 是显式选择，不是隐式回退。
- **不把 `run.json` / `settlement.json` / `coordination.*` 搬进 crate**：它们两后端完全一致，
  且 `coordination.lock` 同时守着 `decide-input.json`，搬走等于把跨进程互斥一起搬走。
- **不升 `synchronous=FULL`**、**不动内核 savepoint**：都属另一个需求的范围。
- **不回写 `events.jsonl` 到 sqlite 局**、不做双向迁移：混用一律 fail-closed（人工裁决）。

### 顺带修复

- `examples/agent.rs` 的两处 **master 遗留** clippy 告警（`field_reassign_with_default`、
  `missing_const_for_thread_local`）——非本轮引入，但本轮拥有该文件的改动面，
  `cargo clippy --workspace --all-targets -D warnings` 是验收命令之一，故一并修掉。
- `u4` 单测跟随函数下沉改为断言 `JsonFileStore::next_seq`（口径不变：seq = 非空行数）。
