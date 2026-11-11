/**
 * index.ts —— `.pi/extensions/ygg-pi-harness` 的装配入口
 *
 * 这一层**只做装配**，不含业务判定：
 * - `session_start` / `session_shutdown`：探测 `.ygg/bin/egg`、重建/关闭客户端；
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
  AUTO_WAVE_CUSTOM_TYPE,
  AutoWatchdog,
  clampAutoIntervalMs,
  collectKey,
  parseAutoWake,
} from "./watchdog.ts";
import {
  analyzeArchiveVerdict,
  archiveSessionsSegment,
  archiveVerdictNarrative,
} from "./archive.ts";
export {
  analyzeArchiveVerdict,
  archiveSessionsSegment,
  archiveVerdictNarrative,
  type ArchiveVerdict,
} from "./archive.ts";
import {
  DEFAULT_ACTOR_ID,
  extractAgentId,
  isResumableAgentId,
  orchestrateActiveJobs,
  sanitizeSegment,
  type ExecuteSubagent,
  type OrchestrationResult,
  type SubagentResult,
} from "./orchestrator.ts";

const STATUS_KEY = "ygg-harness";
const DEFAULT_MOD_DIR = ".ygg/mods/agent-seed";
const DEFAULT_RUNS_ROOT = ".ygg/runs";

/** 每个 runId 一个常驻客户端；`activeRunId` 是页脚展示的那一个。 */
const daemons = new Map<string, YggDaemon>();
/** 并发启动锁：同一 runId 正在 `start()` 时复用同一个 Promise，防并发重复拉起。 */
const daemonsInflight = new Map<string, Promise<YggDaemon>>();
let activeRunId = "";
let lastSummary: RunSummary | null = null;
// ── r1 看门狗状态（每会话单实例；`/ygg auto on|off|status` 管理，默认关） ──
let autoWatchdog: AutoWatchdog | null = null;
/** 已发出收波唤醒、等 `ygg_run_lanes` 消费的波次票（消费后写波次日志 + 结算预算）。 */
let autoWavePending: { runId: string; wave: number; reason: string; keys: string[] } | null = null;
let lastWaveLogPath = "";

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
  return path.join(cwd, ".ygg", "bin", "egg");
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
  const auto =
    autoWatchdog !== null && autoWatchdog.state.enabled
      ? ` · auto:on(波次=${autoWatchdog.state.waveCount})`
      : "";
  const line =
    statusLine(activeRunId === "" ? "(no run)" : activeRunId, summary ?? lastSummary) + auto;
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
          `egg 不支持 --daemon，已降级为单发模式：${client.degradedReason}`,
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
  /**
   * 只读观测：**只报告水位与稳定节点，绝不压缩**（`dispatch` 分支用）。
   *
   * 为什么 `dispatch` 只配观测而不配压缩：见 `docs/DESIGN.md` §3.4「`dispatch` 与压缩的边界」。
   * 一句话—— `dispatch` 承诺「一次派发 = 一次回合 + 一份完整回执」，而压缩会 abort 当前
   * 回合，两者不可能同时成立。压缩只允许经两个显式入口发生：`handoff_compact`（force）
   * 与 `ygg_run_lanes` 的编排层刹车。
   */
  reportOnly?: boolean;
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

  // 只读观测模式：报告完就返回。这里**不调用** `ctx.compact`，因此当前回合不会被打断，
  // 上面那份回执（narrative 整段 + statusLine）一定到得了模型。
  if (options.reportOnly === true) {
    lines.push(
      pressure.shouldHandoffAndCompact
        ? "[handoff] 未压缩：`dispatch` 只报告水位、不触发压缩。" +
            "上下文确实吃紧时显式调 `handoff_compact`（它会写交接单、压缩并自动续跑）。"
        : "[handoff] 未压缩：水位未到（< 软水位）。",
    );
    return lines.join("\n");
  }

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

// ───────────────────────────── dispatch（事件直通）─────────────────────────────

