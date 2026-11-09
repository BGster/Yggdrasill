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
import { spawnSync } from "node:child_process";
import type { RunSummary, StepReceipt, SummaryGate, SummaryJob } from "./daemon.ts";
import {
  formatPercent,
  writeHandoffToVfs,
  type ContextPressureView,
  type HandoffInput,
  type HandoffWrite,
} from "./compaction.ts";

// ───────────────────────────── 注入面 ─────────────────────────────

/**
 * subagent 执行结果（`text` = 它的报告正文；`ok=false` 时带 `error`）。
 *
 * 跨 harness 适配器契约（P0-1）：`agentId` 是本接口与具体主机 subagent 工具之间的
 * **唯一续跑凭据**。`index.ts` 里的 `executeSubagent` 用 Pi Coding Agent 的
 * `Agent` 工具（`description` / `prompt` / `subagent_type` / `run_in_background` /
 * `resume`）并从返回的 `details.agentId` / 正文 `Agent ID:` 里挖出它；换到其它 harness
 * （Claude Code / Codex / Cursor / DSH）时，适配器必须**产出同样的 `agentId` 语义**：
 * 挖得到就填、挖不到就**省略**（`extractAgentId` 返回 `undefined`），宁可每次新起也不
 * 许猜一个 id 去 `resume`（猜错 = 静默把「续跑」变成「新起」并丢掉 `exec=` 账本）。
 */
export interface SubagentResult {
  text: string;
  ok?: boolean;
  error?: string;
  /** Agent 工具给出的**真实** agent id（有它才能「续跑同一个 subagent」）。 */
  agentId?: string;
}

export interface SubagentRequest {
  /** `${job.id} ${job.stage}`（给人看的短标识）。 */
  description: string;
  /** 自洽 brief（[`buildSubagentBrief`] 的产物）。 */
  prompt: string;
  model?: string;
  thinking?: string;
  /** 续跑既有 subagent（对上 `Agent` 工具的 `resume` 参数）；`""` / 缺省 = 新起一个。 */
  resume?: string;
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
  /** 本局已有制品（交接单的「VFS 制品」一节用；缺省 = 不列）。 */
  vfsList?(): Promise<{ ok: boolean; artifacts: string[]; error?: string }>;
  /**
   * drain 一次 inbox 并把这一步的回执交回（`summary` op 会顺带 drain）。
   * 探针的折叠叙事若出现在 drain 里，[`parseLaneProbeNarrative`] 就能读到它。
   */
  drain?(): Promise<StepReceipt>;
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
  /** 本局标识（交接单抬头 / 下一步命令都要它）。 */
  runId?: string;
  /**
   * 上下文压力探针（`index.ts` 传 `() => evaluateContextPressure({...})`）。
   * 看到 `hard_brake` → **一个字节都不派**，先交接；看到 `soft_brake` → 本波照跑，
   * 落地（提案 accept 完 → 停在门上）之后写交接单交给上层压缩。
   */
  checkContextPressure?: () => ContextPressureView | null | undefined;
  /** plan → development 跨阶段续跑（默认关：并行泳道里让原方案执行者接着干是显式选择）。 */
  resumePlanAgent?: boolean;
  /**
   * 优先消费 Core 产出的 `job.subagentAction.brief` / `deltaPrompt`（默认 `true`）。
   * 显式传 `false` 或调用方传了自定义 `verification` / `verificationCommands` 时退回本地拼装或按需补齐门禁尾注。
   */
  preferCoreBrief?: boolean;
  /** 子代理 brief 里的验证门禁（`verification` 的同义别名，兼容调用方直传）。 */
  verificationCommands?: string[];
  /**
   * 跳过 `proposal.submit` 前显式的 `tool.call git.lane_probe` 往返（默认 `false`）。
   * 设为 `true` 时交由 Core 在 `proposal.submit` 内同步执行 `ygg.tool.call("git.lane_probe", ...)`。
   */
  useInlineLaneProbe?: boolean;
  /** 每个作业的自愈重试上限（默认 1：流断开一次、脏工作树一次，再多就是真 bug）。 */
  maxSelfHealRetries?: number;
  /**
   * 读泳道探针真值（默认：先解析回执叙事里的 `commits=` / `clean=`，读不到再退回
   * 本地 git 只读探测）。读不到真值时**不自愈**——宁可不动，也不刷重试。
   */
  readLaneProbe?: (context: LaneProbeContext) => LaneProbeVerdict | Promise<LaneProbeVerdict>;
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
  /** 本次实际执行/续跑的 subagent ID（若可识别）。 */
  agentId?: string;
  /** worktree / subagent / probe 进度摘要。 */
  worktreeStatus?: string;
  subagentStatus?: string;
  probeStatus?: string;
  error?: string;
}

