/**
 * daemon.ts —— 常驻 `ygg-agent --daemon` 客户端的进程管理与请求队列
 *
 * 设计要点：
 * - **一个 runId 一个子进程**（`YggDaemon` 实例），行式 JSON-RPC 走
 *   stdin/stdout；`node:readline` 按行读取，按请求 `id` 配对。
 * - **请求串行化**：Mod 的状态是单写者的，客户端不能并发写同一条 stdin 队列；
 *   [`YggDaemon.request`] 内部把每个请求接到一条 promise 链上。
 * - **30s 超时 + 意外退出自动重启**：子进程退出时先拒绝在飞请求（不假装成功），
 *   下一次请求按需重启；重启失败/不支持 `--daemon` 时降级为**单发 CLI**。
 * - **凭据只在内存**：`readEnvFiles` 只解析 `.env` / `.env.local` 的键值，
 *   对外只暴露 `YGG_SUBAGENT_MODEL` / `YGG_SUBAGENT_THINKING` 两个值，
 *   其余键名一律不返回（日志里也只出现键名，不出现值）。
 *
 * 本模块**不 import** pi 的任何东西：`harness.test.ts` 可以直接跑（含一个用
 * `process.execPath -e` 当假 daemon 的行协议集成测试）。
 */

import { type ChildProcessWithoutNullStreams, spawn } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import * as readline from "node:readline";

// ───────────────────────────── 摘要（Mod 自报的结构化形状） ─────────────────────────────

/** Core `subagentAction` descriptor emitted by the Mod for an active job. */
export interface YggSubagentAction {
  action: "subagent.spawn" | "subagent.resume";
  reqId: string;
  rev: number;
  stage: string;
  lane: number;
  resumeAgentId?: string;
  worktreePath?: string;
  artifactPath?: string;
  brief?: string;
  deltaPrompt?: string;
  fullFallbackBrief?: string;
  submitCommand?: string;
}

/** Generic harness action emitted by Core (`subagent.spawn` / `subagent.resume` or extensible). */
export type YggHarnessAction =
  | YggSubagentAction
  | ({ action: string } & Record<string, unknown>);

/** 摘要里的作业行（`summary.jobs`）。旧 Mod 不给这个键 → 空数组。 */
export interface SummaryJob {
  id: string;
  rev: number;
  stage: string;
  status: string;
  phase: string;
  lane: number;
  branch: string;
  base: string;
  title: string;
  description: string;
  /**
   * 本阶段执行者 id（Mod 的 `settings.executor.<rN>.<stage>` 账本）。
   * 旧 Mod 不给这个键 → `""`（解析时补齐，编排据此决定「续跑原 subagent」）。
   */
  executor?: string;
  /** `plan` 阶段的执行者 id（跨阶段续跑：plan → development 用同一个 subagent）。 */
  planExecutor?: string;
  /** Core 已同步 claim 并绑定好的 worktree 绝对路径（非空时编排层跳过二次 claim）。 */
  worktreePath?: string;
  /** Core 为该作业预先生成的 subagent 派发指令（含 brief / deltaPrompt / resumeAgentId）。 */
  subagentAction?: YggSubagentAction;
}

export type YggActiveJob = SummaryJob;

/** 摘要里待审的门（`summary.gates`）。 */
export interface SummaryGate {
  id: string;
  gate: string;
  rev: number;
  title: string;
}

export interface SummaryConflict {
  id: string;
  peer: string;
  note: string;
}

/** `meta.summary` 的宿主侧形状（缺字段一律取保守默认值）。 */
export interface RunSummary {
  available: boolean;
  stage: string;
  ending: boolean;
  verifyPassed: boolean;
  nextEndsRun: boolean;
  pendingProposals: number;
  requirementCount: number;
  pendingTools: number;
  conflicts: SummaryConflict[];
  jobs: SummaryJob[];
  gates: SummaryGate[];
  /** Core 产出的结构化宿主动作列表（缺省时由 `jobs[].subagentAction` 合成）。 */
  harnessActions?: YggHarnessAction[];
  /** 降级原因（`available=false` 时说明为什么拿不到摘要）。 */
  degraded?: string;
  /** 原始摘要对象（诊断用；不做二次解释）。 */
  raw?: Record<string, unknown>;
}