/** `action=dispatch` 的返回形状（与 `registerTool` 的 `execute` 返回值同形）。 */
export interface DispatchSessionResult {
  content: { type: "text"; text: string }[];
  details: { receipt: StepReceipt; summary: RunSummary | null };
}

/**
 * `action=dispatch` 的完整语义：**派发一次事件，返回该次派发的完整回执**。
 *
 * 契约见 `docs/DESIGN.md` §3.4「`dispatch` 与压缩的边界」。这里对上下文压力
 * 只做 `reportOnly` 观测——**绝不**调用 `ctx.compact()`：
 *
 * - `dispatch` 承诺「一次派发 = 一次回合 + 一份完整回执」（AGENTS §1.3：回执整段可见）；
 * - `ctx.compact()` 会 abort 当前回合，于是上面拼好的 narrative + statusLine 到不了模型。
 *
 * 两个承诺不能同时成立，所以压缩只留给显式入口：`action=handoff_compact`（force）
 * 与 `ygg_run_lanes` 的编排层刹车。
 */
export async function dispatchSessionEvent(
  pi: ExtensionAPI,
  ctx: ExtensionContext,
  client: YggDaemon,
  runId: string,
  event: string,
  payload: string | number | undefined,
): Promise<DispatchSessionResult> {
  // r3 互证：主会话面派发统一自报身份（actor = 扩展占位 handle；actorSession = 原生会话 id）。
  // 会话 id 是不透明审计字段，取不到就缺席（绝不阻塞派发）。
  const sessionId = String(ctx.sessionManager?.getSessionId?.() ?? "").trim();
  const receipt = await client.dispatch(event, payload, {
    actor: DEFAULT_ACTOR_ID,
    ...(sessionId !== "" ? { actorSession: sanitizeSegment(sessionId, 128) } : {}),
  });
  if (receipt.summary !== undefined) {
    lastSummary = receipt.summary;
  } else {
    lastSummary = (await client.summary()) ?? lastSummary;
  }
  setStatus(ctx, lastSummary);
  const handoff = await maybeHandoffAndCompact(pi, ctx, client, { reportOnly: true });
  const text = [
    formatReceipt(`${event} ${payloadText(payload)}`, receipt),
    statusLine(runId, lastSummary),
    handoff,
  ].join("\n\n");
  return { content: [{ type: "text", text }], details: { receipt, summary: lastSummary } };
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
  //
  // **不再编造 0/0**：结论先分三态（见 [`analyzeArchiveVerdict`]）。工具的 `ok` 不是
  // 证据——`generic` 适配器也回 `ok`，而它的 `dest=generic:no-session-store:<batch>`
  // 只能说明「没有库可查」。所以要按**叙事里的三元组**判，而不是按「工具跑过」判。
  const archiveArgs = [
    `parent=${input.sessionId}`,
    `batch=${input.runId}`,
  ].join(";");
  const archivedSessions = await client.dispatch(
    "tool.call",
    `harness.archive_sessions|${archiveArgs}`,
  );
  entries.push({ label: "tool.call harness.archive_sessions", receipt: archivedSessions });
  const archiveVerdict = analyzeArchiveVerdict(archivedSessions.narrative ?? []);
  entries.push({
    label: "archive verdict",
    receipt: {
      ok: archiveVerdict.kind === "swept",
      narrative: [archiveVerdictNarrative(archiveVerdict)],
      pendingChoice: null,
      tools: [],
      raw: { ...archiveVerdict },
    },
  });
  // 非证据态留一份诊断制品（由本次 dispatch 携带，不违反「无事件不落盘」）。
  if (archiveVerdict.kind !== "swept") {
    const diagnostic = [
      "# 会话归档结论（非证据）",
      "",
      `- run：\`${input.runId}\``,
      `- sessionId：\`${input.sessionId}\``,
      `- verdict：\`${archiveVerdict.kind}\``,
      `- ${archiveVerdictNarrative(archiveVerdict)}`,
      "",
      "## 这意味着什么",
      "",
      "本次**没有**可核对的本局会话库落点（harness 未认领 / 未查成 / 没有库）。",
      "台账写 `unsettled:<kind>` 是为了让 `review.note` 不被卡死，同时**自证**：",
      "这不是「查空了」，而是「没有库可查 / 没查成」。",
      "",
      "## 回执原文（整段保留）",
      "",
      ...(archivedSessions.narrative ?? []).map((line) => `    ${line}`),
    ].join("\n");
    const diagWrite = await client.vfsPut("artifacts/sessions-settlement.md", diagnostic);
    entries.push({
      label: "vfs_put artifacts/sessions-settlement.md",
      receipt: {
        ok: diagWrite.ok,
        narrative: diagWrite.ok
          ? [`[VFS] artifacts/sessions-settlement.md ← ${diagnostic.length} bytes`]
          : [`[VFS REFUSED] artifacts/sessions-settlement.md：${diagWrite.error ?? "未知原因"}`],
        pendingChoice: null,
        tools: [],
        raw: { ...diagWrite },
      },
    });
  }

  const sessionsSegment = archiveSessionsSegment(archiveVerdict);
  const evolutionPayload = [
    artifact,
    input.evolutionSummary.trim() === "" ? "(no candidates this round)" : input.evolutionSummary,
    `c=${input.candidates.trim() === "" ? "0/0" : input.candidates.trim()}`,
    ...(input.classes.trim() === "" ? [] : [`cls=${input.classes.trim()}`]),
    ...(sessionsSegment === null ? [] : [sessionsSegment]),
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
    }  }
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

