# 迭代记录（History）

> 只记「为什么这样落」与「留下了什么可判定的口子」；实现细节看代码与各 crate 头注。

## r-pi-harness · 上层扩展 `.pi/extensions/ygg-pi-harness`（CLI 之上的会话面）

**分支** `feature/pi-extension-harness`（base `6cc7fc4`）。目标：把「人肉照 README 位次敲一串
`--event`」升级成**两个工具**，同时把三条靠自觉的纪律变成**物理闸门**——数据红线、批量入口禁令、
「回执整段呈现」。

### 落成的四件事

1. **闸门（`firewall.ts`）**：`tool_call` 上的纯函数 `checkToolCallFirewall(tool, input)`。
   规则 1 拦 `read`/`write`/`edit`/`ls`/`find`/`grep`/`bash` 触碰 `/Volumes/WenshuSpace/**` 或任何
   `*.xlsx`/`*.xls`/`*.csv`（`/tmp` 合成数据除外，判据是**动作**不是「读没读内容」）；
   规则 2 拦 `bash` 里的 `ygg-agent --events`、`for/while … do … ygg-agent … done`、`xargs ygg-agent`。
   只扫**路径语义**的入参键（`write.content` 里出现 `.csv` 不误伤），`grep.pattern` 是正则不扫、
   `find.pattern` 是 glob 要扫。
2. **常驻客户端（`daemon.ts`）**：`ygg-agent --daemon` 的行式 JSON-RPC（按 `id` 配对、请求串行化、
   30s 超时、意外退出重启）。宿主没有 `--daemon` 时**降级单发**并把摘要标成 `available=false`：
   单发回执本来就没有 `meta.summary`，宁可报「不知道」也不猜 jobs/gates。
3. **泳道编排（`orchestrator.ts`）**：`summary.jobs` 里每个 `plan`/`development` 作业走
   「分配泳道 → `git.worktree_claim`（**先建 worktree 再起 subagent**）→ 并发派 subagent →
   报告/落盘制品 `vfs_put` → `git.lane_probe`（含跨泳道文件冲突雷达）→ `proposal.submit` →
   `proposal.accept`」，把人送到 `plan_gate`/`accept_gate` 门口。brief 由纯函数
   `buildSubagentBrief` 生成（工作区边界 / 需求正文逐行 / 验证门禁 / 返回格式 / 红线）。
4. **装配（`index.ts`）**：`session_start`/`session_shutdown`、`tool_call` 闸门、`/ygg status|stop`、
   工具 `ygg_session`（kickoff / step / approve_gate / reject_gate / vfs_* / settle_close）与
   `ygg_run_lanes`，页脚三段式状态行。`settle_close` 把 `coordinate.submit → verify.submit →
   进化复盘入 VFS → harness.archive_sessions → settlement.evolution → review.note → settle`
   串成一次调用，且**被拒就停**（不把「被拒」洗成「已收口」）。

### 留下的可判定口子（下次可以据此复查）

| 口子 | 判定方式 |
| --- | --- |
| 红线拦得住（目录 / 三种数据扩展名 / bash 里的路径与 `--flag=/path`） | `harness.test.ts`「firewall: 数据红线」用例 |
| 红线不误伤（代码文件 / `/tmp` 合成数据 / `write.content` 里的 `.csv` 字面量） | 同文件「放行 /tmp」与「只扫路径语义的入参」用例 |
| 批量入口拦得住（`--events` / `for` 循环 / `while` 循环 / `xargs`），单发 `--event` 放行 | 「firewall: 批量入口」用例 |
| brief 自洽（worktree 边界 / 需求正文逐行 / 门禁命令 / plan 只读） | 「buildSubagentBrief」两条用例 |
| 先 claim 再派活、并发、`vfs_put`、探针、提案位次、落对门 | 「orchestrateActiveJobs」四条用例 |
| 摘要不可用时不推进（不猜 jobs/gates） | 「摘要不可用时不猜」用例 |
| daemon 行协议（id 配对 / 串行化 / 起不来就降级） | 「YggDaemon: 行协议」三条用例（假 daemon = `process.execPath` + 临时脚本） |
| `.env` 只暴露两项 | 「daemon: .env 只暴露两项」用例 |

跑法（无外部依赖）：

```bash
node --experimental-strip-types --test .pi/extensions/ygg-pi-harness/harness.test.ts
```

### 刻意没做的事（避免被误当成「漏了」）

- **不代批 gate**：`ygg_run_lanes` 停在门口，`gate.approve`/`gate.reject` 仍是协调者（人）的决定。
  自动批等于没有门禁，这条写进了 `AGENTS.md` §3 与两份 Mod README。