export interface OrchestrationResult {
  jobs: OrchestrationJobResult[];
  gates: SummaryGate[];
  /** 所有回执的 narrative 行（按作业序、步序展开，**整段**保留）。 */
  narrative: string[];
  summary: RunSummary | null;
  /** 本次编排是否因上下文压力刹车（`true` = 上层应接手写交接单 + 压缩 + 续跑）。 */
  contextBraked: boolean;
  /** 本次看到的上下文压力（`null` = 没接探针）。 */
  pressure?: ContextPressureView | null;
  /** 已写进 VFS 的交接单（`artifacts/handoff-latest.md`）。 */
  handoff?: HandoffWrite;
  /** 无法编排时的原因（摘要不可用 / 无活跃作业以外的问题）。 */
  degraded?: string;
}

const DEFAULT_MAX_LANES = 4;
const DEFAULT_ARTIFACT_PREFIX = "artifacts";
const DEFAULT_LANE_POLL_MS = 200;
const DEFAULT_LANE_WAIT_MS = 20_000;
const DEFAULT_ACTOR_ID = "pi-ygg-harness";
const DEFAULT_VERIFICATION = ["cargo test --workspace"];
const DEFAULT_MAX_SELF_HEAL_RETRIES = 1;
/**
 * 不可续跑的占位执行者 ID 黑名单（**单一来源**）。
 *
 * 这串常量与 Mod 侧 `.ygg/mods/pi-agent-seed/src/render.lua` 的
 * `is_resumable_agent_id`（`unregistered` / `pi-ygg-harness` / `main` / `default`）
 * 必须保持逐字一致：任何一侧漏加一个新占位符（如未来的 `pi-dsh-harness`）都会造成
 * 「Mod 报 `resumeAgentId` 而扩展拒收、或反之」的静默跨阶段续跑失效。
 * 一致性由 `harness.test.ts` 的 parity 用例钉住（单侧加占位符 → 测试红）。
 */
export const RESERVED_AGENT_IDS = new Set<string>([
  "unregistered",
  "pi-ygg-harness",
  "main",
  "default",
]);
/** 脏工作树的自愈提示：**逐字**指定要做的事，别让 subagent 自己猜。 */
export const COMMIT_NUDGE_TEXT =
  "git.lane_probe reported uncommitted changes or 0 commits ahead of base. " +
  "Please git add and git commit your changes in the worktree now.";
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

