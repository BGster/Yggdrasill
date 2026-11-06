/**
 * orchestrator.ts —— 活跃作业（plan / development）的自动化生命周期编排
 *
 * 一段话说明它替人省了什么：`summary.jobs` 里每个处于 `plan` / `development` 的
 * 需求，原本要人手工做四件事——派 subagent、把产出写进 VFS、跑 `git.lane_probe`、
 * 提 `proposal.submit` 并 `proposal.accept` 把它推到 `plan_gate` / `accept_gate`。
 * 本模块把这四步串成一条流水线：
 *
 *   ⓪ 泳道：development 缺 lane/branch → 分配空闲泳道（1..4）+ 生成分支名 +
 *      `tool.call git.worktree_claim`（**先建 worktree 再起 subagent**）；
 *   ① 派活：`executeSubagent` 并发执行（`Promise.all`），brief 自洽（[`buildSubagentBrief`]）；
 *   ② 归档：subagent 报告（或它自己写的 `artifacts/<r>-<stage>.md`）→ `vfs_put`；
 *   ③ 探针 + 提案：development 跑 `tool.call git.lane_probe`（commits / clean /
 *      跨泳道文件冲突雷达），然后 `proposal.submit` + `proposal.accept`。
 *
 * 边界（刻意不做的）：
 * - **不碰 gate**：`gate.approve` / `gate.reject` 留在协调层（人 / 主 Agent 的决定）。
 * - **不改 Mod**：所有状态推进都走 `daemon.dispatch`，本模块不写 `.ygg/**`。
 * - **不吞回执**：每个 `StepReceipt.narrative` 全行保留（禁止解析式截断）。
 *
 * `executeSubagent` 是注入的（`index.ts` 里用 `ctx.executeTool("Agent", …)`），
 * 所以本文件不 import pi，`harness.test.ts` 能用 mock 直接跑。
 */

import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import type { RunSummary, StepReceipt, SummaryGate, SummaryJob } from "./daemon.ts";

// ───────────────────────────── 注入面 ─────────────────────────────

/** subagent 执行结果（`text` = 它的报告正文；`ok=false` 时带 `error`）。 */
export interface SubagentResult {
  text: string;
  ok?: boolean;
  error?: string;
}

export interface SubagentRequest {
  /** `${job.id} ${job.stage}`（给人看的短标识）。 */
  description: string;
  /** 自洽 brief（[`buildSubagentBrief`] 的产物）。 */
  prompt: string;
  model?: string;
  thinking?: string;
}

export type ExecuteSubagent = (request: SubagentRequest) => Promise<SubagentResult>;

/**
 * 编排需要的最小 daemon 面（`YggDaemon` 结构上满足它）。
 * 写成窄接口而不是直接依赖类：`harness.test.ts` 用普通对象就能测，不需要起进程。
 */
export interface OrchestratorDaemon {
  summary(): Promise<RunSummary>;
  dispatch(event: string, payload?: string | number): Promise<StepReceipt>;
  vfsPut(
    artifactPath: string,
    content: string,
  ): Promise<{ ok: boolean; bytes?: number; sha256?: string; error?: string }>;
}

export interface OrchestrateOptions {
  /** 主工作区（`git rev-parse` 的仓库、worktree 根都相对它）。 */
  cwd: string;
  /** 泳道上限（`data/config.json` 的 job cap = 4）。 */
  maxLanes?: number;
  /** worktree 根（默认 `os.tmpdir()/ygg-worktrees`，与宿主 `worktree-claim` 同约定）。 */
  worktreeRoot?: string;
  /** 仓库目录（默认 `cwd`）。 */
  repoDir?: string;
  /** 执行者 id（写进 `proposal.submit` 的 `exec=` 段）。 */
  actorId?: string;
  model?: string;
  thinking?: string;
  /** 制品路径前缀（默认 `artifacts`）。 */
  artifactPrefix?: string;
  /** 泳道绑定等待：轮询间隔 / 总超时。 */
  lanePollMs?: number;
  laneWaitMs?: number;
  /** 覆盖 base（默认读 `job.base`，缺失时 `git rev-parse HEAD`）。 */
  resolveBase?: (job: SummaryJob) => Promise<string> | string;
  /** 读 subagent 落盘的制品（默认 `fs.readFileSync` 存在即读）。 */
  readArtifact?: (absPath: string) => string | null;
  /** 子代理 brief 里的验证门禁（默认 `cargo test --workspace`）。 */
  verification?: string[];
  logger?: (line: string) => void;
}

