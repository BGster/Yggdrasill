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
import type {
  RunSummary,
  StepReceipt,
  SummaryGate,
  SummaryJob,
  YggSubagentAction,
} from "./daemon.ts";
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
   *
   * `true`（默认）：读 Core 的 brief，adapter 只补缺（工作区边界 / 门禁尾注 / 契约锚线）。
   * `false`：**显式的旧 Mod 兼容降级**，不是静默默认——此时本次 brief 由 adapter 本地
   * 拼装、Core 的 `subagentAction.brief` **未被消费**，回执会写 `[ADAPTER NOTE]` 且
   * `OrchestrationResult.degraded` 非空，让上层看见。
   *
   * ⚠ 设为 `false` 不能用来「绕过契约」：本地拼装路径同样要满足 C1 的两条锚线
   * （`Mandatory references:` / `Submit:`），否则会被派活前的闸门拒绝。
   */
  preferCoreBrief?: boolean;
  /**
   * 契约策略覆盖（默认取 {@link DELTA_ASSESSABLE} 与「要求续跑承接段」）。
   * 这是一条给**口径升级**与测试用的缝：把 `deltaAssessable` 改 `true` 前，
   * 必须先给 Mod 的 `subagent_delta` 模板补齐锚线，否则放开是空转。
   */
  adapterContract?: AdapterContractPolicy;
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
 * 这串常量与 Mod 侧 `.ygg/mods/agent-seed/src/render.lua` 的
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
  // 契约锚线（逐字，行首）：与 Mod `data/scenes.json` 的 `subagent_brief` 模板同形。
  // `checkSubagentActionConsumption` 只认行首前缀，所以本地拼装路径**必须**产出这两行，
  // 否则「旧 Mod 兼容路径」会自己违反 C1（见 docs/DESIGN.md §1.2）。
  lines.push(
    "Mandatory references: read docs/CONTEXT.md (domain invariants) and docs/DESIGN.md (architecture & contracts) before proposing or editing.",
  );
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
  // 契约锚线（逐字，行首）：本地拼装路径必须与 Core 模板产出**同形状**的 brief，
  // 否则 C1 会把它自己判死（见 docs/DESIGN.md §1.2）。
  lines.push(buildSubmitLine(job, artifactPath));
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
    ...buildBriefFooter(artifactPath, verification, job),
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

/**
 * 本地拼装的「提交行」：`proposal.submit` 的载荷形状（位次固定，见
 * [`buildProposalSubmitPayload`]）。
 *
 * 这一行的存在意义是**契约同形**：Mod 的 `subagent_brief` 模板带 `Submit: {{submit}}`，
 * 而 adapter 本地拼装的 brief 也必须带 `Submit: …`，否则
 * [`checkSubagentActionConsumption`] 会把「旧 Mod 兼容路径」判成违规——
 * 两条路产出同形状的 brief 才是这一轮真正想要的东西。
 */
export function buildSubmitLine(job: SummaryJob, artifactPath: string): string {
  const rev = job.rev > 0 ? job.rev : 1;
  const stage = job.stage === "" ? "plan" : job.stage;
  const exec = "exec=<agent_id>";
  return (
    `Submit: \`.ygg/bin/egg --mod .ygg/mods/agent-seed --runs-root .ygg/runs ` +
    `--resume --event proposal.submit --payload '"${job.id}|${rev}|${stage}|<summary>|` +
    `${artifactPath}||${exec}"'\``
  );
}

