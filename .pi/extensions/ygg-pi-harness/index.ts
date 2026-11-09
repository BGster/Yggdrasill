/**
 * index.ts —— `.pi/extensions/ygg-pi-harness` 的装配入口
 *
 * 这一层**只做装配**，不含业务判定：
 * - `session_start` / `session_shutdown`：探测 `.ygg/bin/ygg-agent`、重建/关闭客户端；
 * - `tool_call`：把动作交给 [`checkToolCallFirewall`]（物理红线 + 批量入口禁令）；
 * - `/ygg`：`status` / `stop`（人肉兜底通道）；
 * - 两个高层工具：`ygg_session`（会话面）与 `ygg_run_lanes`（泳道编排面）。
 *
 * 为什么要有这一层：CLI 的 `--event` 面要求人（或脚本）记住十几个动作载荷位次；
 * 扩展把「一次会话该做的事」封成两三个动作，同时把 `narrative` 整段回贴
 * （禁止解析式截断），并把状态写进页脚 `[Ygg: <runId> | <stage> | jobs:<N> gates:<M>]`。
 */

import * as fs from "node:fs";
import * as path from "node:path";
import { isToolCallEventType, type ExtensionAPI, type ExtensionContext } from "@earendil-works/pi-coding-agent";
import { StringEnum } from "@earendil-works/pi-ai";
import { Type } from "typebox";

import {
  describeEnvKeys,
  formatReceipt,
  readEnvFiles,
  statusLine,
  subagentSettings,
  YggDaemon,
  type RunSummary,
  type StepReceipt,
  type SubagentSettings,
} from "./daemon.ts";
import {
  buildCompactInstructions,
  buildHandoffMarkdown,
  buildResumeMessage,
  evaluateContextPressure,
  formatPercent,
  HANDOFF_RESUME_CUSTOM_TYPE,
  isStableCheckpoint,
  parsePositiveIntEnv,
  stableCheckpointReason,
  writeHandoffToVfs,
  type ContextPressure,
  type HandoffWrite,
} from "./compaction.ts";
import { checkToolCallFirewall } from "./firewall.ts";
import {
  extractAgentId,
  isResumableAgentId,
  orchestrateActiveJobs,
  type ExecuteSubagent,
  type OrchestrationResult,
  type SubagentResult,
} from "./orchestrator.ts";

const STATUS_KEY = "ygg-harness";
const DEFAULT_MOD_DIR = ".ygg/mods/pi-agent-seed";
const DEFAULT_RUNS_ROOT = ".ygg/runs";

/** 每个 runId 一个常驻客户端；`activeRunId` 是页脚展示的那一个。 */
const daemons = new Map<string, YggDaemon>();
/** 并发启动锁：同一 runId 正在 `start()` 时复用同一个 Promise，防并发重复拉起。 */
const daemonsInflight = new Map<string, Promise<YggDaemon>>();
let activeRunId = "";
let lastSummary: RunSummary | null = null;

/** 仅供单测重置进程内缓存。 */
export function resetClientsForTest(): void {
  daemons.clear();
  daemonsInflight.clear();
  activeRunId = "";
  lastSummary = null;
}
/**
 * 本次压缩是谁发起的：`"self"` = `maybeHandoffAndCompact`（压完自己续跑），
 * `null` = 外部（`/compact` / 阈值 / 溢出恢复，压完只把状态块带回上下文）。
 * 不用它区分的话，一次压缩会被补两条相同的续跑消息。
 */
let compactionOrigin: "self" | null = null;

// ───────────────────────────── 小工具 ─────────────────────────────

function agentBinPath(cwd: string): string {
  return path.join(cwd, ".ygg", "bin", "ygg-agent");
}