/** 一个作业走完全程的结果（含全部回执，narrative 不截断）。 */
export interface OrchestrationJobResult {
  id: string;
  stage: string;
  rev: number;
  lane: number;
  branch: string;
  base: string;
  worktreePath: string | null;
  artifactPath: string;
  brief: string;
  report: string;
  /** 制品入 VFS 的结果（`vfs_put`）。 */
  archived: { ok: boolean; bytes?: number; sha256?: string; error?: string };
  /** 本作业按序产生的回执。 */
  receipts: { label: string; receipt: StepReceipt }[];
  /** 作业结束时应到的门（plan → `plan`，development → `accept`）。 */
  expectedGate: string;
  error?: string;
}

export interface OrchestrationResult {
  jobs: OrchestrationJobResult[];
  gates: SummaryGate[];
  /** 所有回执的 narrative 行（按作业序、步序展开，**整段**保留）。 */
  narrative: string[];
  summary: RunSummary | null;
  /** 无法编排时的原因（摘要不可用 / 无活跃作业以外的问题）。 */
  degraded?: string;
}

const DEFAULT_MAX_LANES = 4;
const DEFAULT_ARTIFACT_PREFIX = "artifacts";
const DEFAULT_LANE_POLL_MS = 200;
const DEFAULT_LANE_WAIT_MS = 20_000;
const DEFAULT_ACTOR_ID = "pi-ygg-harness";
const DEFAULT_VERIFICATION = ["cargo test --workspace"];
const ACTIVE_STAGES = new Set(["plan", "development"]);

// ───────────────────────────── 纯函数：命名与载荷 ─────────────────────────────