function buildBriefFooter(
  artifactPath: string,
  verification: readonly string[],
  job?: SummaryJob,
): string[] {
  const lines: string[] = [];
  if (job !== undefined) {
    lines.push(buildSubmitLine(job, artifactPath));
    lines.push("");
  }
  lines.push("## 验证门禁（必须真跑，贴输出摘要）");
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
 *
 * 同时也补齐**契约锚线**（`Mandatory references:` 行与 `Submit:` 行，见
 * [`checkSubagentActionConsumption`]）：Core 的 brief 模板本来就带这两行，但旧版
 * Mod 或自定义 brief 可能没有，而「缺锚线」在派活前是硬拒绝——补在这里比让
 * 整次派活失败更合理（补齐的是**缺的那一行**，不是替它编整份 brief）。
 *
 * @param job 给了就补 `Submit:` 行（载荷要用 `job.id` / `rev` / `stage`）
 */
export function ensureBriefVerificationAndFooter(
  corePrompt: string,
  artifactPath: string,
  verification: readonly string[],
  options: {
    worktreePath?: string | null;
    cwd?: string;
    job?: SummaryJob;
  } = {},
): string {
  const trimmed = corePrompt.trim();
  const worktreePath = (options.worktreePath ?? "").trim();
  const workspace = worktreePath !== "" ? worktreePath : (options.cwd ?? "").trim();
  const missingWorktree = worktreePath !== "" && !trimmed.includes(worktreePath);
  const missingRefs =
    !trimmed.includes("docs/CONTEXT.md") || !trimmed.includes("docs/DESIGN.md");
  const missingAnchor = !hasLineStartAnchor(trimmed, MANDATORY_REFERENCES_ANCHOR);
  const missingSubmit = options.job !== undefined && !hasSubmitAnchor(trimmed);
  const missingCommands = verification.filter((cmd) => cmd.trim() !== "" && !trimmed.includes(cmd));
  const hasFooter = trimmed.includes(artifactPath) && (trimmed.includes("返回格式") || trimmed.includes("Return Format"));
  if (
    !missingWorktree &&
    !missingRefs &&
    !missingAnchor &&
    !missingSubmit &&
    missingCommands.length === 0 &&
    hasFooter
  ) {
    return corePrompt;
  }
  const sections: string[] = [trimmed];
  if (missingWorktree) {
    sections.push("", "## 工作区（硬边界）", `- **只在这个目录里工作**：\`${worktreePath}\``);
  }
  if (missingRefs) {
    sections.push("", ...buildMandatoryReferencesSection(workspace));
  }
  if (missingAnchor) {
    // 逐字锚线与 Mod 的 `subagent_brief` 模板同形（行首、独立成行）。
    sections.push(
      "",
      "Mandatory references: read docs/CONTEXT.md (domain invariants) and docs/DESIGN.md (architecture & contracts) before proposing or editing.",
    );
  }
  if (missingSubmit && options.job !== undefined) {
    sections.push("", buildSubmitLine(options.job, artifactPath));
  }
  if (missingCommands.length > 0 || !hasFooter) {
    sections.push("", ...buildBriefFooter(artifactPath, verification, options.job));
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

// ───────────────────────────── 纯函数：adapter 一致性契约（C1–C5） ─────────────────────────────

/**
 * 契约锚线 ①：`Mandatory references:` —— **行首前缀**命中。
 *
 * 为什么是行首而不是 `includes`：`job.description` 是多行需求正文，正文里完全可能
 * 出现同样的字样。用任意 `includes` 判定会被一句话绕过（自指类断言的经典空转），
 * 所以锚线只认「自己占一行、且以它开头」。
 */
export const MANDATORY_REFERENCES_ANCHOR = "Mandatory references:";

/** 契约锚线 ②：`Submit:` —— 行首前缀命中 **且后面有非空内容**。 */
const SUBMIT_ANCHOR = /^Submit:\s*\S/;

/** 契约锚线 ③：续跑承接段标题（(c) 的等价制品义务）。 */
export const RESUME_HANDOVER_HEADING = "## 续跑承接（Resume Handover）";

export interface AdapterContractViolation {
  code:
    | "brief-missing-mandatory-references"
    | "brief-missing-submit"
    | "brief-missing-resume-handover"
    | "delta-unassessable"
    | "no-core-action";
  /** 一条人读句子，原样进回执（`[ADAPTER CONTRACT] …`）。 */
  message: string;
}

/**
 * deltaPrompt 的判定口径策略表（**先钉死口径，否则断言是空的**）。
 *
 * 当前 `data/scenes.json` 的 `subagent_delta` 模板是
 * `Resume {{id}} (rev {{rev}}) for stage ... ; write {{artifact}} and submit.`，
 * 它**不含**任何可判定锚线。于是「deltaPrompt 是否满足 C1」的答案完全取决于一条
 * 没写下来的口径——这是最危险的假阳性来源。
 *
 * 本轮选择 **fail-closed 而不猜**：`assessable: false` 的 delta 不得单独作为 prompt，
 * adapter 必须退回 `brief`。等 Mod 侧给 `subagent_delta` 补上锚线后，
 * 把这里改成 `true` 即可放开（那一处改动同时由 Mod 侧的 brief 断言盯住）。
 */
export const DELTA_ASSESSABLE: { assessable: boolean; reason: string } = {
  assessable: false,
  reason: "subagent_delta 模板不含可判定锚线（Mandatory references / Submit:）",
};

/** 契约策略的可选覆盖（测试与未来的口径升级共用一条缝，不改调用点）。 */
export interface AdapterContractPolicy {
  /** deltaPrompt 是否可单独判定为合格的 prompt（默认取 {@link DELTA_ASSESSABLE}）。 */
  deltaAssessable?: boolean;
  /** 是否要求 `## 续跑承接（Resume Handover）` 段（默认 `true`）。 */
  requireResumeHandover?: boolean;
  /**
   * 允许「Core 没有 `subagentAction`、adapter 本地拼 brief」这条**旧 Mod 兼容**路径
   * （默认 `false` = `no-core-action` 违规）。
   *
   * 为什么默认禁止：Core 已经产出作业时 adapter 自拼 brief 正是 (a) 的原始症状。
   * 旧 Mod（没有 `subagentAction`）需要它，但此时**锚线仍然要满足**，且调用方必须
   * 在 `degraded` 里写明「本地拼装」，不能静默。
   */
  allowAdapterLocalBrief?: boolean;
}

/** 行首前缀命中（忽略行首空白，但**不允许**出现在行中间）。 */
function hasLineStartAnchor(text: string, anchor: string): boolean {
  for (const line of String(text ?? "").split(/\r?\n/)) {
    if (line.trimStart().startsWith(anchor)) {
      return true;
    }
  }
  return false;
}

function hasSubmitAnchor(text: string): boolean {
  for (const line of String(text ?? "").split(/\r?\n/)) {
    if (SUBMIT_ANCHOR.test(line.trimStart())) {
      return true;
    }
  }
  return false;
}

/**
 * C1 / C2 / C5：**brief 不是背景材料，是可断言的派发载荷**。
 *
 * 派活前（`orchestrateActiveJobs` 的「检查 → 落盘 → spawn」三段的第一段）调它：
 * 返回非空即**不得 spawn**，回执写 `[ADAPTER CONTRACT] …`，结果进 `degraded`。
 *
 * 判定顺序与「为什么这样排」：
 * 1. `action === undefined` → `no-core-action`：Core 已经产出作业时 adapter **不得**
 *    自行拼 brief（这正是 (a) 的原始症状：brief 在 `summary` 里明明可见，却没人消费）。
 *    调用方可以显式豁免（`policy.allowAdapterLocalBrief`，用于**旧 Mod 兼容**路径），
 *    豁免时降级信号走 `degraded`，而不是静默。
 * 2. 存在 delta 且策略判其不可判定、而 prompt 恰好就是那份 delta → `delta-unassessable`。
 * 3. 两条锚线（`Mandatory references:` / `Submit:`）与续跑承接段逐条检查。
 *
 * @param action   Core 产出的 `job.subagentAction`（旧 Mod 没有 → `undefined`）
 * @param prompt   实际要发给 subagent 的 prompt（**逐字**，含 adapter 补齐的尾注）
 */
export function checkSubagentActionConsumption(
  action: YggSubagentAction | undefined,
  prompt: string,
  policy: AdapterContractPolicy = {},
): AdapterContractViolation[] {
  const violations: AdapterContractViolation[] = [];
  const text = String(prompt ?? "");
  if (action === undefined) {
    if (policy.allowAdapterLocalBrief !== true) {
      violations.push({
        code: "no-core-action",
        message:
          "Core 没有产出 subagentAction，但 adapter 仍要派活：brief 由适配器本地拼装，" +
          "Core 的派发载荷未被消费",
      });
      return violations;
    }
    // 旧 Mod 兼容豁免：锚线仍然要满足（否则本地拼装路径会自己违反契约）。
    pushAnchorViolations(violations, text, policy);
    return violations;
  }

  const delta = String(action.deltaPrompt ?? "").trim();
  const deltaAssessable = policy.deltaAssessable ?? DELTA_ASSESSABLE.assessable;
  if (delta !== "" && !deltaAssessable && sameText(text, delta)) {
    violations.push({
      code: "delta-unassessable",
      message:
        `deltaPrompt 不可单独判定（${DELTA_ASSESSABLE.reason}）：` +
        "adapter 必须退回 brief / fullFallbackBrief",
    });
  }
  pushAnchorViolations(violations, text, policy);
  return violations;
}

/** 三条锚线（`Mandatory references:` / `Submit:` / 续跑承接段）逐条判定。 */
function pushAnchorViolations(
  violations: AdapterContractViolation[],
  text: string,
  policy: AdapterContractPolicy,
): void {
  if (!hasLineStartAnchor(text, MANDATORY_REFERENCES_ANCHOR)) {
    violations.push({
      code: "brief-missing-mandatory-references",
      message:
        `brief 缺少行首锚线「${MANDATORY_REFERENCES_ANCHOR}」——` +
        "子代理不会被告知先读 docs/CONTEXT.md 与 docs/DESIGN.md",
    });
  }
  if (!hasSubmitAnchor(text)) {
    violations.push({
      code: "brief-missing-submit",
      message: "brief 缺少「Submit: <命令>」行（或该行为空）——子代理不知道该提交什么",
    });
  }
  if (policy.requireResumeHandover !== false && !text.includes(RESUME_HANDOVER_HEADING)) {
    violations.push({
      code: "brief-missing-resume-handover",
      message: `brief 缺少「${RESUME_HANDOVER_HEADING}」段——无 resume 时没有等价承接`,
    });
  }
}

/** 逐字比较（去掉首尾空白后完全相同）。 */
function sameText(left: string, right: string): boolean {
  return left.trim() === right.trim();
}

/** 续跑承接段的上下文（每一行都是**可从作业状态派生的事实**，不写主观叙述）。 */
export interface ResumeHandoverContext {
  /** 本次 prompt 的依据：Core 的哪一段，或 adapter 本地拼装。 */
  basis: string;
  /** 前任执行者（`""` → 本次为新起）。 */
  priorExecutor: string;
  /** 未落地清单（生产建议只用「本次 brief 里声明的门禁未全绿」这类可核对事实）。 */
  outstanding: readonly string[];
  /** 已归档制品（VFS 里已存在的 `artifacts/<rN>-<stage>.md`）。 */
  artifacts: readonly string[];
}

/**
 * C5：给 brief 收尾补 `## 续跑承接（Resume Handover）` 段（**幂等**：已存在原样返回）。
 *
 * 有什么用：换 harness / 换上下文窗口后，下一个执行者**没有会话 id 可续**，
 * 唯一能跨 resume 传递的就是这份制品。这一段把「我们没法 resume」变成 subagent
 * 可见的输入，而不是留成 adapter 心里的隐痛。
 *
 * **诚实说明**：这一段本身**不是** `resume` 的等价物（等价物是制品 + 门禁证据）。
 */
export function ensureResumeHandoverSection(
  prompt: string,
  context: ResumeHandoverContext,
): string {
  const text = String(prompt ?? "");
  if (text.includes(RESUME_HANDOVER_HEADING)) {
    return text;
  }
  const prior = context.priorExecutor.trim() === "" ? "(none — 本次为新起)" : context.priorExecutor.trim();
  const outstanding =
    context.outstanding.filter((line) => line.trim() !== "").length === 0
      ? ["(无：本次 brief 里声明的门禁在上一任已全绿)"]
      : context.outstanding.filter((line) => line.trim() !== "");
  const artifacts =
    context.artifacts.filter((line) => line.trim() !== "").length === 0
      ? ["(本局 VFS 暂无本作业制品)"]
      : context.artifacts.filter((line) => line.trim() !== "");
  const lines: string[] = [
    "",
    RESUME_HANDOVER_HEADING,
    `- basis: ${context.basis.trim() === "" ? "adapter-local" : context.basis.trim()}`,
    `- prior executor: ${prior}`,
    "- 未落地清单:",
    ...outstanding.map((line) => `  - ${line}`),
    "- 硬约束: 不读真实数据（`*.xlsx` / `*.xls` / `*.csv`、涉密挂载）；不操作 git 历史；" +
      "制品写 `artifacts/<rN>-<stage>.md`",
    "- 已归档制品:",
    ...artifacts.map((line) => `  - ${line}`),
  ];
  return `${text.replace(/\s+$/, "")}\n${lines.join("\n")}\n`;
}

/**
 * C3：实际使用的 prompt 落进 VFS 的路径 —— `artifacts/<rN>-<stage>-brief.md`。
 *
 * 无条件落盘（不区分 harness）：把义务写成条件分支，等于给未来的 harness
 * 留一个「我不知道自己算哪种」的空档。落盘换来的是**审计面**——任何一局都能
 * 事后核对「子代理到底收到了什么」。
 */
export function briefArtifactPathFor(
  job: SummaryJob,
  prefix = DEFAULT_ARTIFACT_PREFIX,
): string {
  const stage = job.stage === "" ? "plan" : job.stage;
  return `${prefix}/${job.id}-${stage}-brief.md`;
}

/**
 * 一份 brief 的「依据」（写进 `[BRIEF]` 回执与续跑承接段）：
 * 让回执能说清「这一次到底读了 Core 的哪一段」。
 */
export function briefBasis(
  action: YggSubagentAction | undefined,
  isResume: boolean,
  usedCore: boolean,
): string {
  if (!usedCore || action === undefined) {
    return "adapter-local";
  }
  if (isResume) {
    return action.deltaPrompt !== undefined && action.deltaPrompt.trim() !== ""
      ? "subagentAction.deltaPrompt"
      : "subagentAction.brief";
  }
  return "subagentAction.brief";
}

/**
 * 选 Core 的哪一段作为 prompt（**带依据**，供回执说明「为什么退」）。
 *
 * 与旧实现的差别：delta 不可判定时**显式退回 brief** 并给出退回理由，
 * 而不是把一份判不出合格的 delta 当成 prompt（那正是 DSH 实战里静默丢掉
 * `Mandatory references` 的路径）。
 */
export function selectCoreSubagentPrompt(
  job: SummaryJob,
  isResume: boolean,
  preferCoreBrief: boolean,
  policy: AdapterContractPolicy = {},
): { prompt: string; basis: string; fallbackNote: string } {
  if (!preferCoreBrief || job.subagentAction === undefined) {
    return { prompt: "", basis: "adapter-local", fallbackNote: "" };
  }
  const action = job.subagentAction;
  const deltaAssessable = policy.deltaAssessable ?? DELTA_ASSESSABLE.assessable;
  if (isResume && deltaAssessable) {
    const delta = (action.deltaPrompt ?? "").trim();
    if (delta !== "") {
      return { prompt: delta, basis: "subagentAction.deltaPrompt", fallbackNote: "" };
    }
  }
  const brief = (action.brief ?? action.fullFallbackBrief ?? "").trim();
  if (brief !== "") {
    const fellBack = isResume && (action.deltaPrompt ?? "").trim() !== "";
    return {
      prompt: brief,
      basis: "subagentAction.brief",
      fallbackNote: fellBack
        ? "[ADAPTER NOTE] deltaPrompt 缺少可判定锚线，已退回 brief"
        : "",
    };
  }
  const delta = (action.deltaPrompt ?? "").trim();
  if (delta !== "") {
    return { prompt: delta, basis: "subagentAction.deltaPrompt", fallbackNote: "" };
  }
  return { prompt: "", basis: "adapter-local", fallbackNote: "" };
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
  /** 本次 brief 读的是 Core 的哪一段（`adapter-local` = 本地拼装）。 */
  briefBasis: string;
  expectedGate: string;
  receipts: { label: string; receipt: StepReceipt }[];
  claimed: boolean;
  /** 契约闸门命中项（非空 → 本作业**不得 spawn**）。 */
  violations: AdapterContractViolation[];
  /** 派活前就要贴进回执的说明行（delta 退回 brief / preferCoreBrief=false …）。 */
  notes: string[];
}

/** 一次 brief 落盘的结果（叙事行进回执，`ok=false` 不影响派活但必须可见）。 */
export interface BriefWrite {
  jobId: string;
  path: string;
  ok: boolean;
  bytes: number;
  sha256?: string;
  narrative: string[];
}

/**
 * C3：把**实际使用的 prompt** 原样写进 `artifacts/<rN>-<stage>-brief.md`。
 *
 * 内容**不 trim、不追加、不改换行**——这样 `sha256` 就是「子代理收到的字节」的指纹，
 * 事后能核对「子代理到底收到了什么」。失败只降级（`[BRIEF REFUSED]`），不阻断派活。
 */
async function writeBriefArtifact(
  daemon: OrchestratorDaemon,
  item: PreparedJob,
  artifactPrefix: string,
): Promise<BriefWrite> {
  const briefPath = briefArtifactPathFor(item.job, artifactPrefix);
  const content = item.brief;
  let written: { ok: boolean; bytes?: number; sha256?: string; error?: string };
  try {
    written = await daemon.vfsPut(briefPath, content);
  } catch (error) {
    written = { ok: false, error: error instanceof Error ? error.message : String(error) };
  }
  if (!written.ok) {
    return {
      jobId: item.job.id,
      path: briefPath,
      ok: false,
      bytes: 0,
      narrative: [`[BRIEF REFUSED] ${briefPath}：${written.error ?? "未知原因"}`],
    };
  }
  const out: BriefWrite = {
    jobId: item.job.id,
    path: briefPath,
    ok: true,
    bytes: written.bytes ?? content.length,
    narrative: [
      `[BRIEF] ${briefPath} ← ${written.bytes ?? content.length} bytes` +
        (written.sha256 !== undefined ? ` sha256=${written.sha256.slice(0, 12)}` : "") +
        ` (basis=${item.briefBasis})`,
    ],
  };
  if (written.sha256 !== undefined) {
    out.sha256 = written.sha256;
  }
  return out;
}

/** 续跑承接段里的「已归档制品」提示：只报**本局 VFS 里确实存在**的路径。 */
function preparedArtifactHints(
  artifacts: readonly string[] | undefined,
  job: SummaryJob,
  artifactPath: string,
): string[] {
  const stage = job.stage === "" ? "plan" : job.stage;
  const briefPath = `${DEFAULT_ARTIFACT_PREFIX}/${job.id}-${stage}-brief.md`;
  const hints = [artifactPath, briefPath].filter(
    (candidate) => artifacts === undefined || artifacts.length === 0 ||
      artifacts.some((entry) => entry.startsWith(candidate)),
  );
  return hints;
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
  /**
   * 契约策略：默认「Core 没有 subagentAction 就是违规」。
   *
   * 唯一的豁免是**旧 Mod 兼容**——豁免**按作业**判定（该作业行确实没有
   * `subagentAction` 时才允许本地拼 brief），并且必须留痕：`degraded` 里写明这一次
   * brief 是本地拼的。
   *
   * 注意 `preferCoreBrief: false` **不是**豁免理由：Core 明明给了 `subagentAction`
   * 却被关掉，那正是 (a) 原样复活的路径，让它 spawn 等于把契约变回注释。
   */
  const contractPolicy: AdapterContractPolicy = { ...(opts.adapterContract ?? {}) };
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
    const selected = selectCoreSubagentPrompt(job, isResume, preferCoreBrief, contractPolicy);
    const corePrompt = selected.prompt;
    const built =
      corePrompt !== ""
        ? ensureBriefVerificationAndFooter(corePrompt, artifactPath, verification, {
            worktreePath,
            cwd,
            job,
          })
        : buildSubagentBrief(job, worktreePath, artifactPath, cwd, {
            verification,
            lane,
            branch,
            base,
            actorId,
          });
    // C5：每一份 brief 都以续跑承接段收尾（无 resume 时这是唯一的跨窗口承接）。
    const brief = ensureResumeHandoverSection(built, {
      basis: selected.basis,
      priorExecutor: resumeId,
      outstanding: [
        `本次 brief 声明的门禁尚未在本回执里复述为全绿：${verification.join(" / ")}`,
        `制品 \`${artifactPath}\` 需由本任写出；已有内容视为上一任的产出，先读再改`,
      ],
      artifacts: preparedArtifactHints(artifacts, job, artifactPath),
    });
    // C1 / C2：派活前的契约闸门（空 = 通过；非空 = 本作业**不得 spawn**）。
    // 旧 Mod 兼容（该作业行确实没有 `subagentAction`）时豁免 `no-core-action`，
    // 但三条锚线照样查——本地拼装路径也必须产出同形状的 brief。
    const jobPolicy: AdapterContractPolicy =
      job.subagentAction === undefined
        ? { ...contractPolicy, allowAdapterLocalBrief: true }
        : contractPolicy;
    const violations = checkSubagentActionConsumption(job.subagentAction, brief, jobPolicy);
    const notes: string[] = [];
    if (job.subagentAction === undefined) {
      notes.push(
        "[ADAPTER NOTE] 本作业行没有 subagentAction（旧 Mod）：brief 由适配器本地拼装，" +
          "已补齐 Mandatory references / Submit: 两条锚线与续跑承接段",
      );
    }
    if (selected.fallbackNote !== "") {
      notes.push(`${selected.fallbackNote}（${job.id} ${job.stage}）`);
    }
    if (preferCoreBrief === false) {
      notes.push(
        "[ADAPTER NOTE] preferCoreBrief=false：本次 brief 由适配器本地拼装，" +
          "Core 的 subagentAction.brief 未被消费（显式降级，不是默认路径）",
      );
    }
    prepared.push({
      job,
      lane,
      branch,
      base,
      worktreePath,
      artifactPath,
      brief,
      briefBasis: selected.basis,
      expectedGate,
      receipts,
      claimed,
      violations,
      notes,
    });
  }

  // ── ① 派活：独立作业并发（subagent 之间无共享状态） ──
  // 每个作业先算「续谁」：同阶段重派续原执行者（它记得上次为什么被打回），
  // plan → development 显式允许时续方案执行者；拿不到 id 就新起一个。
  //
  // 三段事务（语义是「先检查、再落盘、再 spawn」，避免派了活才发现载荷不合契约）：
  //   ①a 契约闸门：违规 → 不 spawn，回执 `[ADAPTER CONTRACT] …`，结果进 degraded；
  //   ①b brief 落盘：**任何** harness 都把实际 prompt 原样写进 `artifacts/<rN>-<stage>-brief.md`
  //      （无 Agent 工具的环境就靠它承接上下文；有 Agent 工具时它是审计面）；
  //   ①c spawn：只有 a 通过、b 成功（或明确降级）的作业才真的派出去。
  const briefWrites: BriefWrite[] = [];
  for (const item of prepared) {
    for (const note of item.notes) {
      item.receipts.push({
        label: `dispatch note (${item.job.id})`,
        receipt: noteReceipt(note),
      });
    }
    if (item.violations.length > 0) {
      for (const violation of item.violations) {
        item.receipts.push({
          label: `adapter contract (${item.job.id})`,
          receipt: noteReceipt(`[ADAPTER CONTRACT] ${item.job.id} ${item.job.stage}: ${violation.message}`),
        });
      }
      continue;
    }
    const write = await writeBriefArtifact(daemon, item, artifactPrefix);
    briefWrites.push(write);
    item.receipts.push({
      label: `vfs_put ${write.path} (${item.job.id} brief)`,
      receipt: noteReceipt(write.narrative),
    });
  }

  const blocked = new Set(prepared.filter((item) => item.violations.length > 0).map((item) => item.job.id));
  const briefRefused = new Set(briefWrites.filter((write) => !write.ok).map((write) => write.jobId));
  const dispatchable = prepared.filter((item) => !blocked.has(item.job.id));

  const outputs = await Promise.all(
    dispatchable.map(async (item) => {
      // 契约闸门拦下的作业根本不进这一层；brief 落盘失败**不阻断派活**（prompt 还在
      // 内存里，能派出去），但结论必须可见——见 writeBriefArtifact 的叙事。
      if (briefRefused.has(item.job.id)) {
        logWith(logger, `[brief] ${item.job.id} 落盘失败，仍按内存里那一份 prompt 派活`);
      }
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

  // 契约闸门拦下的作业：**没有 spawn、没有提案**，但要在结果里如实出现
  // （一行 `error` + 完整回执），否则上层只看到「少了一个作业」而不知道原因。
  for (const item of prepared.filter((row) => row.violations.length > 0)) {
    for (const entry of item.receipts) {
      for (const line of entry.receipt.narrative) {
        narrative.push(`[${item.job.id}] ${line}`);
      }
    }
    jobs.push({
      id: item.job.id,
      stage: item.job.stage,
      rev: item.job.rev,
      lane: item.lane,
      branch: item.branch,
      base: item.base,
      worktreePath: item.worktreePath,
      artifactPath: item.artifactPath,
      brief: item.brief,
      report: "",
      archived: { ok: false, error: "adapter contract violation" },
      receipts: item.receipts,
      expectedGate: item.expectedGate,
      error:
        `ADAPTER CONTRACT：brief 不满足派发义务（` +
        item.violations.map((violation) => violation.code).join(", ") +
        "），未派发 subagent",
    });
  }

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

  // 降级说明（**每一条都要让上层看见**）：契约拦截 / brief 落盘失败 / preferCoreBrief=false。
  // `degraded` 是 `OrchestrationResult` 上唯一的「本次不完美」通道，`formatOrchestration`
  // 首行就打印它——不写在这里的失败等于没人知道。
  const degradeNotes: string[] = [];
  if (blocked.size > 0) {
    degradeNotes.push(
      `[ADAPTER CONTRACT] ${[...blocked].join(", ")}：brief 不满足派发义务，未派发 subagent`,
    );
  }
  if (briefRefused.size > 0) {
    degradeNotes.push(`[BRIEF REFUSED] ${[...briefRefused].join(", ")}：brief 未能落进 VFS`);
  }
  if (preferCoreBrief === false) {
    degradeNotes.push("preferCoreBrief=false（未消费 Core brief，本次 brief 由适配器本地拼装）");
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
  if (degradeNotes.length > 0) {
    result.degraded = degradeNotes.join("；");
  }
  return result;
}

/** 一行（或多行）回执（自愈/续跑这类「说明性」事件用；不走 daemon，但同构）。 */
function noteReceipt(line: string | readonly string[]): StepReceipt {
  return {
    ok: true,
    narrative: typeof line === "string" ? [line] : [...line],
    pendingChoice: null,
    tools: [],
    raw: {},
  };
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
