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

## r2 · Mod 主动子系统（响应式折叠 + 真值探针）

**分支** `feature/mod-active-subsystem`（base `324522a`）。目标：让 `tool.result` 不只是「一条回执」，
而是把 6 个内建工具（`git.worktree_claim` / `git.worktree_release` / `git.lane_probe` /
`fs.artifact_probe` / `gate.verify_suite` / `harness.archive_sessions`）的输出**折叠进领域状态**，
并把「探针失败 / 机证 / 冲突雷达」三条边界写成可判定的规则。

### 为什么这样落

1. **独立 effect，而不是塞进 `receipt_effect`**：`tools.receipt` 的既有纪律是「只置闩锁 + 一行叙事，
   绝不碰 req/prop/gate/lane/stage」。把领域写面塞进去等于把那条规则作废；新增 `tools.reactive`
   两条同处一次 `flush_effects`（同回合、产出 choice 之前），职责不混，旧规则也一字不改。
2. **`req.<i>.conflict` 写注释文本，不写 bool**：`req_conflict_total()` 的读面是 `text_of`，写
   `true` 会让计数器恒为 0（雷达彻底失效）。复用「`conflict` = 注释文本」的既有语义，另加两个纯
   记录键 `conflict_note` / `conflict_source`；写点唯一，仍是 `props.req_do_conflict`。
3. **`LANES.release_lane(k)` 而不是 `LANES.release(index)`**：后者收的是 req index，传泳道号会释放
   错需求。新增一个按泳道号释放的函数，`lane.release` 也改成调它——单点写方，叙事逐字不变。
4. **截断用显式布尔，不靠字符串嗅探**：截断提示里带数字，嗅探必然脆弱；`parse_result` 返回
   `truncated` 并落 `tool.<call_id>.truncated`，雷达读到 true 就跳过比对（宁可漏报不可误报）。
5. **探针失败不接门禁**：接进去会让一次脏树探针把 `coordinate.submit` / `review.note` 永久卡死
   （没有「撤销探针」的动作）。雷达反而是硬阻塞——它走 `req.<i>.conflict`，天然进 `gate_problem()`。
6. **`vfs.write` 说话算数**：宿主没装 `ygg.fs` 时**不假装写过**（叙事 `[VFS LEDGER ONLY]`），
   但账本照记同一条 `record_artifact`，于是「KV 与叙事」在两种宿主下都一致。

### 留下的可判定口子

- `cargo test -p ygg-sandbox --test agent_seed_semantics` → **24/24**（新增 A1..A6 六条，每条都写清
  「变异后哪条断言变红」）。
- playbook 新增 5 条规则（`tools.reactive-effect-scope` / `tools.wire-kv-output` /
  `subsystems.machine-evidence-parity` / `subsystems.probe-is-warning` / `subsystems.radar-blocks`），
  并改掉 `gates.mod-semantics-matrix` 里过期的用例数（12 → 24）。
- `AGENTS.md` §9.2 用例数与 §9.2c 新增的「三条边界」判据。

### 刻意没做的事

- **不新增 `src/reactive.lua`**：新模块要改 `pack.json code.modules`，而 `tools` 段与该段在文件里
  相邻，会把并行改动的冲突面人为放大；`src/tools.lua` 用分区标题保持可读。
- **不动 `render.lua`**：`agent.summary` 已经在读 `verify.passed`，机证复用同一个键即可。
- **不把 `[LANE PROBE FAILED]` 接进 `gate_problem()`**（理由见上）。