- **不在扩展里放第二套状态机**：所有推进都走 `daemon.dispatch` → Mod handler；扩展不写
  `.ygg/runs/**`，也不解释 `pendingChoice`（原样交给用户）。
- **不改 `crates/**` 与 Mod 的 `src/**`**：并行泳道 `r1` 在改宿主与 Mod（`--daemon`、`summary.jobs`），
  本泳道只落上层扩展 + 文档，文件集 100% 不相交。
- **不伪造摘要**：单发降级下 `summary.available=false`，`orchestrateActiveJobs` 直接返回降级原因，
  不会因为拿不到 `jobs` 就假装「没有作业」。

## r-rename · Seed Mod 更名 `agent-seed` → `pi-agent-seed`（历史局不动）

**分支** `feature/pi-agent-seed-rename`（base `bac2a40`）。目标：把两侧 Mod 目录
（叶子模板 `packages/mods/pi-agent-seed`、项目实例 `.ygg/mods/pi-agent-seed`）改到
带宿主前缀的名字上，**同时保证 `.ygg/runs/**` 里的历史局原样可续**——不需要手工迁移任何 `run.json`。

### 落成的三件事

1. **目录更名**：`git mv` 两侧 Mod 目录（骨架面 / 实例面按原分层一起搬）；
   脚本、CI 骨架守卫、宿主帮助文案、测试定位、文档命令路径同步到新名。
   测试目标名 `agent_seed_semantics` **不变**：它在 AGENTS.md 与 playbook 里是被引用的判据命令，
   改名是断链不是整理。
2. **解析期兼容别名**：`examples/agent.rs` 的 `resolve_mod_dir` 在「请求路径不存在 +
   形状是 `…/mods/agent-seed` + 同父目录下 `pi-agent-seed` 存在」三条同时成立时，
   翻译到新名（`legacy_seed_alias` / `with_legacy_seed_alias`）。只换最后两段、只发生在内存里，
   `--mod` 与 `run.json` 走同一条翻译；磁盘面只在**该局续局并保存**时按宿主原本的
   持久化逻辑落成新名（不是迁移脚本改盘）。
3. **三处同源代码**：`schemas/agent-config.json` 三份（两份 Mod + `packages/schema/`）
   逐字节一致（改名只落在 `description` 文本上），再用 `sync-seed-mod.sh --from project`
   重算两侧 `manifest.json`。

### 留下的可判定口子（下次可以据此复查）

| 口子 | 判定方式 |
| --- | --- |
| 历史 `run.json` 不改也能解析到新目录 | `mod_dir_alias_tests::legacy_run_json_resolves_to_the_renamed_mod_dir`（`cargo test --example agent -p ygg-sandbox`） |
| 相对路径字面量映射（`.ygg/mods/agent-seed` → `.ygg/mods/pi-agent-seed`） | `mod_dir_alias_tests::legacy_relative_path_maps_to_the_renamed_sibling` |
| `--mod` 与 `run.json` 同一条翻译 | `mod_dir_alias_tests::the_mod_flag_takes_the_same_translation` |
| 别名不越界（只认 `mods/agent-seed`） | `mod_dir_alias_tests::mapping_is_narrow_on_purpose` |
| 旧目录还在磁盘 → 不被抢改 | `mod_dir_alias_tests::an_existing_legacy_dir_is_never_rewritten` |
| 两侧都不存在 → 原样返回（不猜默认值） | `mod_dir_alias_tests::missing_both_sides_keeps_the_requested_path` |
| 新目录名能被真装载 | `cargo test -p ygg-sandbox --test agent_seed_semantics`（两侧 pack 装载） |
| 骨架逐字节一致 | CI 骨架守卫（`diff -ru` `mod.lua` / `src/` / `schemas/`） |

### 刻意没做的事（避免被误当成「漏了」）

- **不改测试目标名** `agent_seed_semantics`：AGENTS.md / playbook / history 都按这个名字写判据。
- **不做历史 `run.json` 批量重写**：盘上改历史局等于毁证据，兼容面收敛在解析期一处。
- **不保留旧名目录（连软链也没有）**：留一条旧路径就多一套能装出**不同签名**的同名 Mod，
  兼容面只允许存在一个可判定入口。

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

## r1-harness · 检查点交接 + 自动压缩续跑，subagent `create + resume` 全生命周期

**分支** `feature/context-handoff-and-resume`（base `37d9958`）。目标有两个，都围绕同一件事：
**别让一条长跑的本局因为「压掉上下文」或「subagent 走了」而丢掉现场**。