/** 一次派发的回执（daemon 的 `dispatch` / 单发的 stdout JSON）。 */
export interface StepReceipt {
  ok: boolean;
  op?: string;
  event?: string;
  narrative: string[];
  pendingChoice: unknown;
  tools: unknown[];
  summary?: RunSummary;
  /** 本步产出的结构化宿主动作列表（取自顶层 `harnessActions` 或 `summary.harnessActions`）。 */
  harnessActions?: YggHarnessAction[];
  raw: Record<string, unknown>;
  /** 传输层/宿主层错误（`ok=false` 时给一句话）。 */
  error?: string;
}

export type YggReceipt = StepReceipt;

export interface DaemonOptions {
  /** 会话工作目录（`.ygg/bin/ygg-agent` 与 `.env` 相对它解析）。 */
  cwd: string;
  /** Mod 目录（默认 `<cwd>/.ygg/mods/pi-agent-seed`）。 */
  modDir?: string;
  runId: string;
  /** runs 根（默认 `<cwd>/.ygg/runs`）。 */
  runsRoot?: string;
  /** 可执行文件（默认 `<cwd>/.ygg/bin/ygg-agent`）。 */
  agentBin?: string;
  /** 覆盖 spawn 的可执行文件（测试用 `process.execPath`）。 */
  spawnBin?: string;
  /** 覆盖 spawn 的前置参数（测试用 `["-e", script]`）。 */
  spawnArgs?: string[];
  requestTimeoutMs?: number;
  /** 单发降级模式下单条命令的超时。 */
  cliTimeoutMs?: number;
  env?: NodeJS.ProcessEnv;
}

const DEFAULT_MOD_DIR = ".ygg/mods/pi-agent-seed";
const DEFAULT_RUNS_ROOT = ".ygg/runs";
const DEFAULT_REQUEST_TIMEOUT_MS = 30_000;
const DEFAULT_CLI_TIMEOUT_MS = 120_000;

// ───────────────────────────── .env 读取（只暴露两项） ─────────────────────────────

/** 解析 dotenv 文本为键值表（`#` 注释、`export ` 前缀、成对引号）。 */
export function parseEnvFile(text: string): Map<string, string> {
  const out = new Map<string, string>();
  for (const line of String(text ?? "").split(/\r?\n/)) {
    const trimmed = line.trim();
    if (trimmed === "" || trimmed.startsWith("#")) {
      continue;
    }
    const body = trimmed.startsWith("export ") ? trimmed.slice("export ".length).trim() : trimmed;
    const eq = body.indexOf("=");
    if (eq <= 0) {
      continue;
    }
    const key = body.slice(0, eq).trim();
    let value = body.slice(eq + 1).trim();
    if (value.length >= 2) {
      const first = value[0];
      const last = value[value.length - 1];
      if ((first === '"' && last === '"') || (first === "'" && last === "'")) {
        value = value.slice(1, -1);
      }
    }
    if (key !== "") {
      out.set(key, value);
    }
  }
  return out;
}

/** 逐层读取 `cwd` 下的 `.env` 与 `.env.local`（后者覆盖前者）。 */
export function readEnvFiles(cwd: string): Map<string, string> {
  const merged = new Map<string, string>();
  for (const name of [".env", ".env.local"]) {
    const file = path.join(cwd, name);
    let text = "";
    try {
      text = fs.readFileSync(file, "utf8");
    } catch {
      continue;
    }
    for (const [key, value] of parseEnvFile(text)) {
      merged.set(key, value);
    }
  }
  return merged;
}

/**
 * 子代理派发参数（**只有这两项**会被拿出来用；其余键名不返回、值不打印）。
 */
export interface SubagentSettings {
  model?: string;
  thinking?: string;
}

export function subagentSettings(env: Map<string, string>): SubagentSettings {
  const model = (env.get("YGG_SUBAGENT_MODEL") ?? "").trim();
  const thinking = (env.get("YGG_SUBAGENT_THINKING") ?? "").trim();
  const out: SubagentSettings = {};
  if (model !== "") {
    out.model = model;
  }
  if (thinking !== "") {
    out.thinking = thinking;
  }
  return out;
}

/** 环境变量清单的可安全打印形状：只有键名与长度。 */
export function describeEnvKeys(env: Map<string, string>): string[] {
  return [...env.entries()].map(([key, value]) => `${key}(${value.length})`).sort();
}

// ───────────────────────────── 摘要解析 ─────────────────────────────

function asRecord(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function asNumber(value: unknown, fallback = 0): number {
  if (typeof value === "number" && Number.isFinite(value)) {
    return Math.trunc(value);
  }
  if (typeof value === "string" && value.trim() !== "") {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) {
      return Math.trunc(parsed);
    }
  }
  return fallback;
}

function asString(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}