function hasAgentBinary(cwd: string): boolean {
  try {
    fs.accessSync(agentBinPath(cwd), fs.constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

function setStatus(ctx: ExtensionContext, summary: RunSummary | null): void {
  const line = statusLine(activeRunId === "" ? "(no run)" : activeRunId, summary ?? lastSummary);
  try {
    ctx.ui.setStatus(STATUS_KEY, line);
  } catch {
    // 非 UI 模式（print / json）：状态栏是可选装饰，不影响工具语义。
  }
}

/** 取（或建）一个 runId 的客户端；`start()` 负责 `--daemon` 探测与降级。 */
export async function clientFor(
  ctx: ExtensionContext,
  runId: string,
  overrides: {
    modDir?: string;
    runsRoot?: string;
    createDaemon?: (opts: {
      cwd: string;
      runId: string;
      modDir: string;
      runsRoot: string;
      agentBin: string;
    }) => YggDaemon;
  } = {},
): Promise<YggDaemon> {
  const cwd = ctx.cwd;
  const key = runId;
  activeRunId = runId;
  const existing = daemons.get(key);
  if (existing !== undefined) {
    return existing;
  }
  const inflight = daemonsInflight.get(key);
  if (inflight !== undefined) {
    return await inflight;
  }
  const promise = (async () => {
    const opts = {
      cwd,
      runId,
      modDir: overrides.modDir ?? path.join(cwd, DEFAULT_MOD_DIR),
      runsRoot: overrides.runsRoot ?? path.join(cwd, DEFAULT_RUNS_ROOT),
      agentBin: agentBinPath(cwd),
    };
    const client =
      overrides.createDaemon !== undefined ? overrides.createDaemon(opts) : new YggDaemon(opts);
    await client.start();
    daemons.set(key, client);
    if (client.mode === "cli") {
      try {
        ctx.ui.notify(
          `ygg-agent 不支持 --daemon，已降级为单发模式：${client.degradedReason}`,
          "warning",
        );
      } catch {
        // 非 UI 模式忽略通知异常。
      }
    }
    return client;
  })();
  daemonsInflight.set(key, promise);
  try {
    return await promise;
  } finally {
    daemonsInflight.delete(key);
  }
}

/** 把一串回执合并成一段可贴的文本（每段 narrative 全行保留）。 */
function formatReceipts(entries: { label: string; receipt: StepReceipt }[]): string {
  return entries.map((entry) => formatReceipt(entry.label, entry.receipt)).join("\n\n");
}

function payloadText(payload: string | number | undefined): string {
  if (payload === undefined) {
    return "(none)";
  }
  return typeof payload === "number" ? String(payload) : JSON.stringify(payload);
}

/** 当前活跃 run 的客户端（没有活跃局时 `undefined`）。 */
function activeClient(): YggDaemon | undefined {
  return activeRunId === "" ? undefined : daemons.get(activeRunId);
}

export function resolveEffectiveWindowCapFromEnv(
  cwd: string,
  processEnv: NodeJS.ProcessEnv = process.env,
): number | undefined {
  const fileVal = readEnvFiles(cwd).get("YGG_CONTEXT_WINDOW");
  return parsePositiveIntEnv(fileVal) ?? parsePositiveIntEnv(processEnv.YGG_CONTEXT_WINDOW);
}

/**
 * 压力探针：`getContextUsage()` 的 `tokens` 在刚压完时是 `null`，模型也可能没报
 * `contextWindow`；两者不确定时按「不知道」处理（=`normal`，不刹车）。
 * 若 `.env.local` / `.env` 或 `process.env` 设置了正整数 `YGG_CONTEXT_WINDOW`，
 * 则作为 `effectiveWindowCap` 传入（否则使用默认 200k 有效上限）。
 */
function contextPressure(ctx: ExtensionContext): ContextPressure {
  const usage = ctx.getContextUsage();
  const effectiveWindowCap = resolveEffectiveWindowCapFromEnv(ctx.cwd);
  return evaluateContextPressure({
    usedTokens: usage?.tokens ?? null,
    contextWindow: usage?.contextWindow ?? ctx.model?.contextWindow ?? null,
    ...(effectiveWindowCap !== undefined ? { effectiveWindowCap } : {}),
  });
}

/** 交接单入参（`vfsList` 失败不阻断：交接单会自己补上自己那条）。 */
async function handoffFor(client: YggDaemon, summary: RunSummary, pressure: ContextPressure, reason: string) {
  const listed = await client.vfsList();
  return writeHandoffToVfs(client, {
    runId: client.runId,
    summary,
    pressure,
    reason,
    ...(listed.ok ? { artifacts: listed.artifacts } : {}),
  });
}

/**
 * 任何来源的压缩**之前**先把局面落到 VFS（`artifacts/handoff-latest.md`）。
 *
 * 压缩之后上下文里只剩摘要，交接单是唯一还能被重新读到的「事实快照」。写失败只
 * 安静吞掉：把压缩拦下来（或抛错）比丢一份交接单更糟。
 */
async function ensureHandoffArtifact(ctx: ExtensionContext, reason: string): Promise<void> {
  const client = activeClient();
  if (client === undefined) {
    return;
  }
  try {
    const summary = await client.summary();
    if (!summary.available) {
      return;
    }
    await handoffFor(client, summary, contextPressure(ctx), `compaction(${reason})`);
  } catch {
    // 见函数头注：压缩本体不能被交接单的写失败拖住。
  }
}

interface HandoffOptions {
  /** 写进交接单的触发原因。 */
  note?: string;
  /** 显式触发（`handoff_compact` / 上层已经决定刹车）：不看水位与稳定节点。 */
  force?: boolean;
  /** 上层（`orchestrateActiveJobs`）已经写过的交接单：复用，不重写。 */
  write?: HandoffWrite;
}

/**
 * 检查点交接：判压力 + 判稳定节点 → 写交接单 → 压缩 → **自动续跑**。
 *
 * 为什么要「稳定节点」这个条件：在作业在飞（没有门也没有冲突）时压缩，会把手中
 * 的 subagent 结果压成一段叙述；落在门上/边界上时压缩，压完的局面是可以直接复核
 * 的（谁的提案停在哪个门）。所以不稳定就不压，等这一波落地。
 */
async function maybeHandoffAndCompact(
  pi: ExtensionAPI,
  ctx: ExtensionContext,
  client: YggDaemon,
  options: HandoffOptions = {},
): Promise<string> {
  const summary = await client.summary();
  if (!summary.available) {
    return `[handoff] 摘要不可用（${summary.degraded ?? "未知原因"}）：拒绝盲压缩。`;
  }

  const pressure = contextPressure(ctx);
  const stable = isStableCheckpoint(summary);
  const hard = pressure.level === "hard_brake";
  const lines: string[] = [
    `[handoff] 上下文 ${pressure.usedTokens} / ${pressure.contextWindow}` +
      ` (${formatPercent(pressure.ratio)}) · 水位 ${pressure.level}`,
    `[handoff] 稳定节点：${stable ? "是" : "否"}（${stableCheckpointReason(summary)}）`,
  ];

  const force = options.force === true;
  if (!force && !(pressure.shouldHandoffAndCompact && (stable || hard))) {
    lines.push(
      pressure.shouldHandoffAndCompact
        ? "[handoff] 未压缩：还没落在稳定节点上（等这一波落地再来）。"
        : "[handoff] 未压缩：水位未到（< 软水位）。",
    );
    return lines.join("\n");
  }

  const write =
    options.write !== undefined && options.write.ok
      ? options.write
      : await handoffFor(
          client,
          summary,
          pressure,
          options.note ?? (force ? "manual（显式 handoff_compact）" : pressure.level),
        );
  lines.push(
    write.ok
      ? `[handoff] 已写入 VFS：${write.path}（${write.bytes ?? write.markdown.length} bytes）`
      : `[handoff] 写入失败：${write.error ?? "未知原因"}（仍按同一份内容续跑，但不声称进过 VFS）`,
  );

  const markdown = write.markdown;
  compactionOrigin = "self";
  ctx.compact({
    customInstructions: buildCompactInstructions(client.runId, markdown),
    onComplete: (result) => {
      compactionOrigin = null;
      // 压缩会 abort 当前回合：这次工具调用的输出大概率到不了模型，所以**续跑
      // 消息必须自带全部局面**（交接单 + 下一步命令），并且主动开一个新回合。
      void pi
        .sendMessage(
          {
            customType: HANDOFF_RESUME_CUSTOM_TYPE,
            content: buildResumeMessage(client.runId, summary, markdown),
            display: true,
            details: {
              runId: client.runId,
              handoff: write.path,
              tokensBefore: result.tokensBefore,
            },
          },
          { triggerTurn: true },
        )
        .catch((error: unknown) => {
          ctx.ui.notify(
            `ygg: 压缩完成但续跑消息没送达：${error instanceof Error ? error.message : String(error)}`,
            "warning",
          );
        });
    },
    onError: (error) => {
      compactionOrigin = null;
      ctx.ui.notify(
        `ygg: 压缩失败（局面已在 ${write.path}）：${error.message}`,
        "error",
      );
    },
  });
  lines.push(
    `[handoff] 已请求压缩：摘要会原样保留状态块，压完自动续跑` +
      `（customType=${HANDOFF_RESUME_CUSTOM_TYPE}）。`,
  );
  return lines.join("\n");
}

// ───────────────────────────── kickoff ─────────────────────────────

/**
 * 载荷字段清洗：`|` 是 `task.intake` / `requirement.add` 的**段分隔符**。
 * 需求描述里写一个 `|`（表格、管道命令）就会把后面的正文挤成额外段——不报错，
 * 但需求会被静默截断。所以入口处先把它换成 `/`（可读且无语义损失）。
 */
function wireField(text: string): string {
  return String(text ?? "").replaceAll("|", "/");
}

export interface KickoffInput {
  runId: string;
  title: string;
  description: string;
  goalAuth: string;
  requirements: { title: string; description: string }[];
}

export interface KickoffClient {
  dispatch(event: string, payload?: string | number): Promise<StepReceipt>;
  summary(): Promise<RunSummary>;
}

/**
 * 开局：起 daemon →（若提供非空 `goalAuth`：登记 goal 授权 → 切 goal；留空则保持
 * Mod 默认 `guide` 模式）→ `task.intake` → 逐条 `requirement.add` → `requirement.dispatch`。
 *
 * 顺序是有意的：`goal.authorization` 先于 `play.mode=goal`（goal 局收口时
 * `review.note` 会校这条记录），两者都先于 `task.intake`。
 */
export async function kickoffSession(
  ctx: ExtensionContext,
  client: KickoffClient,
  input: KickoffInput,
): Promise<string> {
  const entries: { label: string; receipt: StepReceipt }[] = [];

  if (input.goalAuth.trim() !== "") {
    const auth = await client.dispatch("config.set", `goal.authorization=${input.goalAuth.trim()}`);
    entries.push({ label: "config.set goal.authorization", receipt: auth });
    const mode = await client.dispatch("config.set", "play.mode=goal");
    entries.push({ label: "config.set play.mode=goal", receipt: mode });
  }

  const intake = await client.dispatch(
    "task.intake",
    `${wireField(input.title)}|${wireField(input.description)}`,
  );
  entries.push({ label: "task.intake", receipt: intake });

  for (const requirement of input.requirements) {
    const added = await client.dispatch(
      "requirement.add",
      `${wireField(requirement.title)}|${wireField(requirement.description)}`,
    );
    entries.push({ label: `requirement.add「${requirement.title}」`, receipt: added });
  }

  if (input.requirements.length > 0) {
    const dispatched = await client.dispatch("requirement.dispatch", "");
    entries.push({ label: "requirement.dispatch", receipt: dispatched });
  }

  lastSummary = (await client.summary()) ?? lastSummary;
  setStatus(ctx, lastSummary);
  return formatReceipts(entries);
}

// ───────────────────────────── settle_close ─────────────────────────────

interface SettleCloseInput {
  runId: string;
  evolutionSummary: string;
  evolutionContent: string;
  lesson: string;
  score: number;
  candidates: string;
  classes: string;
  sessionId: string;
}

/**
 * 收口：`coordinate.submit`（若还在 coordinate）→ `verify.submit true` →
 * 进化复盘制品入 VFS → `harness.archive_sessions` → `settlement.evolution` →
 * `review.note` → `settle`。
 *
 * 每一步都按 receipt 决定是否继续：拒绝（`ok=false` 或 narrative 出现
 * `[REJECTED`）不硬推下去，直接把现场回贴——推断式续跑会把「被拒」洗成「已收口」。
 */
async function settleAndClose(
  ctx: ExtensionContext,
  client: YggDaemon,
  input: SettleCloseInput,
): Promise<string> {
  const entries: { label: string; receipt: StepReceipt }[] = [];
  const rejected = (receipt: StepReceipt): boolean =>
    !receipt.ok || receipt.narrative.some((line) => line.includes("[REJECTED") || line.includes("[BLOCKED"));

  const before = await client.summary();
  if (!before.available || before.stage === "coordinate") {
    const submitted = await client.dispatch("coordinate.submit", "");
    entries.push({ label: "coordinate.submit", receipt: submitted });
    if (rejected(submitted)) {
      lastSummary = await client.summary();
      setStatus(ctx, lastSummary);
      return formatReceipts(entries);
    }
  }

  const verified = await client.dispatch("verify.submit", "true");
  entries.push({ label: "verify.submit true", receipt: verified });
  if (rejected(verified)) {
    lastSummary = await client.summary();
    setStatus(ctx, lastSummary);
    return formatReceipts(entries);
  }

  const artifact = "artifacts/evolution-review.md";
  const content =
    input.evolutionContent.trim() !== ""
      ? input.evolutionContent
      : [`# 进化复盘`, ``, input.evolutionSummary, ``].join("\n");
  const archived = await client.vfsPut(artifact, content);
  entries.push({
    label: `vfs_put ${artifact}`,
    receipt: {
      ok: archived.ok,
      narrative: archived.ok
        ? [`[VFS] ${artifact} ← ${content.length} bytes`]
        : [`[VFS REFUSED] ${artifact}：${archived.error ?? "未知原因"}`],
      pendingChoice: null,
      tools: [],
      raw: { ...archived },
    },
  });

  // 会话归档（本轮 Pi 会话 → 本局目录）。args 同时给父/批次与结算三元组：
  // 宿主的多余键不会被反应器读取，而两种命名（r1 daemon 的 parent/batch 与
  // 当前 Mod 反应器的 moved/remaining/dest）都覆盖到。
  // 若 Core 的 `settlement.evolution` 已经通过同步 `ygg.tool.call` 内联结算
  // `harness.archive_sessions`，这里即使前置 `tool.call` 未落库或已跑过，
  // 下面的 `sessions=` 段也会幂等兜底，确保 `review.note` 不因缺会话登记被拦。
  const archiveDest = `.ygg/runs/${input.runId}/sessions`;
  const archiveArgs = [
    `parent=${input.sessionId}`,
    `batch=${input.runId}`,
  ].join(";");
  const archivedSessions = await client.dispatch(
    "tool.call",
    `harness.archive_sessions|${archiveArgs}`,
  );
  entries.push({ label: "tool.call harness.archive_sessions", receipt: archivedSessions });
  const archiveSettledByTool = archivedSessions.narrative.some((line) =>
    line.includes("[SESSIONS SETTLED]") || line.includes("[TOOL SETTLED] harness.archive_sessions"),
  );

  const evolutionPayload = [
    artifact,
    input.evolutionSummary.trim() === "" ? "(no candidates this round)" : input.evolutionSummary,
    `c=${input.candidates.trim() === "" ? "0/0" : input.candidates.trim()}`,
    ...(input.classes.trim() === "" ? [] : [`cls=${input.classes.trim()}`]),
    ...(archiveSettledByTool ? [] : [`sessions=0/0/${archiveDest}`]),
    "todo=0/verified empty via ygg_session settle_close",
  ].join("|");
  const evolution = await client.dispatch("settlement.evolution", evolutionPayload);
  entries.push({ label: "settlement.evolution", receipt: evolution });

  const note = await client.dispatch(
    "review.note",
    `${input.lesson.trim() === "" ? "no lesson recorded" : input.lesson.trim()}|${input.score}`,
  );
  entries.push({ label: "review.note", receipt: note });

  const settlement = await client.settle();
  entries.push({
    label: "settle",
    receipt: {
      ok: settlement.ok,
      narrative: settlement.ok
        ? [`[SETTLE] ${JSON.stringify(settlement.settlement)}`]
        : [`[SETTLE FAILED] ${settlement.error ?? "未知原因"}`],
      pendingChoice: null,
      tools: [],
      raw: { settlement: settlement.settlement },
    },
  });

  lastSummary = await client.summary();
  setStatus(ctx, lastSummary);
  return formatReceipts(entries);
}

// ───────────────────────────── 泳道编排 ─────────────────────────────

function formatOrchestration(result: OrchestrationResult): string {
  const lines: string[] = [];
  if (result.degraded !== undefined && result.jobs.length === 0) {
    lines.push(`[ygg_run_lanes] ${result.degraded}`);
  }
  const summaryJobs = new Map((result.summary?.jobs ?? []).map((j) => [j.id, j] as const));
  for (const job of result.jobs) {
    const sj = summaryJobs.get(job.id);
    const wtStatus =
      job.worktreeStatus ??
      sj?.worktreeStatus ??
      (job.worktreePath !== null && job.worktreePath !== ""
        ? job.worktreePath
        : job.stage === "development"
          ? "待创建"
          : "主工作区(只读)");
    const subStatus =
      job.subagentStatus ??
      sj?.subagentStatus ??
      (isResumableAgentId(job.agentId)
        ? `已派发 ${job.agentId}`
        : sj !== undefined && isResumableAgentId(sj.executor)
          ? `已派发 ${sj.executor}`
          : "待派发");
    const probeStatus = job.probeStatus ?? sj?.probeStatus;
    lines.push(
      `### ${job.id} · ${job.stage} → ${job.expectedGate} gate` +
        ` | lane ${job.lane > 0 ? job.lane : "-"} | ${job.branch === "" ? "-" : job.branch}`,
    );
    lines.push(`worktree: ${wtStatus}`);
    lines.push(`subagent: ${subStatus}`);
    if (probeStatus !== undefined && probeStatus !== "") {
      lines.push(`probe: ${probeStatus}`);
    }
    lines.push(`artifact: ${job.artifactPath} (${job.archived.ok ? "archived" : "REFUSED"})`);
    if (job.error !== undefined) {
      lines.push(`error: ${job.error}`);
    }
  }
  if (result.narrative.length > 0) {
    lines.push("");
    lines.push("--- receipts ---");
    for (const line of result.narrative) {
      lines.push(line);
    }
  }
  if (result.gates.length > 0) {
    lines.push("");
    lines.push("--- gates ready ---");
    for (const gate of result.gates) {
      lines.push(`- ${gate.id} · ${gate.gate} gate · rev ${gate.rev} · ${gate.title}`);
    }
  }
  return lines.join("\n");
}

// ───────────────────────────── 扩展本体 ─────────────────────────────

export default function yggHarnessExtension(pi: ExtensionAPI): void {
  const subagentTuning = (cwd: string): SubagentSettings => subagentSettings(readEnvFiles(cwd));

  // ① 生命周期：只在 session_start 之后起资源（扩展工厂可能被永不建会话的调用加载）。
  pi.on("session_start", async (_event, ctx) => {
    lastSummary = null;
    if (!hasAgentBinary(ctx.cwd)) {
      ctx.ui.setStatus(STATUS_KEY, `[Ygg: unavailable | ${path.join(".ygg", "bin", "ygg-agent")} missing]`);
      return;
    }
    const env = readEnvFiles(ctx.cwd);
    const tuning = subagentSettings(env);
    setStatus(ctx, null);
    ctx.ui.notify(
      `ygg-pi-harness ready · env keys: ${describeEnvKeys(env).join(", ") || "(none)"}` +
        (tuning.model !== undefined ? ` · subagent model: ${tuning.model}` : ""),
      "info",
    );
  });

  pi.on("session_shutdown", async () => {
    for (const client of daemons.values()) {
      await client.closeAll();
    }
    daemons.clear();
    daemonsInflight.clear();
    activeRunId = "";
    lastSummary = null;
    compactionOrigin = null;
  });

  // ①' 上下文压缩的三个钩子（任何来源的压缩都不让运行状态丢）。
  //
  //   session_before_compact：先把局面落到 VFS（压完只剩摘要，交接单是唯一可重读的事实快照）；
  //   session_compact：非自己发起的压缩 → 把状态块**带回上下文**（不触发新回合）；
  //   session_compact_failed：清掉「自己发起」的标记，否则下一次外部压缩会误以为已续跑。
  pi.on("session_before_compact", async (event, ctx) => {
    await ensureHandoffArtifact(ctx, event.reason);
    return undefined;
  });

  pi.on("session_compact", async (event, ctx) => {
    if (compactionOrigin === "self") {
      // 自己发起的那次由 `ctx.compact({ onComplete })` 负责续跑，这里不再补一条。
      compactionOrigin = null;
      return;
    }
    const client = activeClient();
    if (client === undefined) {
      return;
    }
    try {
      const summary = await client.summary();
      if (!summary.available) {
        return;
      }
      const pressure = contextPressure(ctx);
      const write = await handoffFor(client, summary, pressure, `compaction(${event.reason})`);
      await pi.sendMessage(
        {
          customType: HANDOFF_RESUME_CUSTOM_TYPE,
          content: buildResumeMessage(client.runId, summary, write.markdown),
          display: true,
          details: { runId: client.runId, handoff: write.path, reason: event.reason },
        },
        // 外部压缩（`/compact`）之后停在原地是预期行为：只把状态块放进上下文，
        // 不替用户开新回合。
        { triggerTurn: false },
      );
    } catch (error) {
      ctx.ui.notify(
        `ygg: 压缩后没能把状态块带回上下文：${error instanceof Error ? error.message : String(error)}`,
        "warning",
      );
    }
  });

  pi.on("session_compact_failed", async () => {
    compactionOrigin = null;
  });

  // ② 物理红线 + 批量入口禁令（拦动作，不靠自觉）。
  pi.on("tool_call", async (event, _ctx) => {
    const input = (event.input ?? {}) as Record<string, unknown>;
    const verdict = checkToolCallFirewall(event.toolName, input);
    if (verdict.block) {
      return { block: true, reason: verdict.reason ?? "blocked by ygg-pi-harness firewall" };
    }
    return undefined;
  });

  // ③ `/ygg` 命令：人肉兜底（状态查询 / 停守护进程）。
  pi.registerCommand("ygg", {
    description: "Yggdrasill 会话面：`/ygg status` 查当前处境，`/ygg stop` 关闭常驻守护进程",
    handler: async (args, ctx) => {
      const sub = (args ?? "").trim();
      if (sub === "" || sub === "status") {
        const client = activeRunId === "" ? undefined : daemons.get(activeRunId);
        if (client === undefined) {
          ctx.ui.notify(
            hasAgentBinary(ctx.cwd)
              ? "ygg: 还没有活跃的 run（先调 `ygg_session` 的 kickoff）"
              : "ygg: 找不到 .ygg/bin/ygg-agent（先在仓库根构建宿主）",
            "info",
          );
          return;
        }
        const summary = await client.summary();
        lastSummary = summary.available ? summary : lastSummary;
        setStatus(ctx, lastSummary);
        const lines: string[] = [
          `run: ${client.runId}`,
          `mode: ${client.mode}${client.mode === "cli" ? ` (${client.degradedReason})` : ""}`,
          `stage: ${summary.available ? summary.stage || "unknown" : "unknown"}`,
          `jobs: ${summary.jobs.length} · gates: ${summary.gates.length} · pending proposals: ${summary.pendingProposals}`,
        ];
        for (const job of summary.jobs) {
          const wtStatus =
            job.worktreeStatus ??
            (job.worktreePath !== undefined && job.worktreePath !== ""
              ? job.worktreePath
              : job.stage === "development"
                ? "待创建"
                : "主工作区(只读)");
          const subStatus =
            job.subagentStatus ??
            (isResumableAgentId(job.executor) ? `已派发 ${job.executor}` : "待派发");
          const probe =
            job.probeStatus !== undefined && job.probeStatus !== ""
              ? ` | probe=${job.probeStatus}`
              : "";
          const act = job.subagentAction !== undefined ? ` | action=${job.subagentAction.action}` : "";
          lines.push(
            `  - ${job.id} ${job.stage} rev${job.rev} lane=${job.lane} ${job.branch === "" ? "-" : job.branch}` +
              ` | worktree=${wtStatus} | subagent=${subStatus}${probe}${act}`,
          );
        }
        for (const gate of summary.gates) {
          lines.push(`  gate: ${gate.id} ${gate.gate} rev${gate.rev}`);
        }
        ctx.ui.notify(lines.join("\n"), "info");
        return;
      }
      if (sub === "stop") {
        const stopped: string[] = [];
        for (const [runId, client] of daemons) {
          await client.closeAll();
          stopped.push(runId);
        }
        daemons.clear();
        daemonsInflight.clear();
        activeRunId = "";
        lastSummary = null;
        ctx.ui.setStatus(STATUS_KEY, undefined);
        ctx.ui.notify(`ygg: 已关闭 ${stopped.length} 个守护进程（${stopped.join(", ") || "none"}）`, "info");
        return;
      }
      ctx.ui.notify(`ygg: 未知子命令「${sub}」（可用：status / stop）`, "warning");
    },
  });

  // ④ 会话面：kickoff / dispatch（旧名 step）/ gate / vfs / settle_close。
  pi.registerTool({
    name: "ygg_session",
    label: "Ygg Session",
    description:
      "驱动 Yggdrasill 会话（`.ygg/mods/pi-agent-seed`）：开局、单步派发、过门禁、读写本局 VFS、收口结算。" +
      "所有回执的 narrative 会整段返回（不截断）。" +
      "`action` 是唯一路由参数：其中 `dispatch`（旧名 `step`，仍兼容）是**透传任意底层事件的逃生舱**，" +
      "其余 8 个 action（kickoff / approve_gate / reject_gate / vfs_* / handoff_compact / settle_close）" +
      "是封装好载荷位次的语义化入口——两者不是平级冗余，而是「专用入口 + 通用兜底」的两层结构。",
    promptSnippet:
      "Drive one Yggdrasill run: kickoff (goal mode + intake + requirements), dispatch (dispatch any event; " +
      "legacy alias `step`), approve_gate / reject_gate (plan_gate / accept_gate), vfs_write / vfs_read / vfs_list, " +
      "handoff_compact (checkpoint handoff + compaction + auto-resume), settle_close.",
    promptGuidelines: [
      "Use ygg_session kickoff when the user asks to start a Yggdrasill run; pass runId, title, description, goalAuth and requirements.",
      "Use ygg_session dispatch to dispatch exactly one event and read the full receipt; never batch events in a shell loop. `dispatch` is the escape hatch for events without a dedicated action (guide.show / map.show / config.set / requirement.resolve / tool.call).",
      "Use ygg_session approve_gate only after reading the receipt that raised the gate — approving is a human/coordinator decision.",
      "Use ygg_session handoff_compact when the context is getting long: it writes artifacts/handoff-latest.md, compacts, and resumes the run automatically.",
      "Use ygg_session settle_close when every requirement is done and the run is in review; it archives the evolution review and settles.",
    ],
    parameters: Type.Object({
      action: StringEnum([
        "kickoff",
        "dispatch",
        "step",
        "approve_gate",
        "reject_gate",
        "vfs_write",
        "vfs_read",
        "vfs_list",
        "handoff_compact",
        "settle_close",
      ] as const, { description: "要执行的会话动作" }),
      runId: Type.Optional(Type.String({ description: "本局标识（同一 runId 复用同一个守护进程）" })),
      title: Type.Optional(Type.String({ description: "kickoff：本局标题（task.intake 的第一段）" })),
      description: Type.Optional(Type.String({ description: "kickoff：本局描述（task.intake 的第二段）" })),
      goalAuth: Type.Optional(
        Type.String({
          description:
            "kickoff：仅当用户当次明确授权 goal 模式时填写授权说明（非空时切入 goal 模式；留空则保持默认 guide 模式）",
        }),
      ),
      requirements: Type.Optional(
        Type.Array(
          Type.Object({
            title: Type.String({ description: "需求标题" }),
            description: Type.String({ description: "需求描述（含不允许合并时的理由）" }),
          }),
          { description: "kickoff：要逐条 requirement.add 的需求" },
        ),
      ),
      event: Type.Optional(Type.String({ description: "dispatch：要派发的事件名（如 requirement.dispatch）" })),
      payload: Type.Optional(
        Type.Union([Type.String(), Type.Number()], {
          description: "dispatch：事件载荷（字符串或 JSON 数字；菜单选择必须是数字）",
        }),
      ),
      reqId: Type.Optional(Type.String({ description: "approve_gate / reject_gate：需求 id（rN）" })),
      note: Type.Optional(Type.String({ description: "approve_gate / reject_gate：备注或打回理由" })),
      path: Type.Optional(Type.String({ description: "vfs_write / vfs_read：制品路径（artifacts/…）" })),
      content: Type.Optional(Type.String({ description: "vfs_write：制品内容" })),
      evolutionSummary: Type.Optional(Type.String({ description: "settle_close：进化复盘 summary（必填）" })),
      evolutionContent: Type.Optional(Type.String({ description: "settle_close：进化复盘正文（写入 VFS）" })),
      lesson: Type.Optional(Type.String({ description: "settle_close：review.note 的经验" })),
      score: Type.Optional(Type.Number({ description: "settle_close：review.note 的评分 1-5" })),
      candidates: Type.Optional(Type.String({ description: "settle_close：进化候选计数 kept/screened" })),
      classes: Type.Optional(Type.String({ description: "settle_close：进化候选分类 kind:n,…" })),
      sessionId: Type.Optional(Type.String({ description: "settle_close：要归档的 Pi 会话 id" })),
    }),

    async execute(_toolCallId, params, _signal, _onUpdate, ctx) {
      if (params.action === "approve_gate" || params.action === "reject_gate") {
        const runId = requireRunId(params.runId);
        const client = await clientFor(ctx, runId);
        const reqId = requireString(params.reqId, "reqId");
        const note = params.note ?? "";
        const event = params.action === "approve_gate" ? "gate.approve" : "gate.reject";
        const receipt = await client.dispatch(event, `${reqId}|${note}`);
        lastSummary = (await client.summary()) ?? lastSummary;
        setStatus(ctx, lastSummary);
        const text = formatReceipt(`${event} ${reqId}`, receipt);
        return { content: [{ type: "text", text }], details: { receipt, summary: lastSummary } };
      }

      if (params.action === "dispatch" || params.action === "step") {
        const runId = requireRunId(params.runId);
        const client = await clientFor(ctx, runId);
        const event = requireString(params.event, "event");
        const receipt = await client.dispatch(event, params.payload);
        if (receipt.summary !== undefined) {
          lastSummary = receipt.summary;
        } else {
          lastSummary = (await client.summary()) ?? lastSummary;
        }
        setStatus(ctx, lastSummary);
        // 稳定节点 + 水位到了 → 自动写交接单 + 压缩 + 续跑（否则什么也不做）。
        const handoff = await maybeHandoffAndCompact(pi, ctx, client);
        const text = [
          formatReceipt(`${event} ${payloadText(params.payload)}`, receipt),
          statusLine(runId, lastSummary),
          handoff,
        ].join("\n\n");
        return { content: [{ type: "text", text }], details: { receipt, summary: lastSummary } };
      }

      if (params.action === "handoff_compact") {
        const runId = requireRunId(params.runId ?? (activeRunId === "" ? undefined : activeRunId));
        const client = await clientFor(ctx, runId);
        const text = await maybeHandoffAndCompact(pi, ctx, client, {
          force: true,
          ...(params.note === undefined ? {} : { note: params.note }),
        });
        lastSummary = (await client.summary()) ?? lastSummary;
        setStatus(ctx, lastSummary);
        return { content: [{ type: "text", text }], details: { summary: lastSummary } };
      }

      if (params.action === "vfs_write" || params.action === "vfs_read" || params.action === "vfs_list") {
        const runId = requireRunId(params.runId);
        const client = await clientFor(ctx, runId);
        if (params.action === "vfs_write") {
          const artifactPath = requireString(params.path, "path");
          const result = await client.vfsPut(artifactPath, params.content ?? "");
          const text = result.ok
            ? `[VFS] ${artifactPath} ← ${(params.content ?? "").length} bytes` +
              (result.sha256 !== undefined ? ` sha256=${result.sha256.slice(0, 12)}` : "")
            : `[VFS REFUSED] ${artifactPath}：${result.error ?? "未知原因"}`;
          return { content: [{ type: "text", text }], details: { result } };
        }
        if (params.action === "vfs_read") {
          const artifactPath = requireString(params.path, "path");
          const result = await client.vfsGet(artifactPath);
          const text = result.ok ? result.content : `[VFS MISS] ${artifactPath}：${result.error ?? ""}`;
          return { content: [{ type: "text", text }], details: { result } };
        }
        const listed = await client.vfsList();
        const text = listed.ok
          ? listed.artifacts.length === 0
            ? "(本局 VFS 为空)"
            : listed.artifacts.join("\n")
          : `[VFS LIST FAILED] ${listed.error ?? ""}`;
        return { content: [{ type: "text", text }], details: { result: listed } };
      }

      if (params.action === "settle_close") {
        const runId = requireRunId(params.runId);
        const client = await clientFor(ctx, runId);
        const summary = await client.summary();
        if (!summary.available) {
          const text = `[ygg_session settle_close] 摘要不可用：${summary.degraded ?? "未知原因"}。拒绝盲收口。`;
          return { content: [{ type: "text", text }], details: { summary } };
        }
        const text = await settleAndClose(ctx, client, {
          runId,
          evolutionSummary: params.evolutionSummary ?? "",
          evolutionContent: params.evolutionContent ?? "",
          lesson: params.lesson ?? "",
          score: typeof params.score === "number" ? Math.max(1, Math.min(5, Math.round(params.score))) : 3,
          candidates: params.candidates ?? "",
          classes: params.classes ?? "",
          sessionId: params.sessionId ?? String(ctx.sessionManager.getSessionId() ?? runId),
        });
        return { content: [{ type: "text", text }], details: { summary: lastSummary } };
      }

      // kickoff（缺省分支）
      const runId = requireRunId(params.runId);
      const client = await clientFor(ctx, runId);
      const text = await kickoffSession(ctx, client, {
        runId,
        title: requireString(params.title, "title"),
        description: requireString(params.description, "description"),
        goalAuth: params.goalAuth ?? "",
        requirements: params.requirements ?? [],
      });
      return { content: [{ type: "text", text }], details: { summary: lastSummary } };
    },
  });

  // ⑤ 泳道编排面：活跃作业 → subagent → VFS → 探针 → 提案 → 等门。
  pi.registerTool({
    name: "ygg_run_lanes",
    label: "Ygg Run Lanes",
    description:
      "对所有处于 plan / development 的需求自动跑完泳道生命周期：分配泳道并建 worktree、" +
      "派 subagent 执行、产出归档进本局 VFS、跑 lane 探针、提 proposal 并 accept，推到 plan_gate / accept_gate。" +
      "不碰 gate（批准仍由协调者决定）。",
    promptSnippet:
      "Run every active plan/development job end-to-end with isolated worktrees and subagents, " +
      "archiving artifacts and landing proposals at plan_gate / accept_gate.",
    promptGuidelines: [
      "Use ygg_run_lanes after requirements are dispatched when the user wants the lanes executed automatically.",
      "ygg_run_lanes never approves gates: read the returned gates list and ask before calling ygg_session approve_gate.",
    ],
    reachesTools: () => ["Agent"],
    parameters: Type.Object({
      runId: Type.Optional(Type.String({ description: "本局标识（缺省用当前活跃局）" })),
      model: Type.Optional(Type.String({ description: "subagent 使用的模型（缺省读 YGG_SUBAGENT_MODEL）" })),
      thinking: Type.Optional(Type.String({ description: "subagent 的思考级别（缺省读 YGG_SUBAGENT_THINKING）" })),
      verification: Type.Optional(
        Type.Array(Type.String(), { description: "subagent brief 里声明的验证命令（缺省 cargo test --workspace）" }),
      ),
      maxLanes: Type.Optional(Type.Number({ description: "泳道上限（缺省 4，与 Mod 的 job cap 一致）" })),
    }),

    async execute(_toolCallId, params, signal, onUpdate, ctx) {
      const runId = requireRunId(params.runId ?? (activeRunId === "" ? undefined : activeRunId));
      const client = await clientFor(ctx, runId);
      const tuning = subagentTuning(ctx.cwd);
      const model = params.model ?? tuning.model;
      const thinking = params.thinking ?? tuning.thinking;

      const executeSubagent: ExecuteSubagent = async (request) => {
        if (signal?.aborted === true) {
          return { text: "", ok: false, error: "已取消（signal aborted）" };
        }
        const args: Record<string, unknown> = {
          description: request.description,
          prompt: request.prompt,
          subagent_type: "general-purpose",
          run_in_background: false,
        };
        if (request.model !== undefined) {
          args.model = request.model;
        }
        if (request.thinking !== undefined) {
          args.thinking = request.thinking;
        }
        // 续跑既有 subagent：它带着上一轮的全部记忆（被打回的原因 / 方案细节），
        // 比重新给一个自洽 brief 更准——brief 再全也只是背景，不是它自己的现场。
        if (isResumableAgentId(request.resume)) {
          args.resume = request.resume;
        }
        const outcome = await ctx.executeTool("Agent", args, {
          ...(signal !== undefined ? { signal } : {}),
        });
        const text = outcome.result.content
          .map((part) => (part.type === "text" ? part.text : ""))
          .join("\n");
        // 真实 agent id：先看结构化 `details`，再退回正文里的 `Agent ID: <id>`。
        // 失败时也照取——它就是「自愈续跑」要的那个 id。
        const agentId = extractAgentId(text, outcome.result.details);
        const result: SubagentResult = { text, ok: !outcome.isError };
        if (outcome.isError) {
          result.error = "Agent 工具返回错误";
        }
        if (agentId !== undefined) {
          result.agentId = agentId;
        }
        return result;
      };

      onUpdate?.({
        content: [{ type: "text", text: `ygg_run_lanes: 正在编排 ${runId} 的活跃作业…` }],
        details: {},
      });

      const result = await orchestrateActiveJobs(client, executeSubagent, {
        cwd: ctx.cwd,
        runId,
        ...(model !== undefined ? { model } : {}),
        ...(thinking !== undefined ? { thinking } : {}),
        ...(params.verification !== undefined ? { verification: params.verification } : {}),
        ...(typeof params.maxLanes === "number" ? { maxLanes: params.maxLanes } : {}),
        // 压力探针：本波落地（或硬刹车）后由下面一次交接 + 压缩 + 续跑接手。
        checkContextPressure: () => contextPressure(ctx),
        logger: (line) => {
          onUpdate?.({ content: [{ type: "text", text: line }], details: {} });
        },
      });

      lastSummary = result.summary ?? lastSummary;
      setStatus(ctx, lastSummary);
      const sections = [formatOrchestration(result)];
      if (result.contextBraked) {
        // 编排层已经刹车（硬：一个字都没派；软：本波已落在门上）并写好了交接单，
        // 这里只负责压缩 + 续跑——不再重写一份交接单。
        sections.push(
          await maybeHandoffAndCompact(pi, ctx, client, {
            force: true,
            note: "context brake（编排层已刹车）",
            ...(result.handoff !== undefined ? { write: result.handoff } : {}),
          }),
        );
      }
      const text = sections.join("\n\n");
      return { content: [{ type: "text", text }], details: { result } };
    },
  });
}

function requireString(value: string | undefined, field: string): string {
  if (value === undefined || value.trim() === "") {
    throw new Error(`ygg_session: 缺少必填参数 \`${field}\``);
  }
  return value;
}

function requireRunId(value: string | undefined): string {
  return requireString(value, "runId");
}
