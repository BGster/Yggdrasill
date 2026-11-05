# 迭代记录（History）

> 只记「为什么这样落」与「留下了什么可判定的口子」；实现细节看代码与各 crate 头注。

## r-evo · 新局默认 sqlite（旧局不动）

**分支** `feature/default-sqlite`（base `9446858`）。目标：让「强崩溃承诺」成为默认
（`cargo build --example agent` 不带旗标即得 sqlite），同时保证既有 JSON 局的产物、
事件日志与错误回执一字不变。

### 落成的三件事

1. **默认 feature**：`crates/ygg-sandbox/Cargo.toml` 增 `default = ["store-sqlite"]`。
   `ygg-store` **仍在 dev-dependencies**——`default` 只是「本包 feature 转发到依赖
   feature」，对无 dev 图的构建（ygg-wasm / ygg-lint）是空指向，不拉 `rusqlite`。
   回退构建：`--no-default-features`。
2. **默认后端**：`ygg_store::DEFAULT_BACKEND = Backend::Sqlite`，`resolve_with` 的最后
   一行用它（`probed.unwrap_or(DEFAULT_BACKEND)`）。`resolve_with` 真值表里只有
   「无 `store` 事实 + 无任何产物」这一行会变；有 JSON 产物的一律仍落 `json`。
3. **单一常量来源**：`SQLITE_FILE_NAME` 是唯一字面量，`sqlite::SQLITE_FILE` 由它派生——
   探测与建库漂移 = 静默混代，这条不再靠人记。

### 留下的可判定口子（下次可以据此复查）

| 口子 | 判定方式 |
| --- | --- |
| 新局确实落 sqlite、且不产 JSON 文件 | `r10a_fresh_run_defaults_to_sqlite`（`cargo test --example agent -p ygg-sandbox`） |
| `--store json` 仍是回退通道 | `r10b_json_flag_new_run_writes_json_files` |
| 旧 JSON 局续局 / `--settle` 绝不建库 | `r10c_legacy_json_run_never_gets_sqlite_db`、`r10d_settle_on_legacy_json_run_stays_json` |
| `run.json` 事实锁住后端 | `r10e_new_run_then_declared_fact_locks_backend` |
| 回退构建 fail-closed（不静默降级到 json） | `r10f_default_sqlite_without_feature_fails_closed`（需 `--no-default-features` 构建；CI 有同名步骤） |
| 既有 JSON 断言不被默认值污染 | `probe_args` / `args_for` 钉 `store: Some(Json)`；默认后端只由 `r10*` 覆盖 |
| 默认值退化会红 | `empty_dir_defaults_to_sqlite`（`cargo test -p ygg-store`）断言字面量 `Backend::Sqlite` |
| store/rusqlite 不进 wasm/lint | CI `Guard — wasm/lint 不得间接依赖 store/sqlite`（`cargo tree -e normal` + `ygg-store v`） |
| 错误回执不指向不存在的文件 | `single_shot_error` 与 `resolve` 同序（`run.json` > `--store` > 产物探针 > 默认） |

### 刻意没做的事（避免被误当成「漏了」）

- **不给 `ygg-store` 设 `default = ["sqlite"]`**：那样 `cargo test -p ygg-store` 会顺手
  覆盖 sqlite 半边，但必须把 `ygg-sandbox` 的 dev-dep 改成 `default-features = false`
  才能保住回退通道——一个容易被后人删掉的反向依赖。本轮只做最小面。
- **不改 `run_settle` / `run_inbox_append` 的打开顺序**（端口早于 `enter_closing` /
  `begin_write`）：**接受**「在空目录上 `--settle` 会先试开库并失败」这一副作用——
  `SqliteStore::open` 不建父目录，失败即报错，不会静默建局；挪顺序属另一个需求。
- **不动 `detect()` 的旧局证据面**（仍只看 `autosave.json` / `events.jsonl` / `run.sqlite`）：
  `.ygg/runs/` 现有 15 个局全部命中「有 JSON 产物」或显式 `store` 事实，无需扩证据。

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