function asBool(value: unknown): boolean {
  return value === true;
}

function parseSubagentAction(
  value: unknown,
  fallback?: { id?: string; rev?: number; stage?: string; lane?: number },
): YggSubagentAction | undefined {
  const row = asRecord(value);
  if (row === null) {
    return undefined;
  }
  const action = asString(row.action);
  if (action !== "subagent.spawn" && action !== "subagent.resume") {
    return undefined;
  }
  const reqId = asString(row.reqId, asString(row.id, fallback?.id ?? ""));
  const out: YggSubagentAction = {
    action,
    reqId,
    rev: asNumber(row.rev, fallback?.rev ?? 1),
    stage: asString(row.stage, fallback?.stage ?? ""),
    lane: asNumber(row.lane, fallback?.lane ?? 0),
  };
  const resumeAgentId = asString(row.resumeAgentId).trim();
  if (resumeAgentId !== "") {
    out.resumeAgentId = resumeAgentId;
  }
  const worktreePath = asString(row.worktreePath).trim();
  if (worktreePath !== "") {
    out.worktreePath = worktreePath;
  }
  const artifactPath = asString(row.artifactPath).trim();
  if (artifactPath !== "") {
    out.artifactPath = artifactPath;
  }
  const brief = asString(row.brief);
  if (brief !== "") {
    out.brief = brief;
  }
  const deltaPrompt = asString(row.deltaPrompt);
  if (deltaPrompt !== "") {
    out.deltaPrompt = deltaPrompt;
  }
  const fullFallbackBrief = asString(row.fullFallbackBrief);
  if (fullFallbackBrief !== "") {
    out.fullFallbackBrief = fullFallbackBrief;
  }
  const submitCommand = asString(row.submitCommand).trim();
  if (submitCommand !== "") {
    out.submitCommand = submitCommand;
  }
  return out;
}

function parseHarnessActions(value: unknown, jobs: SummaryJob[] = []): YggHarnessAction[] {
  if (Array.isArray(value)) {
    const out: YggHarnessAction[] = [];
    for (const item of value) {
      const subagent = parseSubagentAction(item);
      if (subagent !== undefined) {
        out.push(subagent);
        continue;
      }
      const row = asRecord(item);
      if (row !== null) {
        const action = asString(row.action).trim();
        if (action !== "") {
          out.push({ ...row, action });
        }
      }
    }
    return out;
  }
  return jobs
    .map((job) => job.subagentAction)
    .filter((action): action is YggSubagentAction => action !== undefined);
}

function parseJobs(value: unknown): SummaryJob[] {
  if (!Array.isArray(value)) {
    return [];
  }
  const out: SummaryJob[] = [];
  for (const item of value) {
    const row = asRecord(item);
    if (row === null) {
      continue;
    }
    const id = asString(row.id);
    if (id === "") {
      continue;
    }
    const rev = asNumber(row.rev, 1);
    const stage = asString(row.stage);
    const lane = asNumber(row.lane, 0);
    const subagentAction = parseSubagentAction(row.subagentAction, { id, rev, stage, lane });
    const worktreePath = asString(row.worktreePath).trim() || subagentAction?.worktreePath || "";
    const job: SummaryJob = {
      id,
      rev,
      stage,
      status: asString(row.status),
      phase: asString(row.phase, asString(row.status)),
      lane,
      branch: asString(row.branch),
      base: asString(row.base),
      title: asString(row.title, id),
      description: asString(row.description),
      // 执行者账本：Mod 没给这个键时落 `""`（与「没登记」同义），而不是 `undefined`——
      // 编排层只判空串，不需要在两处分别处理两种「没有」。
      executor: asString(row.executor),
      planExecutor: asString(row.planExecutor),
    };
    if (worktreePath !== "") {
      job.worktreePath = worktreePath;
    }
    if (subagentAction !== undefined) {
      job.subagentAction = subagentAction;
    }
    out.push(job);
  }
  return out;
}

function parseGates(value: unknown): SummaryGate[] {
  if (!Array.isArray(value)) {
    return [];
  }
  const out: SummaryGate[] = [];
  for (const item of value) {
    const row = asRecord(item);
    if (row === null) {
      continue;
    }
    const id = asString(row.id);
    const gate = asString(row.gate);
    if (id === "" || gate === "") {
      continue;
    }
    out.push({ id, gate, rev: asNumber(row.rev, 1), title: asString(row.title, id) });
  }
  return out;
}