// ───────────────────────────── 波次日志（r1 看门狗） ─────────────────────────────

/** 波次日志路径：`artifacts/auto-wave-<NNNN>.md`（四位序号；与交接单同类的宿主侧归档）。 */
export function waveLogPath(wave: number): string {
  return `artifacts/auto-wave-${String(Math.max(1, wave)).padStart(4, "0")}.md`;
}

/**
 * 波次日志 markdown：触发原因 + 逐作业逐回执的 narrative **全文**（永不截断）+ 停点判定。
 * 这是「三不之永不截断回执」在无人值守场景的落点：主会话不在场时，下个在场回合
 * 经交接单指针完整可达（DESIGN §3.5）。
 */
export function buildWaveLogMarkdown(input: {
  runId: string;
  wave: number;
  reason: string;
  result: OrchestrationResult;
  now?: Date;
}): string {
  const { runId, wave, reason, result } = input;
  const lines: string[] = [
    `# 自动收波波次日志 ${String(Math.max(1, wave)).padStart(4, "0")}`,
    "",
    `- run：\`${runId === "" ? "(unknown)" : runId}\``,
    `- 触发：${reason === "" ? "(未记录)" : reason}`,
    `- 时间：${(input.now ?? new Date()).toISOString()}`,
    `- 作业数：${result.jobs.length}（braked=${result.contextBraked}）`,
    "",
  ];
  if (result.jobs.length === 0) {
    lines.push("(本波没有作业落地——编排拒绝或无活跃作业，原因见上方触发段)");
    lines.push("");
  }
  for (const job of result.jobs) {
    lines.push(
      `## ${job.id}（${job.stage} rev${job.rev > 0 ? job.rev : 1}）lane=${job.lane} ${job.branch}`,
      "",
    );
    if (job.error !== undefined) {
      lines.push(`> ⚠ 作业错误：${job.error}`, "");
    }
    for (const entry of job.receipts) {
      lines.push(`### ${entry.label}`, "");
      for (const line of entry.receipt.narrative) {
        lines.push(line);
      }
      lines.push("");
    }
  }
  lines.push("## 停点判定", "");
  if (result.gates.length === 0) {
    lines.push("(无已升起门禁——本波落在待决提案或无作业)");
  } else {
    for (const gate of result.gates) {
      lines.push(`- ${gate.id} 停在 \`${gate.gate}\`（rev ${gate.rev}）${gate.title}`);
    }
    lines.push("");
    lines.push("> 门禁裁决是人的决定：看门狗不批门禁，等下个在场者处理。");
  }
  lines.push("");
  return lines.join("\n");
}