1. **上下文压力 → 检查点交接 → 压缩 → 自动续跑**（`compaction.ts` 新增，`index.ts` 接线）。
2. **subagent 的真身份**：`Agent` 工具给出的 `agent_id` 写进 `exec=` 账本，下一次重派 /
   跨阶段 / 失败的场合都 `resume` 同一个 subagent，而不是重新叙述一遍 brief。

### 为什么这样落

1. **「不知道用量」不等于「压力低」也不等于「该刹车」**：只用 `getContextUsage()` 判断时，
   刚压完是 `null`、模型不报 `contextWindow` 也是常事。取不到数字就按 `normal` 处理——
   刹车的副作用（整局停在半路）比「晚一点再压」大得多；反过来，数字可信时一律按水位办。
2. **只在稳定节点上压缩**（`isStableCheckpoint`）：门禁上 / 干净阶段边界 / 冲突边界。作业在飞
   又没有门的时候压缩，会把手上那波 subagent 的结果压成一段叙述；落在门上压缩，压完的局面
   是「谁的提案停在哪个门」，可以直接复核。硬刹车（≥75%）是唯一例外：此时**一个 subagent 都
   没起**，压掉不会有任何在飞结果。
3. **硬刹车先交接、软刹车先落地**：都是「先把本波收干净再压」，区别是硬刹车连 claim 都不做
   （不建 worktree），软刹车让本波跑完、`proposal.accept` 完再写交接单。
4. **压缩一定先落 VFS**：`session_before_compact` 里写 `artifacts/handoff-latest.md`，任何来源的
   压缩（`/compact`、阈值、溢出恢复）都覆盖到。写失败**安静吞掉**——把压缩拦下来比丢一份交接单更糟。
5. **自己触发的压缩走 `customInstructions`，外部压缩走消息注入**：`ctx.compact()` 会先 `abort()`
   当前回合，所以自己压完必须用 `triggerTurn: true` 把局面重新送进模型（工具返回值大概率到不了）；
   而人敲的 `/compact` 之后停在原地是预期行为，只注入状态块、`triggerTurn: false`。
6. **`resume` 的三条来源各自有据**：`job.executor`（gate 打回后同阶段重派，rev+1）→ 续原执行者；
   `job.planExecutor` + `resumePlanAgent`（plan → development）→ 续方案执行者；都没有 → 新起一个。
   猜一个 id 去 resume 只会变成「新起一个」，还把 `exec=` 账本写脏。
7. **自愈只补一次，且只在知道续谁时补**：流断开一次（`ok=false` + `agentId`）、脏工作树一次
   （探针 `commits=0` / `clean=0` + `agentId`，提示词逐字要求 `git add` + `git commit`，随后复跑探针）。
   探针真值读不到（`known=false`）就什么都不做——重试风暴比脏树更难查。

### 宿主侧的一个实情（决定了探针真值怎么读）

Mod 的 `lane.<k>.probe.*` 不在 `agent.summary` 的暴露面里，宿主的 `summary` op 只回 drain 的
计数（`{applied, skipped, remaining}`）而不回折叠叙事。所以探针真值有两个来源，按顺序取：
回执叙事里的 `commits=` / `clean=`（`parseLaneProbeNarrative`），读不到再退回**本地只读 git**
（`status --porcelain` / `rev-list --count`）。后者只用来决定「要不要催一次 commit」，
**不进门禁、不进结算**——门禁与结算永远只看 Mod 的账本。

### 留下的可判定口子

- `node --experimental-strip-types --test .pi/extensions/ygg-pi-harness/harness.test.ts` → **75/75**
  （原 37 + 新 38）。
- 四条变异探针（改回旧语义后必须变红）：`resolveResumeId` 恒 `""` → 3 条红；`laneProbeNeedsCommit`
  恒 `false` → 脏树自愈红；删掉硬刹车 → 硬刹车用例红；`exec=` 硬编码 `actorId` → create 用例红。
- `AGENTS.md` §3 表格补 `compaction.ts`，§3.1 把水位 / 稳定节点 / 交接单 / 压缩钩子 / 子代理生命
  周期写成一张可判定的表，并把「不变的红线」补两条（不半路压缩、不 resume 猜出来的 id）。

### 刻意没做的事

- **不改 Mod**（`src/**` / `data/*.json` / `schemas/*`）：本泳道只动扩展；`executor` /
  `planExecutor` 按「宿主给了就读、没给就落空串」解析，旧 Mod 零改动照常工作。
- **不替 Mod 记账**：本地 git 探测只是决策输入，不写任何 KV、不冒充 `git.lane_probe` 的证据。
- **不做「压缩后自动过门禁」**：门禁是人的决定，压缩只负责把局面摆回来。