function parseConflicts(value: unknown): SummaryConflict[] {
  if (!Array.isArray(value)) {
    return [];
  }
  const out: SummaryConflict[] = [];
  for (const item of value) {
    const row = asRecord(item);
    if (row === null) {
      continue;
    }
    const note = asString(row.note).trim();
    if (note === "") {
      continue;
    }
    out.push({ id: asString(row.id), peer: asString(row.peer, "?"), note });
  }
  return out;
}

/** 未知摘要的安全值（`available=false`，调用方必须停下来而不是猜）。 */
export function unknownSummary(reason: string): RunSummary {
  return {
    available: false,
    stage: "",
    ending: false,
    verifyPassed: false,
    nextEndsRun: false,
    pendingProposals: 0,
    requirementCount: 0,
    pendingTools: 0,
    conflicts: [],
    jobs: [],
    gates: [],
    degraded: reason,
  };
}

/** `meta.summary` / 单发回执 / daemon 回执 → 统一摘要形状。 */
export function parseSummary(value: unknown, metaKey = "summary"): RunSummary {
  const row = asRecord(value);
  if (row === null) {
    return unknownSummary("回执里没有结构化摘要（`meta.summary` 缺失或不是对象）");
  }
  const jobs = parseJobs(row.jobs);
  const gates = parseGates(row.gates);
  const harnessActions = parseHarnessActions(row.harnessActions, jobs);
  const hasStage = typeof row.stage === "string" && row.stage !== "";
  return {
    available: true,
    stage: hasStage ? asString(row.stage) : "",
    ending: asBool(row.ending),
    verifyPassed: asBool(row.verifyPassed),
    nextEndsRun: asBool(row.nextEndsRun),
    pendingProposals: asNumber(row.pendingProposals),
    requirementCount: asNumber(row.requirementCount),
    pendingTools: asNumber(row.pendingTools),
    conflicts: parseConflicts(row.conflicts),
    jobs,
    gates,
    harnessActions,
    raw: row,
    ...(hasStage
      ? {}
      : { degraded: `摘要形状可用但 \`stage\` 为空（metaKey=${metaKey}）` }),
  };
}

/** 从任意回执对象里挖出摘要（daemon 直给 / `meta.<metaKey>` 两种都认）。 */
export function extractSummary(
  receipt: Record<string, unknown> | null,
  metaKey = "summary",
): RunSummary {
  if (receipt === null) {
    return unknownSummary("回执不是对象");
  }
  const topSummary = asRecord(receipt.summary);
  if (topSummary !== null) {
    const innerSummary = asRecord(topSummary.summary);
    return parseSummary(innerSummary ?? topSummary, metaKey);
  }
  const meta = asRecord(receipt.meta);
  if (meta !== null && asRecord(meta[metaKey]) !== null) {
    return parseSummary(meta[metaKey], metaKey);
  }
  return unknownSummary("回执里没有结构化摘要（`summary` / `meta.summary` 都缺）");
}

/** 回执 → [`StepReceipt`]（narrative 原样全量保留，**不截断**）。 */
export function toStepReceipt(
  value: unknown,
  metaKey = "summary",
  error?: string,
): StepReceipt {
  const row = asRecord(value) ?? {};
  const narrative = Array.isArray(row.narrative)
    ? row.narrative.filter((line): line is string => typeof line === "string")
    : [];
  const tools = Array.isArray(row.tools) ? row.tools : [];
  const ok = row.ok === undefined ? error === undefined : row.ok === true;
  const receipt: StepReceipt = {
    ok,
    narrative,
    pendingChoice: row.pendingChoice ?? null,
    tools,
    raw: row,
  };
  if (typeof row.op === "string") {
    receipt.op = row.op;
  }
  if (typeof row.event === "string") {
    receipt.event = row.event;
  }
  if (error !== undefined) {
    receipt.error = error;
  }
  const summary = extractSummary(row, metaKey);
  if (summary.available) {
    receipt.summary = summary;
  }
  const meta = asRecord(row.meta);
  const rawActions = row.harnessActions ?? meta?.harnessActions;
  if (Array.isArray(rawActions)) {
    receipt.harnessActions = parseHarnessActions(rawActions, summary.available ? summary.jobs : []);
  } else if (summary.available && summary.harnessActions !== undefined) {
    receipt.harnessActions = summary.harnessActions;
  }
  return receipt;
}

/** Alias for {@link toStepReceipt} — normalizes raw daemon / CLI receipt JSON into {@link StepReceipt}. */
export const normalizeReceipt = toStepReceipt;