// ───────────────────────────── 扩展本体 ─────────────────────────────

export default function yggHarnessExtension(pi: ExtensionAPI): void {
  const subagentTuning = (cwd: string): SubagentSettings => subagentSettings(readEnvFiles(cwd));

  // ① 生命周期：只在 session_start 之后起资源（扩展工厂可能被永不建会话的调用加载）。
  pi.on("session_start", async (_event, ctx) => {
    lastSummary = null;
    if (!hasAgentBinary(ctx.cwd)) {
      ctx.ui.setStatus(STATUS_KEY, `[Ygg: unavailable | ${path.join(".ygg", "bin", "egg")} missing]`);
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
    // r1：看门狗随会话终局（进程内定时器，不跨会话；下一会话 `/ygg auto on` 重开）。
    autoWatchdog?.stop("会话关闭");
    autoWatchdog = null;
    autoWavePending = null;
    lastWaveLogPath = "";
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
    description: "Yggdrasill 会话面：`/ygg status` 查当前处境，`/ygg auto on|off|status` 自动收波看门狗，`/ygg stop` 关闭常驻守护进程",
    handler: async (args, ctx) => {
      const sub = (args ?? "").trim();
      // ③' r1 看门狗：`/ygg auto on|off|status`（默认关；on 需已有活跃 run）。
      if (sub.startsWith("auto")) {
        const arg = sub.slice("auto".length).trim();
        const client = activeRunId === "" ? undefined : daemons.get(activeRunId);
        if (arg === "on") {
          if (client === undefined) {
            ctx.ui.notify("ygg auto: 没有活跃 run（先 ygg_session kickoff）", "warning");
            return;
          }
          if (autoWatchdog === null) {
            const env = readEnvFiles(ctx.cwd);
            autoWatchdog = new AutoWatchdog({
              client,
              sender: pi,
              runId: client.runId,
              isIdle: () => ctx.isIdle(),
              pressure: () => contextPressure(ctx),
              wakeEnabled: parseAutoWake(env.get("YGG_AUTO_WAKE")),
              intervalMs: clampAutoIntervalMs(env.get("YGG_AUTO_INTERVAL_MS")),
              logger: (line) => ctx.ui.notify(`ygg: ${line}`, "info"),
            });
          }
          autoWatchdog.start();
          setStatus(ctx, lastSummary);
          ctx.ui.notify(`ygg auto: on（间隔 ${autoWatchdog.interval}ms；/ygg auto status 看判定）`, "info");
          return;
        }
        if (arg === "off") {
          autoWatchdog?.stop("手动 /ygg auto off");
          setStatus(ctx, lastSummary);
          ctx.ui.notify("ygg auto: off", "info");
          return;
        }
        const st = autoWatchdog?.state;
        const lines = [
          `auto: ${st?.enabled === true ? "on" : "off"} · wake=${parseAutoWake(readEnvFiles(ctx.cwd).get("YGG_AUTO_WAKE")) ? "on" : "off"}`,
          st === undefined
            ? "(从未启动——`/ygg auto on` 开启)"
            : `波次=${st.waveCount} · 连续全败=${st.consecutiveWaveFailures} · 停车表=${Object.keys(st.parked).length} 项`,
          `最近判定：${autoWatchdog?.lastReason ?? "-"}`,
          autoWatchdog?.isWaveArmed === true && autoWavePending !== null
            ? `波票未消费：run=${autoWavePending.runId} wave=${autoWavePending.wave}（等 ygg_run_lanes）`
            : lastWaveLogPath !== ""
              ? `最近波次日志：${lastWaveLogPath}`
              : "(尚无波次日志)",
        ];
        ctx.ui.notify(lines.filter((line) => line !== "").join("\n"), "info");
        return;
      }
      if (sub === "" || sub === "status") {
        const client = activeRunId === "" ? undefined : daemons.get(activeRunId);
        if (client === undefined) {
          ctx.ui.notify(
            hasAgentBinary(ctx.cwd)
              ? "ygg: 还没有活跃的 run（先调 `ygg_session` 的 kickoff）"
              : "ygg: 找不到 .ygg/bin/egg（先在仓库根构建宿主）",
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

  // ④ 会话面：kickoff / dispatch / gate / vfs / settle_close。
  pi.registerTool({
    name: "ygg_session",
    label: "Ygg Session",
    description:
      "驱动 Yggdrasill 会话（`.ygg/mods/agent-seed`）：开局、单步派发、过门禁、读写本局 VFS、收口结算。" +
      "所有回执的 narrative 会整段返回（不截断）。" +
      "`action` 是唯一路由参数：其中 `dispatch` 是**透传任意底层事件的逃生舱**，" +
      "其余 8 个 action（kickoff / approve_gate / reject_gate / vfs_* / handoff_compact / settle_close）" +
      "是封装好载荷位次的语义化入口——两者不是平级冗余，而是「专用入口 + 通用兜底」的两层结构" +
      "（契约见 `docs/DESIGN.md` §3.4）。",
    promptSnippet:
      "Drive one Yggdrasill run: kickoff (goal mode + intake + requirements), dispatch (dispatch any event), " +
      "approve_gate / reject_gate (plan_gate / accept_gate), vfs_write / vfs_read / vfs_list, " +
      "handoff_compact (checkpoint handoff + compaction + auto-resume), settle_close.",
    promptGuidelines: [
      "Use ygg_session kickoff when the user asks to start a Yggdrasill run; pass runId, title, description, goalAuth and requirements.",
      "Use ygg_session dispatch to dispatch exactly one event and read the full receipt; never batch events in a shell loop. `dispatch` is the escape hatch for events without a dedicated action (guide.show / map.show / config.set / requirement.resolve / tool.call). It never compacts: when the context is tight, call handoff_compact explicitly.",
      "Use ygg_session approve_gate only after reading the receipt that raised the gate — approving is a human/coordinator decision.",
      "Use ygg_session handoff_compact when the context is getting long: it writes artifacts/handoff-latest.md, compacts, and resumes the run automatically.",
      "Use ygg_session settle_close when every requirement is done and the run is in review; it archives the evolution review and settles.",
    ],
    parameters: Type.Object({
      action: StringEnum([
        "kickoff",
        "dispatch",
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

      if (params.action === "dispatch") {
        const runId = requireRunId(params.runId);
        const client = await clientFor(ctx, runId);
        const event = requireString(params.event, "event");
        return await dispatchSessionEvent(pi, ctx, client, runId, event, params.payload);
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
      // r1：看门狗触发的波（消费波票）→ 波次日志（narrative 全量）+ 预算结算 + 紧凑状态块。
      const wave =
        autoWavePending !== null && autoWavePending.runId === runId && result !== undefined
          ? autoWavePending
          : null;
      if (wave !== null) {
        autoWavePending = null;
        lastWaveLogPath = waveLogPath(wave.wave);
        const wavePut = await client.vfsPut(
          lastWaveLogPath,
          buildWaveLogMarkdown({ runId, wave: wave.wave, reason: wave.reason, result }),
        );
        autoWatchdog?.recordWave(
          result.jobs.map((job) => ({ key: collectKey(job), ok: job.error === undefined })),
        );
        setStatus(ctx, lastSummary);
        // 紧凑状态块：追加到本回合结束后（triggerTurn:false——不替用户开回合）。
        try {
          pi.sendMessage(
            {
              customType: AUTO_WAVE_CUSTOM_TYPE,
              content:
                `[ygg 看门狗] 第 ${wave.wave} 波已落地：${result.jobs.length} 作业；` +
                `停点=${result.gates.map((g) => `${g.id}:${g.gate}`).join(", ") || "无"}；` +
                `波次日志 ${lastWaveLogPath}${wavePut.ok ? "" : `（VFS 写入失败：${wavePut.error ?? "?"}）`}`,
              display: true,
            },
            { triggerTurn: false },
          );
        } catch {
          // sendMessage 面在个别宿主形态下可能缺席：波次日志已落 VFS，状态块是锦上添花。
        }
      }
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