/** 制品路径：优先取 `job.subagentAction.artifactPath`，缺省退回 `artifacts/<rN>-<stage>.md`。 */
export function artifactPathFor(job: SummaryJob, prefix = DEFAULT_ARTIFACT_PREFIX): string {
  const corePath = (job.subagentAction?.artifactPath ?? "").trim();
  if (corePath !== "") {
    return corePath;
  }
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

// ───────────────────────────── 纯函数：subagent 身份 ─────────────────────────────

/**
 * 从 `Agent` 工具的返回里取出**真实** agent id。
 *
 * 两条路：结果 `details` 里的 `agentId` / `agent_id` / `id`（结构化，优先），
 * 或正文里的 `Agent ID: <id>`（后台运行时的固定写法）。取不到就返回 `undefined`——
 * 拿一个猜出来的 id 去 `resume` 只会把「续跑」变成「新起一个」并丢掉 `exec=` 账本。
 */
export function extractAgentId(text: string, details?: unknown): string | undefined {
  if (details !== null && typeof details === "object") {
    const record = details as Record<string, unknown>;
    for (const key of ["agentId", "agent_id", "id"]) {
      const value = record[key];
      if (typeof value === "string" && value.trim() !== "") {
        return value.trim();
      }
    }
  }
  const match = /Agent ID:\s*([A-Za-z0-9_-]+)/.exec(String(text ?? ""));
  return match === null ? undefined : match[1];
}

/**
 * 判断候选 agent ID 是否是可传给 `Agent(resume=...)` 的**真实** subagent 会话 ID。
 *
 * 同步或兜底路径写进账本的占位值（见 {@link RESERVED_AGENT_IDS}：
 * `"unregistered"` / `"pi-ygg-harness"` / `"main"` / `"default"`，以及调用方自定义的
 * `fallbackActorId`）并不是可续跑的 subagent 会话，若直接传给 `resume`
 * 会导致 `Agent not found: "pi-ygg-harness"`。
 *
 * 注意：黑名单是「双份维护」的准硬编码（Mod `render.lua` 一份、这里一份），
 * 单一来源的收敛见 {@link RESERVED_AGENT_IDS} 上的说明与 parity 测试。
 */
export function isResumableAgentId(
  id: string | undefined,
  fallbackActorId?: string,
): boolean {
  if (!id) {
    return false;
  }
  const trimmed = id.trim();
  if (trimmed === "" || RESERVED_AGENT_IDS.has(trimmed)) {
    return false;
  }
  if (fallbackActorId !== undefined && trimmed === fallbackActorId.trim()) {
    return false;
  }
  return true;
}

/**
 * 该不该 `resume` 一个既有 subagent，答案是「续谁」。
 *
 * - `job.executor` 非空且通过 {@link isResumableAgentId} → 同一阶段重派（`gate.reject` 后 rev+1）：
 *   续原执行者，它记得上一次为什么被打回；
 * - 否则 `plan` → `development` 时显式允许（`resumePlanAgent`）且 `planExecutor`
 *   为真实可续跑 ID → 续方案执行者（方案是它写的，实现时不必重新推导）；
 * - 否则若 `job.subagentAction?.resumeAgentId` 为真实可续跑 ID → 续该执行者；
 * - 否则 `""`（新起一个，绝不把 `"pi-ygg-harness"` 等兜底名传给 `resume`）。
 */
export function resolveResumeId(
  job: SummaryJob,
  options: { resumePlanAgent?: boolean; actorId?: string } = {},
): string {
  const actorId = options.actorId;
  const executor = (job.executor ?? "").trim();
  if (isResumableAgentId(executor, actorId)) {
    return executor;
  }
  if (options.resumePlanAgent === true && job.stage === "development") {
    const planExecutor = (job.planExecutor ?? "").trim();
    if (isResumableAgentId(planExecutor, actorId)) {
      return planExecutor;
    }
  }
  const coreResume = (job.subagentAction?.resumeAgentId ?? "").trim();
  if (isResumableAgentId(coreResume, actorId)) {
    return coreResume;
  }
  return "";
}

/**
 * `exec=` 段能用的 id：Mod 的 `parse_executor_value` 会在 `id` 含空白或超 64 字节时
 * **拒掉整条提案**，所以带空白的 id 一律退回默认执行者（宁可少一条账本，
 * 也不能让一条好提案因为一个怪 id 被拒）。`resume` 仍然用原值（它要精确匹配）。
 */
export function actorForLedger(agentId: string | undefined, fallback: string): string {
  const candidate = (agentId ?? "").trim();
  if (candidate === "" || /\s/.test(candidate) || candidate.length > 64) {
    return fallback;
  }
  return candidate;
}

/** 续跑理由（写进回执；「为什么续」比「续了」更有诊断价值）。 */
export function resumeReason(
  job: SummaryJob,
  resumePlanAgent: boolean,
  actorId?: string,
): string {
  const executor = (job.executor ?? "").trim();
  if (isResumableAgentId(executor, actorId)) {
    return `同阶段重派（rev ${job.rev}），续原执行者`;
  }
  if (
    resumePlanAgent &&
    job.stage === "development" &&
    isResumableAgentId(job.planExecutor, actorId)
  ) {
    return "plan → development 跨阶段续跑";
  }
  if (isResumableAgentId(job.subagentAction?.resumeAgentId, actorId)) {
    return `Core subagentAction 指定续跑（rev ${job.rev}）`;
  }
  return "新起一个 subagent";
}

// ───────────────────────────── 纯函数：泳道探针真值 ─────────────────────────────

/** 泳道探针的真值（`known=false` = 没读到，**不得**据此自愈）。 */
export interface LaneProbeVerdict {
  known: boolean;
  commits: number | null;
  clean: boolean | null;
}

export const UNKNOWN_LANE_PROBE: LaneProbeVerdict = { known: false, commits: null, clean: null };

export interface LaneProbeContext {
  job: SummaryJob;
  lane: number;
  branch: string;
  base: string;
  worktreePath: string | null;
  /** `tool.call git.lane_probe` 的回执。 */
  probeReceipt: StepReceipt;
  /** 探针之后那次 drain 的回执（Mod 的折叠叙事若在，就在这里）。 */
  drainReceipt: StepReceipt | null;
}

/**
 * 从叙事里读探针真值（`commits=<n>` / `clean=0|1`）。
 *
 * 取**最后**一次出现的值：同一回合里重跑探针时，后面的才是现在的局面。
 * 两条都读不到 → `known=false`（调用方必须当「不知道」处理）。
 */
export function parseLaneProbeNarrative(lines: readonly string[]): LaneProbeVerdict {
  let commits: number | null = null;
  let clean: boolean | null = null;
  for (const line of lines) {
    const text = String(line ?? "");
    const commitsMatch = /commits=(-?\d+)/.exec(text);
    if (commitsMatch !== null) {
      const parsed = Number(commitsMatch[1]);
      if (Number.isFinite(parsed) && parsed >= 0) {
        commits = Math.trunc(parsed);
      }
    }
    const cleanMatch = /clean=(0|1|true|false)/.exec(text);
    if (cleanMatch !== null) {
      clean = cleanMatch[1] === "1" || cleanMatch[1] === "true";
    }
  }
  return { known: commits !== null || clean !== null, commits, clean };
}

/** 探针说「没提交」：0 个提交或工作树不干净（两者都要人复核，都不阻塞门禁）。 */
export function laneProbeNeedsCommit(verdict: LaneProbeVerdict): boolean {
  if (!verdict.known) {
    return false;
  }
  return verdict.commits === 0 || verdict.clean === false;
}

/**
 * 本地只读 git 探测（决策输入，**不是**证据）。
 *
 * 实情：Mod 的 `lane.<k>.probe.*` 不在 `summary` 的暴露面里，宿主的 `summary` op
 * 也不回 drain 叙事，所以「探针真值」只能从两条路来：回执叙事（若宿主回）
 * 或直接问工作树。后者只读（`status --porcelain` / `rev-list --count`），
 * 用来决定「要不要催一次 git commit」；门禁与结算永远只看 Mod 的账本。
 */
export function readLaneProbeFromGit(worktreePath: string | null, base: string): LaneProbeVerdict {
  if (worktreePath === null || worktreePath === "") {
    return UNKNOWN_LANE_PROBE;
  }
  try {
    if (!fs.existsSync(worktreePath)) {
      return UNKNOWN_LANE_PROBE;
    }
    const run = (args: string[]) =>
      spawnSync("git", ["-C", worktreePath, ...args], { encoding: "utf8", timeout: 5000 });
    const status = run(["status", "--porcelain"]);
    if (status.error !== undefined || status.status !== 0) {
      return UNKNOWN_LANE_PROBE;
    }
    const clean = String(status.stdout ?? "").trim() === "";
    let commits: number | null = null;
    if (base !== "") {
      const count = run(["rev-list", "--count", `${base}..HEAD`]);
      if (count.error === undefined && count.status === 0) {
        const parsed = Number(String(count.stdout ?? "").trim());
        if (Number.isFinite(parsed)) {
          commits = Math.trunc(parsed);
        }
      }
    }
    return { known: true, commits, clean };
  } catch {
    return UNKNOWN_LANE_PROBE;
  }
}

/** 默认探针读者：叙事优先（Mod 的账本），读不到再退回本地 git。 */
function defaultReadLaneProbe(context: LaneProbeContext): LaneProbeVerdict {
  const fromNarrative = parseLaneProbeNarrative([
    ...(context.probeReceipt.narrative ?? []),
    ...(context.drainReceipt?.narrative ?? []),
  ]);
  if (fromNarrative.known) {
    return fromNarrative;
  }
  return readLaneProbeFromGit(context.worktreePath, context.base);
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
  lines.push(...buildMandatoryReferencesSection(workspace));
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

/**
 * 续跑既有 subagent 时的增量提示（纯函数，可单测）：只交代新阶段/新修订的增量要求与门禁。
 */
export function buildResumeDeltaPrompt(
  job: SummaryJob,
  worktreePath: string | null,
  artifactPath: string,
  cwd: string,
  options: {
    verification?: string[];
    lane?: number;
    branch?: string;
    base?: string;
  } = {},
): string {
  const stage = job.stage === "" ? "plan" : job.stage;
  const verification =
    options.verification !== undefined && options.verification.length > 0
      ? options.verification
      : DEFAULT_VERIFICATION;
  const workspace = worktreePath ?? cwd;
  const lines: string[] = [
    `# 需求 ${job.id} · ${stage} 阶段续跑增量指令（rev ${job.rev > 0 ? job.rev : 1}）`,
    "",
    `- 作用域根：\`${workspace}\``,
    ...(options.lane !== undefined && options.lane > 0 ? [`- 泳道：lane-${options.lane}`] : []),
    ...(options.branch !== undefined && options.branch !== "" ? [`- 分支：\`${options.branch}\``] : []),
    ...(options.base !== undefined && options.base !== "" ? [`- 基线 commit：\`${options.base}\``] : []),
    `- 标题：${job.title === "" ? job.id : job.title}`,
    `- 描述：${job.description.trim() === "" ? "(同上)" : job.description}`,
    `- 制品路径：\`${artifactPath}\``,
    "",
    ...buildBriefFooter(artifactPath, verification),
  ];
  return lines.join("\n");
}

function buildMandatoryReferencesSection(workspace: string): string[] {
  const contextDoc = workspace !== "" ? path.join(workspace, "docs/CONTEXT.md") : "docs/CONTEXT.md";
  const designDoc = workspace !== "" ? path.join(workspace, "docs/DESIGN.md") : "docs/DESIGN.md";
  return [
    "### 必读领域与架构文档（Mandatory References）",
    `- 提案或写代码前必须先读 \`docs/CONTEXT.md\`（领域知识、核心概念与不变量，路径：\`${contextDoc}\`）。`,
    `- 提案或写代码前必须先读 \`docs/DESIGN.md\`（架构设计、模块边界与接口契约，路径：\`${designDoc}\`）。`,
  ];
}

function buildBriefFooter(artifactPath: string, verification: readonly string[]): string[] {
  const lines: string[] = ["## 验证门禁（必须真跑，贴输出摘要）"];
  for (const command of verification) {
    lines.push(`- \`${command}\``);
  }
  lines.push("- 门禁失败时不要声称完成：报告里写清失败命令、首行错误、你已排除的方向。");
  lines.push("");
  lines.push("## 返回格式（协调者按这个形状归档）");
  lines.push("1. **做了什么**：文件路径 + 符号级改动清单（`path:line` 形式优先）。");
  lines.push("2. **验证结果**：每条门禁命令 + 结论（passed / failed + 摘要）；没跑就说没跑。");
  lines.push("3. **风险与遗留**：未覆盖的场景、你知道但没修的问题。");
  lines.push(`4. **制品**：正文或 \`${artifactPath}\` 的内容（协调者会把它写进本局 VFS）。`);
  lines.push("");
  lines.push("报告里禁止使用竖线 `|` 作为分隔符（它会被当成提案载荷的分段符）。");
  return lines;
}

/**
 * 若 Core 产出的 `brief` / `deltaPrompt` 尚未包含工作区边界、必读文档、验证门禁或返回格式尾注，按需补齐，
 * 保证 [`parseSubagentOutput`] 与制品归档在薄壳模式下同样稳健。
 */
export function ensureBriefVerificationAndFooter(
  corePrompt: string,
  artifactPath: string,
  verification: readonly string[],
  options: { worktreePath?: string | null; cwd?: string } = {},
): string {
  const trimmed = corePrompt.trim();
  const worktreePath = (options.worktreePath ?? "").trim();
  const workspace = worktreePath !== "" ? worktreePath : (options.cwd ?? "").trim();
  const missingWorktree = worktreePath !== "" && !trimmed.includes(worktreePath);
  const missingRefs =
    !trimmed.includes("docs/CONTEXT.md") || !trimmed.includes("docs/DESIGN.md");
  const missingCommands = verification.filter((cmd) => cmd.trim() !== "" && !trimmed.includes(cmd));
  const hasFooter = trimmed.includes(artifactPath) && (trimmed.includes("返回格式") || trimmed.includes("Return Format"));
  if (!missingWorktree && !missingRefs && missingCommands.length === 0 && hasFooter) {
    return corePrompt;
  }
  const sections: string[] = [trimmed];
  if (missingWorktree) {
    sections.push("", "## 工作区（硬边界）", `- **只在这个目录里工作**：\`${worktreePath}\``);
  }
  if (missingRefs) {
    sections.push("", ...buildMandatoryReferencesSection(workspace));
  }
  if (missingCommands.length > 0 || !hasFooter) {
    sections.push("", ...buildBriefFooter(artifactPath, verification));
  }
  return sections.join("\n");
}

/** 解析 subagent 输出为提案一行摘要与完整报告正文。 */
export function parseSubagentOutput(text: string): { summary: string; report: string } {
  const report = String(text ?? "").trim();
  return {
    summary: firstLine(report),
    report,
  };
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

function initialLaneFor(job: SummaryJob): number {
  if (job.lane > 0) {
    return job.lane;
  }
  const actionLane = job.subagentAction?.lane ?? 0;
  return actionLane > 0 ? actionLane : 0;
}

function selectCoreSubagentPrompt(
  job: SummaryJob,
  isResume: boolean,
  preferCoreBrief: boolean,
): string {
  if (!preferCoreBrief || job.subagentAction === undefined) {
    return "";
  }
  const action = job.subagentAction;
  if (isResume) {
    return (action.deltaPrompt ?? action.brief ?? action.fullFallbackBrief ?? "").trim();
  }
  return (action.brief ?? action.fullFallbackBrief ?? action.deltaPrompt ?? "").trim();
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
    runId = "",
    maxSelfHealRetries = DEFAULT_MAX_SELF_HEAL_RETRIES,
    logger,
  } = opts;
  const customVerification = opts.verificationCommands ?? opts.verification;
  const verification =
    customVerification !== undefined && customVerification.length > 0
      ? customVerification
      : DEFAULT_VERIFICATION;
  const preferCoreBrief = opts.preferCoreBrief !== false;
  const useInlineLaneProbe = opts.useInlineLaneProbe === true;

  const pressure = opts.checkContextPressure?.() ?? null;
  const readProbe = opts.readLaneProbe ?? defaultReadLaneProbe;
  const resumePlanAgent = opts.resumePlanAgent === true;
  const summary = await daemon.summary();
  const artifacts = await listArtifactPaths(daemon);

  /** 交接单入参（runId / 压力 / 制品列表三处都一样，集中一次以免漂移）。 */
  const handoffInput = (input: { summary: RunSummary; reason: string }): HandoffInput => {
    const out: HandoffInput = { runId, summary: input.summary, pressure, reason: input.reason };
    if (artifacts !== undefined) {
      out.artifacts = artifacts;
    }
    return out;
  };

  if (!summary.available) {
    return {
      jobs: [],
      gates: [],
      narrative: [],
      summary,
      contextBraked: false,
      pressure,
      degraded: summary.degraded ?? "摘要不可用（宿主没有给出 `meta.summary`）",
    };
  }

  // ── 硬刹车（已到硬水位）──
  // 一个字节都不派：先把「现在的局面」写成交接单，然后交给上层压缩 + 续跑。
  // 安全前提：此刻**没有在飞的 subagent**（本函数自己一个都还没起）。
  if (pressure !== null && pressure.level === "hard_brake" && pressure.shouldBrakeSubagents) {
    const handoff = await writeHandoffToVfs(
      daemon,
      handoffInput({ summary, reason: "hard_brake（已到硬水位，未起新一波 subagent）" }),
    );
    return {
      jobs: [],
      gates: summary.gates,
      narrative: [handoffNarrative(handoff)],
      summary,
      contextBraked: true,
      handoff,
      pressure,
    };
  }

  const active = summary.jobs.filter((job) => ACTIVE_STAGES.has(job.stage));
  if (active.length === 0) {
    return {
      jobs: [],
      gates: summary.gates,
      narrative: [],
      summary,
      contextBraked: false,
      pressure,
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
    let lane = initialLaneFor(job);
    let branch = job.branch;
    let base = job.base;
    let claimed = false;
    let worktreePath: string | null = null;

    if (job.stage === "development") {
      const preclaimedWorktree = (job.worktreePath ?? job.subagentAction?.worktreePath ?? "").trim();
      if (preclaimedWorktree !== "") {
        // Core 在 requirement.dispatch / gate.approve 期间已同步调用 git.worktree_claim 绑好 worktree：零往返直接复用。
        worktreePath = preclaimedWorktree;
        claimed = true;
        if (lane === 0) {
          const match = /-lane-(\d+)$/.exec(preclaimedWorktree);
          const fallbackLane = pickFreeLane(ledger, maxLanes, job.id) ?? 1;
          lane = match !== null ? Number(match[1]) : fallbackLane;
        }
        if (branch === "") {
          branch = branchNameFor(job);
        }
        base = base !== "" ? base : String(await resolveBase(job));
        ledger.push({ ...job, lane, branch, base, worktreePath });
      } else if (lane === 0 || branch === "") {
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
        worktreePath = lane > 0 ? worktreePathFor(worktreeRoot, repoDir, lane) : null;
        ledger.push({ ...job, lane, branch, base });
      } else {
        base = base !== "" ? base : String(await resolveBase(job));
        worktreePath = lane > 0 ? worktreePathFor(worktreeRoot, repoDir, lane) : null;
      }
    } else if (lane > 0) {
      base = base !== "" ? base : String(await resolveBase(job));
      worktreePath = worktreePathFor(worktreeRoot, repoDir, lane);
    }

    const artifactPath = artifactPathFor(job, artifactPrefix);
    const resumeId = resolveResumeId(job, { resumePlanAgent, actorId });
    const isResume = resumeId !== "";
    const corePrompt = selectCoreSubagentPrompt(job, isResume, preferCoreBrief);
    const brief =
      corePrompt !== ""
        ? ensureBriefVerificationAndFooter(corePrompt, artifactPath, verification, {
            worktreePath,
            cwd,
          })
        : buildSubagentBrief(job, worktreePath, artifactPath, cwd, {
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
  // 每个作业先算「续谁」：同阶段重派续原执行者（它记得上次为什么被打回），
  // plan → development 显式允许时续方案执行者；拿不到 id 就新起一个。
  const outputs = await Promise.all(
    prepared.map(async (item) => {
      const request: SubagentRequest = {
        description: `${item.job.id} ${item.job.stage}`,
        prompt: item.brief,
        ...(opts.model === undefined ? {} : { model: opts.model }),
        ...(opts.thinking === undefined ? {} : { thinking: opts.thinking }),
      };
      const resumeId = resolveResumeId(item.job, { resumePlanAgent, actorId });
      if (resumeId !== "") {
        request.resume = resumeId;
      }
      logWith(
        logger,
        `[subagent] ${item.job.id} ${item.job.stage} → ${
          resumeId !== "" ? `续跑 ${resumeId}` : "新派发"
        }` + (item.worktreePath !== null ? ` (worktree=${item.worktreePath})` : ""),
      );
      const attempt = await runSubagent(executeSubagent, request, maxSelfHealRetries);
      const notes: string[] = [];
      if (resumeId !== "") {
        notes.push(
          `[RESUME] ${item.job.id} ${item.job.stage}：${resumeReason(item.job, resumePlanAgent, actorId)}` +
            `（resume=${resumeId}）`,
        );
      }
      if (attempt.retries > 0) {
        const used = attempt.agentId ?? resumeId;
        notes.push(
          `[SELF-HEAL] ${item.job.id} subagent 报告失败，已自动续跑 ${attempt.retries} 次` +
            `（resume=${used === "" ? "?" : used}）`,
        );
      }
      return {
        item,
        agentId: attempt.agentId,
        report: attempt.text ?? "",
        error: attempt.error,
        notes,
      };
    }),
  );

  // ── ②③ 归档 / 探针 / 提案：串行（Mod 是单写者，回执序才可复核） ──
  const jobs: OrchestrationJobResult[] = [];
  const narrative: string[] = [];

  for (const output of outputs) {
    const { item } = output;
    const receipts = item.receipts;
    let report = output.report.trim();
    let error = output.error;
    // 自愈预算按作业计：流断开一次、脏工作树一次，再多就是真 bug（不刷重试）。
    let selfHealBudget = maxSelfHealRetries;
    for (const note of output.notes) {
      receipts.push({ label: `dispatch note (${item.job.id})`, receipt: noteReceipt(note) });
    }

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

    let archived = await daemon.vfsPut(item.artifactPath, report);
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

    if (item.job.stage === "development" && item.lane > 0 && !useInlineLaneProbe) {
      const probeArgs = buildLaneProbeArgs({ lane: item.lane, base: item.base, branch: item.branch });
      let probeReceipt = await daemon.dispatch("tool.call", `git.lane_probe|${probeArgs}`);
      receipts.push({
        label: `tool.call git.lane_probe (${item.job.id} lane-${item.lane})`,
        receipt: probeReceipt,
      });
      // 探针是**警告面**（commits=0 / clean=0 不阻塞门禁），但要 drain 一次让
      // 冲突雷达与 `lane.<k>.probe.*` 落库；drain 回执里若带折叠叙事，探针真值
      // 从那里读，读不到再退回本地 git 只读探测（读不到就什么都不做）。
      let drainReceipt = await drainInbox(daemon);
      let verdict = await readProbe({
        job: item.job,
        lane: item.lane,
        branch: item.branch,
        base: item.base,
        worktreePath: item.worktreePath,
        probeReceipt,
        drainReceipt,
      });
      // 脏工作树自愈（只一次）：subagent 忘了 `git commit` 时探针会报
      // `commits=0` / `clean=0`；续跑**同一个** subagent 要求补提交，再复跑探针。
      if (
        laneProbeNeedsCommit(verdict) &&
        isResumableAgentId(output.agentId, actorId) &&
        selfHealBudget > 0
      ) {
        selfHealBudget -= 1;
        const nudge = await safeExecute(executeSubagent, {
          description: `${item.job.id} ${item.job.stage} (补提交)`,
          prompt: COMMIT_NUDGE_TEXT,
          ...(opts.model === undefined ? {} : { model: opts.model }),
          ...(opts.thinking === undefined ? {} : { thinking: opts.thinking }),
          resume: output.agentId,
        });
        receipts.push({
          label: `self-heal commit (${item.job.id})`,
          receipt: noteReceipt(
            `[SELF-HEAL] ${item.job.id} 探针报 commits=${verdict.commits ?? "?"} ` +
              `clean=${verdict.clean === null ? "?" : verdict.clean ? "1" : "0"}：` +
              `${nudge.ok === false ? "续跑失败" : "已续跑要求补提交"}（resume=${output.agentId}）`,
          ),
        });
        if (nudge.ok !== false && nudge.text.trim() !== "") {
          report = `${report}\n\n## 自愈补提交\n\n${nudge.text.trim()}`;
          const reArchived = await daemon.vfsPut(item.artifactPath, report);
          if (reArchived.ok) {
            archived = reArchived;
          }
        }
        probeReceipt = await daemon.dispatch("tool.call", `git.lane_probe|${probeArgs}`);
        receipts.push({
          label: `tool.call git.lane_probe retry (${item.job.id} lane-${item.lane})`,
          receipt: probeReceipt,
        });
        drainReceipt = await drainInbox(daemon);
        const afterVerdict = await readProbe({
          job: item.job,
          lane: item.lane,
          branch: item.branch,
          base: item.base,
          worktreePath: item.worktreePath,
          probeReceipt,
          drainReceipt,
        });
        receipts.push({
          label: `lane probe verdict (${item.job.id})`,
          receipt: noteReceipt(
            `[LANE PROBE] ${item.job.id} 补提交后 commits=${afterVerdict.commits ?? "?"} ` +
              `clean=${afterVerdict.clean === null ? "?" : afterVerdict.clean ? "1" : "0"}`,
          ),
        });
        verdict = afterVerdict;
      }
    }

    // 执行者账本写**真实** subagent id（不是扩展自己的名字）：下一次 re-dispatch
    // 的 `resume` 就靠它；Mod 的 `settings.executor.<rN>.<stage>` 也靠它可核对。
    const parsedOutput = parseSubagentOutput(report);
    const submitActor = actorForLedger(output.agentId, actorId);
    const submitPayload = buildProposalSubmitPayload({
      job: item.job,
      summary: `auto ${item.job.stage}: ${parsedOutput.summary}`,
      artifactPath: item.artifactPath,
      actorId: submitActor,
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

    const wtStatus =
      item.job.worktreeStatus ??
      (item.worktreePath !== null && item.worktreePath !== ""
        ? item.worktreePath
        : item.job.stage === "development"
          ? "待创建"
          : "主工作区(只读)");
    const usedAgentId = isResumableAgentId(output.agentId, actorId)
      ? output.agentId
      : resolveResumeId(item.job, { resumePlanAgent, actorId }) || undefined;
    const subStatus =
      item.job.subagentStatus ??
      (usedAgentId !== undefined ? `已派发 ${usedAgentId}` : "待派发");
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
      ...(usedAgentId !== undefined ? { agentId: usedAgentId } : {}),
      worktreeStatus: wtStatus,
      subagentStatus: subStatus,
      ...(item.job.probeStatus !== undefined ? { probeStatus: item.job.probeStatus } : {}),
    };
    if (error !== undefined) {
      jobResult.error = error;
    }
    jobs.push(jobResult);
  }

  const after = await daemon.summary();
  const touched = new Set(jobs.map((job) => job.id));
  const gates = (after.available ? after.gates : []).filter((gate) => touched.has(gate.id));

  // ── 软刹车：本波已经落地（提案 accept 完 → 停在 plan_gate / accept_gate 这个
  // **稳定节点**），现在把局面写成交接单，交给上层压缩 + 续跑。硬刹车在函数开头
  // 已经交接过一次，这里不重复写（同一波压两次只会白烧 token）。
  let handoff: HandoffWrite | undefined;
  if (pressure !== null && pressure.shouldBrakeSubagents && pressure.level !== "hard_brake") {
    const landed = after.available ? after : summary;
    handoff = await writeHandoffToVfs(
      daemon,
      handoffInput({
        summary: landed,
        reason: `soft_brake（${formatPercent(pressure.ratio)}）本波已落地`,
      }),
    );
    narrative.push(handoffNarrative(handoff));
  }

  const result: OrchestrationResult = {
    jobs,
    gates,
    narrative,
    summary: after.available ? after : summary,
    contextBraked: handoff !== undefined,
    pressure,
  };
  if (handoff !== undefined) {
    result.handoff = handoff;
  }
  return result;
}

/** 一行回执（自愈/续跑这类「说明性」事件用；不走 daemon，但同构）。 */
function noteReceipt(line: string): StepReceipt {
  return { ok: true, narrative: [line], pendingChoice: null, tools: [], raw: {} };
}

/** 交接单的叙事行（成功报字节数 + sha 前缀，失败原样贴原因）。 */
function handoffNarrative(write: HandoffWrite): string {
  if (!write.ok) {
    return `[HANDOFF REFUSED] ${write.path}：${write.error ?? "未知原因"}`;
  }
  return (
    `[HANDOFF] ${write.path} ← ${write.bytes !== undefined ? write.bytes : write.markdown.length} bytes` +
    (write.sha256 !== undefined ? ` sha256=${write.sha256.slice(0, 12)}` : "")
  );
}

/** 本局制品列表（拿不到就返回 `undefined`，交接单会自己补上自己那一条）。 */
async function listArtifactPaths(daemon: OrchestratorDaemon): Promise<string[] | undefined> {
  const lister = daemon.vfsList;
  if (lister === undefined) {
    return undefined;
  }
  try {
    const listed = await lister.call(daemon);
    return listed.ok ? listed.artifacts : undefined;
  } catch {
    return undefined;
  }
}

/** drain 一次（探针折叠叙事若在，就在这一次里）；daemon 不提供 → `null`。 */
async function drainInbox(daemon: OrchestratorDaemon): Promise<StepReceipt | null> {
  if (daemon.drain !== undefined) {
    try {
      return await daemon.drain();
    } catch {
      return null;
    }
  }
  await daemon.summary();
  return null;
}

/** 调一次 subagent；抛错也归一成 `ok=false`（调用方只判一个形状）。 */
async function safeExecute(
  executeSubagent: ExecuteSubagent,
  request: SubagentRequest,
): Promise<SubagentResult> {
  try {
    const result = await executeSubagent(request);
    const out: SubagentResult = { text: result.text ?? "", ok: result.ok !== false };
    if (result.agentId !== undefined && result.agentId !== "") {
      out.agentId = result.agentId;
    }
    if (out.ok === false) {
      out.error = result.error ?? "subagent 报告失败";
      out.text = result.text ?? "";
    }
    return out;
  } catch (error) {
    return {
      text: "",
      ok: false,
      error: error instanceof Error ? error.message : String(error),
    };
  }
}

/**
 * 带自愈的派活：失败一次就 `resume` 同一个 subagent 重试（`limit` 次）。
 * 续跑的前提是**知道续谁**（`agentId` 或调用方已给的 `resume`）；两者都没有时
 * 不重试——写一个猜出来的 id 等于新起一个，会把「自愈」变成「无痕重跑」。
 */
async function runSubagent(
  executeSubagent: ExecuteSubagent,
  request: SubagentRequest,
  limit: number,
): Promise<{ text: string; agentId?: string; error?: string; retries: number }> {
  let attempt = await safeExecute(executeSubagent, request);
  let retries = 0;
  while (attempt.ok === false && retries < Math.max(0, Math.trunc(limit))) {
    const resume = attempt.agentId ?? request.resume;
    if (!isResumableAgentId(resume)) {
      break;
    }
    retries += 1;
    attempt = await safeExecute(executeSubagent, { ...request, resume });
  }
  const out: { text: string; agentId?: string; error?: string; retries: number } = {
    text: attempt.text ?? "",
    retries,
  };
  if (attempt.agentId !== undefined && attempt.agentId !== "") {
    out.agentId = attempt.agentId;
  }
  if (attempt.ok === false) {
    out.error = attempt.error ?? "subagent 报告失败";
  }
  return out;
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