/** 收据 → 给人看的文本（回执整段呈现：每一行 narrative 都保留）。 */
export function formatReceipt(label: string, receipt: StepReceipt): string {
  const lines: string[] = [`### ${label}${receipt.ok ? "" : " · FAILED"}`];
  if (receipt.error !== undefined) {
    lines.push(`error: ${receipt.error}`);
  }
  if (receipt.narrative.length === 0) {
    lines.push("(narrative 为空)");
  } else {
    for (const line of receipt.narrative) {
      lines.push(line);
    }
  }
  if (receipt.pendingChoice !== null && receipt.pendingChoice !== undefined) {
    lines.push("--- pendingChoice ---");
    lines.push(JSON.stringify(receipt.pendingChoice, null, 2));
  }
  return lines.join("\n");
}

/** 页脚状态行：`[Ygg: <runId> | <stage> | jobs:<N> gates:<M>]`。 */
export function statusLine(runId: string, summary: RunSummary | null): string {
  if (summary === null || !summary.available) {
    return `[Ygg: ${runId} | unknown | jobs:0 gates:0]`;
  }
  const stage = summary.stage === "" ? "unknown" : summary.stage;
  return `[Ygg: ${runId} | ${stage} | jobs:${summary.jobs.length} gates:${summary.gates.length}]`;
}

// ───────────────────────────── daemon 客户端 ─────────────────────────────

interface PendingRequest {
  resolve: (value: Record<string, unknown>) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

export type DaemonMode = "daemon" | "cli";

/**
 * 常驻守护进程客户端。一个实例对应一个 runId；`request` 串行化。
 */
export class YggDaemon {
  readonly opts: Required<Omit<DaemonOptions, "env" | "spawnBin" | "spawnArgs">> & DaemonOptions;

  private child: ChildProcessWithoutNullStreams | null = null;
  private reader: readline.Interface | null = null;
  private pending = new Map<number, PendingRequest>();
  private seq = 0;
  private chain: Promise<unknown> = Promise.resolve();
  private started = false;
  private closed = false;
  private modeValue: DaemonMode = "daemon";
  /** daemon 的**非回执行**（日志 / 崩溃尾音），诊断用。 */
  readonly noise: string[] = [];
  /** 最近一次降级原因（`mode === "cli"` 时说明为何不走 daemon）。 */
  degradedReason = "";

  constructor(options: DaemonOptions) {
    const cwd = options.cwd;
    this.opts = {
      ...options,
      cwd,
      modDir: options.modDir ?? path.join(cwd, DEFAULT_MOD_DIR),
      runsRoot: options.runsRoot ?? path.join(cwd, DEFAULT_RUNS_ROOT),
      agentBin: options.agentBin ?? path.join(cwd, ".ygg", "bin", "ygg-agent"),
      requestTimeoutMs: options.requestTimeoutMs ?? DEFAULT_REQUEST_TIMEOUT_MS,
      cliTimeoutMs: options.cliTimeoutMs ?? DEFAULT_CLI_TIMEOUT_MS,
    };
  }

  get runId(): string {
    return this.opts.runId;
  }

  get runDir(): string {
    return path.join(this.opts.runsRoot, this.opts.runId);
  }

  get mode(): DaemonMode {
    return this.modeValue;
  }

  get alive(): boolean {
    return this.child !== null && this.child.exitCode === null && !this.closed;
  }

  /** 起进程并 ping 一次；`--daemon` 不被支持则降级单发（并记下原因）。 */
  async start(): Promise<DaemonMode> {
    if (this.closed) {
      throw new Error("daemon 已关闭");
    }
    if (this.started) {
      return this.modeValue;
    }
    this.started = true;
    try {
      this.spawnDaemon();
      await this.exchange("ping", {}, this.opts.requestTimeoutMs);
      this.modeValue = "daemon";
    } catch (error) {
      this.degradedReason = error instanceof Error ? error.message : String(error);
      this.killChild();
      this.modeValue = "cli";
    }
    return this.modeValue;
  }

  /** 摘要（只读动作；daemon 下也顺带 drain 一次 inbox）。 */
  async summary(): Promise<RunSummary> {
    const receipt = await this.request("summary", {});
    if (receipt.summary !== undefined) {
      return receipt.summary;
    }
    return unknownSummary(receipt.error ?? "summary 回执缺失");
  }

  /**
   * 排空一次 inbox 并把回执交回（`summary` op 内部会 drain）。
   *
   * 与 {@link summary} 的区别只在「要不要叙事」：工具结果（如 `git.lane_probe`）
   * 的折叠行会在 drain 里产生，走 `summary` 拿不到，走这里能拿到。
   */
  async drain(): Promise<StepReceipt> {
    return this.request("summary", {});
  }