/** 标题 → 分支名可用的 slug（小写、连字符、≤40 字节）。 */
export function slugify(text: string, max = 40): string {
  const slug = String(text ?? "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, max)
    .replace(/-+$/g, "");
  return slug === "" ? "work" : slug;
}

/** `r<N>` / `1` / `R7` 一律归一成数字（认不出 → 0）。 */
export function requirementNumber(id: string): number {
  const match = String(id ?? "").match(/(\d+)/);
  if (match === null) {
    return 0;
  }
  return Number(match[1]);
}

/** 泳道分支名：`feature/r<N>-<slug(标题)>`（幂等：同一需求同一分支）。 */
export function branchNameFor(job: SummaryJob): string {
  if (job.branch !== "") {
    return job.branch;
  }
  const number = requirementNumber(job.id);
  const tag = number > 0 ? `r${number}` : slugify(job.id, 12);
  return `feature/${tag}-${slugify(job.title === "" ? job.id : job.title)}`;
}

/** 最小空闲泳道（1..maxLanes，跳过已被其他作业占用的号）；全占满 → `null`。 */
export function pickFreeLane(
  jobs: SummaryJob[],
  maxLanes: number = DEFAULT_MAX_LANES,
  selfId?: string,
): number | null {
  const taken = new Set<number>();
  for (const job of jobs) {
    if (selfId !== undefined && job.id === selfId) {
      continue;
    }
    if (job.lane > 0) {
      taken.add(job.lane);
    }
  }
  for (let lane = 1; lane <= maxLanes; lane += 1) {
    if (!taken.has(lane)) {
      return lane;
    }
  }
  return null;
}

/** worktree 路径（与宿主 `worktree-claim` 同约定：`<root>/<仓库名>-lane-<k>`）。 */
export function worktreePathFor(worktreeRoot: string, repoDir: string, lane: number): string {
  const label = path.basename(path.resolve(repoDir));
  return path.join(worktreeRoot, `${label}-lane-${lane}`);
}

/** 制品路径：`artifacts/<rN>-<stage>.md`（形状固定，便于人肉核对）。 */
export function artifactPathFor(job: SummaryJob, prefix = DEFAULT_ARTIFACT_PREFIX): string {
  return `${prefix}/${job.id}-${job.stage}.md`;
}

/** 提案载荷的单段清洗：禁竖线（段分隔符）、禁换行、限长。 */
export function sanitizeSegment(text: string, max = 200): string {
  const flat = String(text ?? "")
    .replace(/[\r\n|]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  if (flat.length <= max) {
    return flat;
  }
  return `${flat.slice(0, Math.max(0, max - 1))}…`;
}

/** `git.worktree_claim` 的 args（`k=v;k=v`，宿主按位读键名）。 */
export function buildWorktreeClaimArgs(input: {
  lane: number;
  req: string;
  branch: string;
  base: string;
}): string {
  const parts = [`lane=${input.lane}`, `req=${sanitizeSegment(input.req, 24)}`];
  if (input.branch !== "") {
    parts.push(`branch=${sanitizeSegment(input.branch, 96)}`);
  }
  if (input.base !== "") {
    parts.push(`base=${sanitizeSegment(input.base, 64)}`);
  }
  return parts.join(";");
}

/** `git.lane_probe` 的 args（`commits` / `clean` / `head` / `files` 由宿主回填）。 */
export function buildLaneProbeArgs(input: {
  lane: number;
  base: string;
  branch: string;
}): string {
  const parts = [`lane=${input.lane}`];
  if (input.base !== "") {
    parts.push(`base=${sanitizeSegment(input.base, 64)}`);
  }
  if (input.branch !== "") {
    parts.push(`branch=${sanitizeSegment(input.branch, 96)}`);
  }
  return parts.join(";");
}

/**
 * `proposal.submit` 载荷（位次固定）：
 * `rN|rev|stage|summary|artifacts[|changes[|external_id[|exec=<id>[:<reason>]]]]`
 */
export function buildProposalSubmitPayload(input: {
  job: SummaryJob;
  summary: string;
  artifactPath: string;
  actorId?: string;
  externalId?: string;
}): string {
  const { job } = input;
  const rev = job.rev > 0 ? job.rev : 1;
  const summary = sanitizeSegment(input.summary, 240);
  const artifacts = sanitizeSegment(input.artifactPath, 128);
  const external = sanitizeSegment(
    input.externalId ?? `auto-${job.id}-${job.stage}-r${rev}`,
    96,
  );
  const actor = sanitizeSegment(input.actorId ?? DEFAULT_ACTOR_ID, 64);
  const reason = sanitizeSegment(`auto ${job.id} ${job.stage} r${rev}`, 96);
  // 空 changes 段：`…|<artifacts>||<external_id>|exec=…`
  return [
    job.id,
    String(rev),
    job.stage,
    summary,
    artifacts,
    "",
    external,
    `exec=${actor}:${reason}`,
  ].join("|");
}

// ───────────────────────────── 纯函数：brief ─────────────────────────────

const STAGE_RULES: Record<string, string[]> = {
  plan: [
    "本阶段只产出**实现方案**（文件/函数级改动清单、顺序、风险、验证方式），不写实现代码。",
    "方案要自洽到「另一个执行者拿到就能直接写代码」：点名文件路径与函数/符号名。",
    "把方案写到 `artifacts/<rN>-plan.md`（相对工作区），或直接在报告正文里给出全文。",
  ],
  development: [
    "本阶段实现方案并**跑通验证门禁**；提交留在本泳道分支上（不要 push，不要合并）。",
    "只改本需求需要的文件；**不得**触碰其他泳道 / 其他仓库的文件。",
    "把实现摘要 + 验证输出摘要写到 `artifacts/<rN>-development.md`，或直接在报告正文里给出全文。",
  ],
};

/**
 * subagent brief（纯函数，可单测）：自洽到「不看会话也能独立推进」。
 *
 * @param job           `summary.jobs` 的一行
 * @param worktreePath  隔离 worktree 路径（`null` = 本阶段在主工作区只读推进）
 * @param artifactPath  VFS 制品路径（`artifacts/<rN>-<stage>.md`）
 * @param cwd           主工作区（`AGENTS.md` / 验证命令的相对根）
 * @param options       验证门禁与泳道信息（可选）
 */
export function buildSubagentBrief(
  job: SummaryJob,
  worktreePath: string | null,
  artifactPath: string,
  cwd: string,
  options: {
    verification?: string[];
    lane?: number;
    branch?: string;
    base?: string;
    actorId?: string;
  } = {},
): string {
  const stage = job.stage === "" ? "plan" : job.stage;
  const verification =
    options.verification !== undefined && options.verification.length > 0
      ? options.verification
      : DEFAULT_VERIFICATION;
  const workspace = worktreePath ?? cwd;
  const lines: string[] = [];

  lines.push(`# 需求 ${job.id} · ${stage} 阶段执行 brief`);
  lines.push("");
  lines.push("你是一个隔离 worktree 内的执行者。下面这些信息**自洽**：不需要读别的会话，");
  lines.push("也不需要向协调者追问；不确定时按「最小改动 + 保留既有约定」处理并在报告里标注。");
  lines.push("");
  lines.push("## 1. 工作区（硬边界）");
  lines.push(`- 作用域根：\`${workspace}\``);
  if (worktreePath === null) {
    lines.push(
      `- 本阶段没有独立 worktree（plan 阶段）：只在 \`${cwd}\` 内做**只读**分析与方案撰写，不要改代码。`,
    );
  } else {
    lines.push(`- **只在这个目录里工作**：\`${worktreePath}\``);
    lines.push("- 主检出与其他泳道（`*-lane-<其他号>`）**一律不许改**；跨泳道的冲突会被探针抓成硬冲突。");
  }
  if (options.lane !== undefined && options.lane > 0) {
    lines.push(`- 泳道：lane-${options.lane}`);
  }
  if (options.branch !== undefined && options.branch !== "") {
    lines.push(`- 分支：\`${options.branch}\`（提交留在本分支，**不要 push / 不要合并**）`);
  }
  if (options.base !== undefined && options.base !== "") {
    lines.push(`- 基线 commit：\`${options.base}\``);
  }
  lines.push("");
  lines.push("## 2. 需求");
  lines.push(`- 标识：\`${job.id}\`（rev ${job.rev > 0 ? job.rev : 1}）`);
  lines.push(`- 标题：${job.title === "" ? job.id : job.title}`);
  lines.push("- 描述（权威需求正文，逐条落实）：");
  const description = job.description.trim() === "" ? "(描述为空：按标题与仓库既有约定推进，并在报告里声明这一点)" : job.description;
  for (const line of description.split(/\r?\n/)) {
    lines.push(`  ${line}`);
  }
  lines.push("");
  lines.push("## 3. 硬约束（不可协商）");
  lines.push("1. **数据红线（优先级 0）**：不得读取 / 列出 / 统计 / 写入任何真实数据文件");
  lines.push("   （`*.xlsx` / `*.xls` / `*.csv`、涉密挂载目录 `/Volumes/WenshuSpace/**`）。");
  lines.push("   只有 `/tmp` 下自造的合成数据可以读写；需要真实数据验证时把命令交给人运行。");
  lines.push(`2. 先读 \`${path.join(workspace, "AGENTS.md")}\`（若存在）与目标目录的就近约定，冲突时以仓库内约定为准。`);
  lines.push("3. 不得改写既有测试断言来「让它通过」；不得删别人的代码来消错。");
  lines.push("4. 不得操作 git 历史（no rebase / no force-push / no branch -D），不碰 `main` / `master`。");
  lines.push("");
  lines.push("## 4. 本阶段要求");
  for (const rule of STAGE_RULES[stage] ?? STAGE_RULES.plan) {
    lines.push(`- ${rule}`);
  }
  lines.push("");
  lines.push("## 5. 验证门禁（必须真跑，贴输出摘要）");
  for (const command of verification) {
    lines.push(`- \`${command}\``);
  }
  lines.push("- 门禁失败时不要声称完成：报告里写清失败命令、首行错误、你已排除的方向。");
  lines.push("");
  lines.push("## 6. 返回格式（协调者按这个形状归档）");
  lines.push("1. **做了什么**：文件路径 + 符号级改动清单（`path:line` 形式优先）。");
  lines.push("2. **验证结果**：每条门禁命令 + 结论（passed / failed + 摘要）；没跑就说没跑。");
  lines.push("3. **风险与遗留**：未覆盖的场景、你知道但没修的问题。");
  lines.push(`4. **制品**：正文或 \`${artifactPath}\` 的内容（协调者会把它写进本局 VFS）。`);
  lines.push("");
  lines.push("报告里禁止使用竖线 `|` 作为分隔符（它会被当成提案载荷的分段符）。");
  return lines.join("\n");
}

// ───────────────────────────── 编排主流程 ─────────────────────────────

interface PreparedJob {
  job: SummaryJob;
  lane: number;
  branch: string;
  base: string;
  worktreePath: string | null;
  artifactPath: string;
  brief: string;
  expectedGate: string;
  receipts: { label: string; receipt: StepReceipt }[];
  claimed: boolean;
}

function logWith(logger: ((line: string) => void) | undefined, line: string): void {
  if (logger !== undefined) {
    logger(line);
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}

/** 读 `git rev-parse HEAD`（只有同步 IO；测试注入 `resolveBase` 即可绕过）。 */
function defaultResolveBase(cwd: string): string {
  try {
    const head = fs.readFileSync(path.join(cwd, ".git", "HEAD"), "utf8").trim();
    if (head.startsWith("ref: ")) {
      const ref = head.slice("ref: ".length).trim();
      const refFile = path.join(cwd, ".git", ref);
      return fs.readFileSync(refFile, "utf8").trim();
    }
    return head;
  } catch {
    return "";
  }
}

/** `artifacts/<…>.md` 存在即读（不存在 → `null`，退回 subagent 报告正文）。 */
function defaultReadArtifact(absPath: string): string | null {
  try {
    const text = fs.readFileSync(absPath, "utf8");
    return text === "" ? null : text;
  } catch {
    return null;
  }
}

/**
 * 对 `summary.jobs` 里所有 plan / development 作业跑完整流水线。
 *
 * 回执一条都不吞：每个作业的 `receipts` 保留 label + 完整 [`StepReceipt`]，
 * 顶层 `narrative` 再按序平铺一份，方便协调层一次性贴给用户。
 */
export async function orchestrateActiveJobs(
  daemon: OrchestratorDaemon,
  executeSubagent: ExecuteSubagent,
  opts: OrchestrateOptions,
): Promise<OrchestrationResult> {
  const {
    cwd,
    maxLanes = DEFAULT_MAX_LANES,
    worktreeRoot = path.join(os.tmpdir(), "ygg-worktrees"),
    repoDir = cwd,
    actorId = DEFAULT_ACTOR_ID,
    artifactPrefix = DEFAULT_ARTIFACT_PREFIX,
    lanePollMs = DEFAULT_LANE_POLL_MS,
    laneWaitMs = DEFAULT_LANE_WAIT_MS,
    verification = DEFAULT_VERIFICATION,
    logger,
  } = opts;

  const summary = await daemon.summary();
  if (!summary.available) {
    return {
      jobs: [],
      gates: [],
      narrative: [],
      summary,
      degraded: summary.degraded ?? "摘要不可用（宿主没有给出 `meta.summary`）",
    };
  }

  const active = summary.jobs.filter((job) => ACTIVE_STAGES.has(job.stage));
  if (active.length === 0) {
    return {
      jobs: [],
      gates: summary.gates,
      narrative: [],
      summary,
      degraded: "没有处于 plan / development 的作业",
    };
  }

  const resolveBase =
    opts.resolveBase ??
    ((job: SummaryJob) =>
      job.base !== "" ? job.base : defaultResolveBase(repoDir));
  const readArtifact = opts.readArtifact ?? defaultReadArtifact;

  // ── ⓪ 泳道：先建 worktree 再起 subagent（否则 subagent 没有隔离作用域） ──
  const prepared: PreparedJob[] = [];
  const ledger: SummaryJob[] = [...summary.jobs];
  for (const job of active) {
    const expectedGate = job.stage === "development" ? "accept" : "plan";
    const receipts: { label: string; receipt: StepReceipt }[] = [];
    let lane = job.lane > 0 ? job.lane : 0;
    let branch = job.branch;
    let base = job.base;
    let claimed = false;

    const needsClaim = job.stage === "development" && (lane === 0 || branch === "");
    if (needsClaim) {
      const free = pickFreeLane(ledger, maxLanes, job.id);
      lane = free ?? 1;
      branch = branchNameFor(job);
      base = base !== "" ? base : String(await resolveBase(job));
      const args = buildWorktreeClaimArgs({ lane, req: job.id, branch, base });
      logWith(logger, `[lane] ${job.id} → lane-${lane} (${branch})`);
      const claim = await daemon.dispatch("tool.call", `git.worktree_claim|${args}`);
      receipts.push({ label: `tool.call git.worktree_claim (${job.id})`, receipt: claim });
      // 宿主的工具结果走 inbox：再查一次摘要（顺带 drain），确认泳道已绑。
      claimed = await waitForLaneBound(daemon, job.id, lane, { pollMs: lanePollMs, waitMs: laneWaitMs });
      if (!claimed) {
        logWith(logger, `[lane] ${job.id} 泳道绑定未在超时内确认（继续推进，回执里有现场）`);
      }
      ledger.push({ ...job, lane, branch, base });
    } else if (lane > 0) {
      base = base !== "" ? base : String(await resolveBase(job));
    }

    const worktreePath = lane > 0 ? worktreePathFor(worktreeRoot, repoDir, lane) : null;
    const artifactPath = artifactPathFor(job, artifactPrefix);
    const brief = buildSubagentBrief(job, worktreePath, artifactPath, cwd, {
      verification,
      lane,
      branch,
      base,
      actorId,
    });
    prepared.push({
      job,
      lane,
      branch,
      base,
      worktreePath,
      artifactPath,
      brief,
      expectedGate,
      receipts,
      claimed,
    });
  }

  // ── ① 派活：独立作业并发（subagent 之间无共享状态） ──
  const outcomes = await Promise.all(
    prepared.map(async (item) => {
      try {
        const request: SubagentRequest = {
          description: `${item.job.id} ${item.job.stage}`,
          prompt: item.brief,
        };
        if (opts.model !== undefined) {
          request.model = opts.model;
        }
        if (opts.thinking !== undefined) {
          request.thinking = opts.thinking;
        }
        const result = await executeSubagent(request);
        const ok = result.ok !== false;
        return {
          item,
          report: result.text ?? "",
          error: ok ? undefined : (result.error ?? "subagent 报告失败"),
        };
      } catch (error) {
        return {
          item,
          report: "",
          error: error instanceof Error ? error.message : String(error),
        };
      }
    }),
  );

  // ── ②③ 归档 / 探针 / 提案：串行（Mod 是单写者，回执序才可复核） ──
  const jobs: OrchestrationJobResult[] = [];
  const narrative: string[] = [];

  for (const outcome of outcomes) {
    const { item } = outcome;
    const receipts = item.receipts;
    let report = outcome.report.trim();
    let error = outcome.error;

    // 制品优先：subagent 按 brief 写了落盘制品就用它（内容比聊天报告更完整）。
    const artifactOnDisk =
      item.worktreePath === null
        ? path.join(cwd, item.artifactPath)
        : path.join(item.worktreePath, item.artifactPath);
    const diskContent = readArtifact(artifactOnDisk);
    if (diskContent !== null) {
      report = diskContent;
    }
    if (report === "") {
      report = `(subagent 未返回正文：${error ?? "空报告"})`;
      error = error ?? "subagent 未返回正文";
    }

    const archived = await daemon.vfsPut(item.artifactPath, report);
    const archiveReceipt: StepReceipt = {
      ok: archived.ok,
      narrative: archived.ok
        ? [
            `[VFS] ${item.artifactPath} ← ${report.length} bytes` +
              (archived.sha256 !== undefined ? ` sha256=${archived.sha256.slice(0, 12)}` : ""),
          ]
        : [`[VFS REFUSED] ${item.artifactPath}：${archived.error ?? "未知原因"}`],
      pendingChoice: null,
      tools: [],
      raw: { ...archived },
    };
    if (!archived.ok) {
      archiveReceipt.error = archived.error ?? "vfs_put 失败";
    }
    receipts.push({
      label: `vfs_put ${item.artifactPath} (${item.job.id})`,
      receipt: archiveReceipt,
    });

    if (item.job.stage === "development" && item.lane > 0) {
      const probe = buildLaneProbeArgs({ lane: item.lane, base: item.base, branch: item.branch });
      const probeReceipt = await daemon.dispatch("tool.call", `git.lane_probe|${probe}`);
      receipts.push({
        label: `tool.call git.lane_probe (${item.job.id} lane-${item.lane})`,
        receipt: probeReceipt,
      });
      // 探针是**警告面**（commits=0 / clean=0 不阻塞门禁），但要 drain 一次让
      // 冲突雷达与 `lane.<k>.probe.*` 落库。
      await daemon.summary();
    }

    const submitPayload = buildProposalSubmitPayload({
      job: item.job,
      summary: `auto ${item.job.stage}: ${firstLine(report)}`,
      artifactPath: item.artifactPath,
      actorId,
    });
    const submit = await daemon.dispatch("proposal.submit", submitPayload);
    receipts.push({ label: `proposal.submit (${item.job.id})`, receipt: submit });

    const accept = await daemon.dispatch("proposal.accept", item.job.id);
    receipts.push({ label: `proposal.accept (${item.job.id})`, receipt: accept });
    if (!accept.ok && error === undefined) {
      error = accept.error ?? undefined;
    }

    for (const entry of receipts) {
      for (const line of entry.receipt.narrative) {
        narrative.push(`[${item.job.id}] ${line}`);
      }
    }

    const jobResult: OrchestrationJobResult = {
      id: item.job.id,
      stage: item.job.stage,
      rev: item.job.rev,
      lane: item.lane,
      branch: item.branch,
      base: item.base,
      worktreePath: item.worktreePath,
      artifactPath: item.artifactPath,
      brief: item.brief,
      report,
      archived,
      receipts,
      expectedGate: item.expectedGate,
    };
    if (error !== undefined) {
      jobResult.error = error;
    }
    jobs.push(jobResult);
  }

  const after = await daemon.summary();
  const touched = new Set(jobs.map((job) => job.id));
  const gates = (after.available ? after.gates : []).filter((gate) => touched.has(gate.id));

  return {
    jobs,
    gates,
    narrative,
    summary: after.available ? after : summary,
  };
}

function firstLine(text: string): string {
  for (const line of String(text ?? "").split(/\r?\n/)) {
    const trimmed = line.trim();
    if (trimmed !== "") {
      return trimmed.replace(/^#+\s*/, "");
    }
  }
  return "(empty)";
}

/** 轮询摘要直到 `job.lane === lane`（超时返回 false；不抛）。 */
async function waitForLaneBound(
  daemon: OrchestratorDaemon,
  jobId: string,
  lane: number,
  timing: { pollMs: number; waitMs: number },
): Promise<boolean> {
  const deadline = Date.now() + timing.waitMs;
  for (;;) {
    const summary = await daemon.summary();
    if (summary.available) {
      const row = summary.jobs.find((job) => job.id === jobId);
      if (row !== undefined && row.lane === lane) {
        return true;
      }
    }
    if (Date.now() >= deadline) {
      return false;
    }
    await sleep(timing.pollMs);
  }
}