  /** 派发一个事件（payload 可以是字符串 / 数字 / 缺省）。 */
  async dispatch(event: string, payload?: string | number): Promise<StepReceipt> {
    const params: Record<string, unknown> = { event };
    if (payload !== undefined) {
      params.payload = payload;
    }
    return this.request("dispatch", params);
  }

  /** 把内容写进本局 VFS（`artifacts/<名>`）。 */
  async vfsPut(
    artifactPath: string,
    content: string,
  ): Promise<{ ok: boolean; bytes?: number; sha256?: string; error?: string }> {
    const receipt = await this.request("vfs_put", { path: artifactPath, content });
    if (!receipt.ok) {
      return { ok: false, error: receipt.error ?? "vfs_put 失败" };
    }
    const out: { ok: boolean; bytes?: number; sha256?: string } = { ok: true };
    const bytes = asNumber((receipt.raw ?? {}).bytes, -1);
    if (bytes >= 0) {
      out.bytes = bytes;
    }
    const sha = asString((receipt.raw ?? {}).sha256);
    if (sha !== "") {
      out.sha256 = sha;
    }
    return out;
  }

  async vfsGet(artifactPath: string): Promise<{ ok: boolean; content: string; error?: string }> {
    const receipt = await this.request("vfs_get", { path: artifactPath });
    if (!receipt.ok) {
      return { ok: false, content: "", error: receipt.error ?? "vfs_get 失败" };
    }
    return { ok: true, content: asString((receipt.raw ?? {}).content) };
  }

  async vfsList(): Promise<{ ok: boolean; artifacts: string[]; error?: string }> {
    const receipt = await this.request("vfs_list", {});
    if (!receipt.ok) {
      return { ok: false, artifacts: [], error: receipt.error ?? "vfs_list 失败" };
    }
    const raw = (receipt.raw ?? {}).artifacts;
    const artifacts = Array.isArray(raw)
      ? raw
          .map((item) => {
            if (typeof item === "string") return item;
            const rec = asRecord(item);
            if (rec === null || typeof rec.path !== "string") return "";
            const bytes = asNumber(rec.bytes, -1);
            const sha = asString(rec.sha256);
            return bytes >= 0 && sha !== ""
              ? `${rec.path} (${bytes} B, sha256=${sha.slice(0, 12)})`
              : rec.path;
          })
          .filter((v) => v !== "")
      : [];
    return { ok: true, artifacts };
  }

  /** 局末结算（读状态事实写 settlement.json）。 */
  async settle(): Promise<{ ok: boolean; settlement: unknown; error?: string }> {
    const receipt = await this.request("settle", {});
    if (!receipt.ok) {
      return { ok: false, settlement: null, error: receipt.error ?? "settle 失败" };
    }
    return { ok: true, settlement: (receipt.raw ?? {}).settlement ?? null };
  }

  /** 单条请求（对外唯一入口；内部串行 + 一次重启重试）。 */
  async request(op: string, params: Record<string, unknown> = {}): Promise<StepReceipt> {
    const task = this.chain.then(async () => {
      if (!this.started) {
        await this.start();
      }
      if (this.modeValue === "cli") {
        return this.requestCli(op, params);
      }
      try {
        return await this.requestDaemon(op, params);
      } catch (error) {
        // 传输层失败（进程死了 / 超时）→ 重启一次再试；语义错误不重试。
        if (!this.closed) {
          this.killChild();
          try {
            return await this.requestDaemon(op, params);
          } catch (retryError) {
            return toStepReceipt(null, "summary", errorText(retryError));
          }
        }
        return toStepReceipt(null, "summary", errorText(error));
      }
    });
    this.chain = task.catch(() => undefined);
    return task;
  }

  /** 关闭所有子进程（`session_shutdown` 调它；幂等）。 */
  async closeAll(): Promise<void> {
    if (this.closed) {
      return;
    }
    this.closed = true;
    if (this.child !== null && this.modeValue === "daemon") {
      try {
        this.writeLine({ id: ++this.seq, op: "shutdown" });
        this.child.stdin.end();
      } catch {
        // 关不掉就直接杀；关闭路径不吞咽语义错误，但没有需要汇报的对象。
      }
    }
    this.killChild();
  }

  private async requestDaemon(
    op: string,
    params: Record<string, unknown>,
  ): Promise<StepReceipt> {
    if (!this.alive) {
      this.spawnDaemon();
    }
    const response = await this.exchange(op, params, this.opts.requestTimeoutMs);
    return toStepReceipt(response);
  }

  /** 写一行并等同 id 的回行（daemon 协议的最小单元）。 */
  private exchange(
    op: string,
    params: Record<string, unknown>,
    timeoutMs: number,
  ): Promise<Record<string, unknown>> {
    const child = this.child;
    if (child === null) {
      return Promise.reject(new Error("daemon 子进程不在"));
    }
    const id = ++this.seq;
    return new Promise<Record<string, unknown>>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`daemon 请求超时（${op} > ${timeoutMs}ms）`));
      }, timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      this.writeLine({ id, op, ...params });
    });
  }

  private writeLine(payload: Record<string, unknown>): void {
    const child = this.child;
    if (child === null || child.stdin === null || child.stdin.destroyed) {
      throw new Error("daemon 子进程不可写");
    }
    child.stdin.write(`${JSON.stringify(payload)}\n`);
  }

  private spawnDaemon(): void {
    if (this.closed) {
      throw new Error("daemon 已关闭");
    }
    this.killChild();
    const bin = this.opts.spawnBin ?? this.opts.agentBin;
    const args = [
      ...(this.opts.spawnArgs ?? []),
      "--daemon",
      "--mod",
      this.opts.modDir,
      "--run-id",
      this.opts.runId,
      "--runs-root",
      this.opts.runsRoot,
    ];
    const env = { ...process.env, ...(this.opts.env ?? {}) };
    const child = spawn(bin, args, { cwd: this.opts.cwd, env, stdio: ["pipe", "pipe", "pipe"] });
    this.child = child;

    const reader = readline.createInterface({ input: child.stdout });
    this.reader = reader;
    reader.on("line", (line) => this.onLine(line));
    child.stderr.on("data", (chunk: Buffer) => {
      this.noise.push(chunk.toString("utf8").trim());
      if (this.noise.length > 40) {
        this.noise.shift();
      }
    });
    child.on("exit", (code, signal) => {
      this.child = null;
      reader.close();
      this.reader = null;
      const why = signal !== null ? `signal=${signal}` : `code=${code ?? "?"}`;
      for (const [id, pending] of this.pending) {
        clearTimeout(pending.timer);
        this.pending.delete(id);
        pending.reject(new Error(`daemon 子进程退出（${why}），请求 #${id} 未完成`));
      }
    });
    child.on("error", (error) => {
      this.noise.push(`spawn error: ${error.message}`);
    });
  }

  private onLine(line: string): void {
    const text = line.trim();
    if (text === "") {
      return;
    }
    let parsed: unknown;
    try {
      parsed = JSON.parse(text);
    } catch {
      this.noise.push(text);
      if (this.noise.length > 40) {
        this.noise.shift();
      }
      return;
    }
    const row = asRecord(parsed);
    if (row === null) {
      return;
    }
    const id = asNumber(row.id, -1);
    const pending = id >= 0 ? this.pending.get(id) : undefined;
    if (pending === undefined) {
      this.noise.push(text);
      if (this.noise.length > 40) {
        this.noise.shift();
      }
      return;
    }
    clearTimeout(pending.timer);
    this.pending.delete(id);
    pending.resolve(row);
  }

  private killChild(): void {
    const child = this.child;
    this.child = null;
    if (this.reader !== null) {
      try {
        this.reader.close();
      } catch (error) {
        // 已关闭：记一行诊断，不改变关闭语义（关闭路径没有需要汇报的对象）。
        this.noise.push(`reader close: ${errorText(error)}`);
      }
      this.reader = null;
    }
    for (const [id, pending] of this.pending) {
      clearTimeout(pending.timer);
      this.pending.delete(id);
      pending.reject(new Error(`daemon 被重置，请求 #${id} 未完成`));
    }
    if (child !== null) {
      try {
        child.kill();
      } catch (error) {
        this.noise.push(`child kill: ${errorText(error)}`);
      }
    }
  }

  // ── 单发降级（老宿主不支持 `--daemon`）：一条命令一次派发 ──

  private async requestCli(
    op: string,
    params: Record<string, unknown>,
  ): Promise<StepReceipt> {
    const bin = this.opts.spawnBin ?? this.opts.agentBin;
    const base = [
      ...(this.opts.spawnArgs ?? []),
      "--mod",
      this.opts.modDir,
      "--run-id",
      this.opts.runId,
      "--runs-root",
      this.opts.runsRoot,
      "--resume",
    ];
    switch (op) {
      case "ping":
      case "summary": {
        // 单发回执没有 `meta.summary`：返回明确的降级摘要（不猜 jobs/gates）。
        const step = await this.runCli(bin, [...base, "--event", "agent.summary"]);
        const receipt = toStepReceipt(step.value, "summary", step.error);
        receipt.summary = unknownSummary(
          "单发降级模式：宿主不支持 `--daemon`，回执里没有 `meta.summary`（jobs/gates 未知）",
        );
        return receipt;
      }
      case "dispatch": {
        const event = asString(params.event);
        const argv = [...base, "--event", event];
        const payload = params.payload;
        if (payload !== undefined) {
          argv.push("--payload", JSON.stringify(payload));
        }
        const step = await this.runCli(bin, argv);
        return toStepReceipt(step.value, "summary", step.error);
      }
      case "vfs_put": {
        const artifactPath = asString(params.path);
        const content = asString(params.content);
        const tmp = path.join(os.tmpdir(), `ygg-vfs-put-${process.pid}-${Date.now()}.txt`);
        try {
          fs.writeFileSync(tmp, content, "utf8");
          const step = await this.runCli(bin, [
            "--mod",
            this.opts.modDir,
            "--run-id",
            this.opts.runId,
            "--runs-root",
            this.opts.runsRoot,
            "--artifact-put",
            `${artifactPath}=${tmp}`,
          ]);
          return toStepReceipt(step.value, "summary", step.error);
        } finally {
          try {
            fs.unlinkSync(tmp);
          } catch {
            // 临时件已消失
          }
        }
      }
      case "vfs_get": {
        const artifactPath = asString(params.path);
        const step = await this.runCli(bin, [
          "--mod",
          this.opts.modDir,
          "--run-id",
          this.opts.runId,
          "--runs-root",
          this.opts.runsRoot,
          "--artifact-get",
          artifactPath,
        ]);
        if (step.error !== undefined) {
          return toStepReceipt({ ok: false }, "summary", step.error);
        }
        return toStepReceipt({ ok: true, path: artifactPath, content: step.stdout });
      }
      case "vfs_list": {
        const dir = path.join(this.runDir, "artifacts");
        let artifacts: string[] = [];
        try {
          artifacts = fs.readdirSync(dir).map((name) => `artifacts/${name}`);
        } catch {
          artifacts = [];
        }
        return toStepReceipt({ ok: true, artifacts });
      }
      case "settle": {
        const step = await this.runCli(bin, [
          "--mod",
          this.opts.modDir,
          "--run-id",
          this.opts.runId,
          "--runs-root",
          this.opts.runsRoot,
          "--settle",
        ]);
        return toStepReceipt(step.value, "summary", step.error);
      }
      default:
        return toStepReceipt(null, "summary", `单发降级模式不支持 op=${op}`);
    }
  }

  private runCli(
    bin: string,
    argv: string[],
  ): Promise<{ value: unknown; stdout: string; error?: string }> {
    return new Promise((resolve) => {
      const env = { ...process.env, ...(this.opts.env ?? {}) };
      const child = spawn(bin, argv, { cwd: this.opts.cwd, env });
      let stdout = "";
      let stderr = "";
      let settled = false;
      const timer = setTimeout(() => {
        if (!settled) {
          settled = true;
          try {
            child.kill();
          } catch (error) {
            this.noise.push(`cli kill: ${errorText(error)}`);
          }
          resolve({ value: null, stdout, error: `单发命令超时（> ${this.opts.cliTimeoutMs}ms）` });
        }
      }, this.opts.cliTimeoutMs);
      child.stdout.on("data", (chunk: Buffer) => {
        stdout += chunk.toString("utf8");
      });
      child.stderr.on("data", (chunk: Buffer) => {
        stderr += chunk.toString("utf8");
      });
      child.on("error", (error) => {
        if (!settled) {
          settled = true;
          clearTimeout(timer);
          resolve({ value: null, stdout, error: `无法执行 ${bin}：${error.message}` });
        }
      });
      child.on("close", (code) => {
        if (settled) {
          return;
        }
        settled = true;
        clearTimeout(timer);
        const text = stdout.trim();
        let value: unknown = null;
        if (text !== "") {
          try {
            value = JSON.parse(text.split("\n").pop() ?? text);
          } catch {
            value = null;
          }
        }
        const error =
          code === 0
            ? undefined
            : `退出码 ${code ?? "?"}${stderr.trim() === "" ? "" : `：${stderr.trim().split("\n")[0]}`}`;
        resolve({ value, stdout: text, error });
      });
    });
  }
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
