/**
 * compaction.ts —— 上下文压力监测、稳定节点判定、结构化交接单
 *
 * 这一段解决一个具体的失效场景：一局 Yggdrasill 跑到中后段，Pi 的上下文要压缩。
 * 压缩本身不危险，危险的是**压完之后主 Agent 不知道自己跑到哪了**——它会重新猜
 * 一遍需求状态、重复派活、或者把「停在门禁上」误判成「可以继续推进」。所以这里
 * 把「当前局面」写成一份**机器可核对的交接单**（`artifacts/handoff-latest.md`），
 * 在三个时刻落盘并带回上下文：
 *
 *   ① 软刹车（≥60%）：本波 subagent 收完、提案 accept 完、落在 `plan_gate` /
 *      `accept_gate`（**稳定节点**）之后，再写交接单 + 压缩 + 自动续跑；
 *   ② 硬刹车（≥75%）：**不再起新一波** subagent，先写交接单 + 压缩 + 自动续跑；
 *   ③ 任何来源的压缩（人敲 `/compact`、阈值触发、溢出恢复）：`index.ts` 的
 *      `session_before_compact` 先把交接单写进 VFS，压完再把状态块带回上下文。
 *
 * ## 三条边界（刻意不做的）
 *
 * - **不判门禁**：本模块只说「现在是不是稳定节点」，批准/打回仍是人的决定；
 * - **不猜状态**：`summary.available === false` 时直接说「不知道」，不拿默认值
 *   假装自己是干净的边界（猜出来的交接单比没有更坏）；
 * - **不依赖 pi 运行时**：本文件不 import `@earendil-works/*`，`harness.test.ts`
 *   用普通对象就能整段测（`daemon.ts` 只在 type 位置被引用，运行时不可见）。
 */

import type { RunSummary } from "./daemon.ts";

// ───────────────────────────── 上下文压力 ─────────────────────────────

/** 默认中位上下文窗口（宿主没报 `contextWindow` 时的保守假想值）。 */
export const DEFAULT_CONTEXT_WINDOW = 128_000;
/**
 * 默认有效上下文窗口上限：模型元数据常报 1M（`1_000_000`），但网关/压缩水位
 * 的实际可用上限通常在 200k 左右；若按 1M 作分母，60%/75% 刹车点会被推迟到
 * 600k/750k 而彻底失效。
 */
export const DEFAULT_EFFECTIVE_WINDOW_CAP = 200_000;
/** 软刹车水位：到这就不起新一波 subagent，等本波落地再交接。 */
export const DEFAULT_SOFT_WATERMARK = 0.6;
/** 硬刹车水位：到这立刻停手，交接 + 压缩 + 续跑。 */
export const DEFAULT_HARD_WATERMARK = 0.75;

export type PressureLevel = "normal" | "soft_brake" | "hard_brake";

export interface ContextPressureInput {
  /** 已用 token（`ctx.getContextUsage().tokens`，未知时为 `null`）。 */
  usedTokens?: number | null;
  /** 上下文窗口（`ctx.model.contextWindow` / `ContextUsage.contextWindow`）。 */
  contextWindow?: number | null;
  /**
   * 有效上下文窗口上限（缺省 `DEFAULT_EFFECTIVE_WINDOW_CAP = 200_000`；
   * 显式传 `null` 或 `<= 0` 则不封顶，直接用 `rawWindow`）。
   */
  effectiveWindowCap?: number | null;
  /** 软刹车水位（默认 0.60）。 */
  softWatermark?: number;
  /** 硬刹车水位（默认 0.75）。 */
  hardWatermark?: number;
}

export interface ContextPressure {
  usedTokens: number;
  contextWindow: number;
  /** `usedTokens / contextWindow`（可 > 1：溢出也算硬刹车）。 */
  ratio: number;
  level: PressureLevel;
  /** 到软水位即 true：不要再起新一波 subagent（在手的那波要收完）。 */
  shouldBrakeSubagents: boolean;
  /** 到软水位即 true：**在稳定节点上**该写交接单并压缩。 */
  shouldHandoffAndCompact: boolean;
}

/**
 * 压力的只读视图：`orchestrator.ts` 与交接单只消费这几项（不关心水位怎么算的）。
 * `ContextPressure` 是它的超集，所以 `evaluateContextPressure()` 的返回值直接能递进来。
 */
export interface ContextPressureView {
  usedTokens: number;
  contextWindow: number;
  ratio: number;
  level: string;
  shouldBrakeSubagents: boolean;
}
function finitePositive(value: number | null | undefined): number | null {
  if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) {
    return null;
  }
  return value;
}

function watermark(value: number | undefined, fallback: number): number {
  const number = finitePositive(value);
  return number === null ? fallback : number;
}

export function parsePositiveIntEnv(raw: string | undefined | null): number | undefined {
  if (typeof raw !== "string" || raw.trim() === "") {
    return undefined;
  }
  const parsed = Number(raw.trim());
  return Number.isInteger(parsed) && parsed > 0 ? parsed : undefined;
}

/**
 * 上下文压力判定（纯函数）。
 *
 * 未知用量（`null` / `undefined` / 非有限数）→ 一律按 `normal` 处理：拿不到数字时
 * 刹车会让整局永远停在半路，而「不知道」不足以支撑一个破坏性动作。水位非法
 * （`0` / 负数 / `NaN`）退回默认值，`hard < soft` 时把 hard 抬到 soft（保持单调）。
 */
export function evaluateContextPressure(input: ContextPressureInput = {}): ContextPressure {
  const rawWindow = finitePositive(input.contextWindow) ?? DEFAULT_CONTEXT_WINDOW;
  const cap =
    input.effectiveWindowCap === undefined
      ? DEFAULT_EFFECTIVE_WINDOW_CAP
      : finitePositive(input.effectiveWindowCap);
  const contextWindow = cap !== null && rawWindow > cap ? cap : rawWindow;
  const soft = watermark(input.softWatermark, DEFAULT_SOFT_WATERMARK);
  const hard = Math.max(watermark(input.hardWatermark, DEFAULT_HARD_WATERMARK), soft);
  const known = finitePositive(input.usedTokens);
  const usedTokens = known === null ? 0 : Math.trunc(known);
  const ratio = known === null ? 0 : usedTokens / contextWindow;
  const level: PressureLevel =
    ratio >= hard ? "hard_brake" : ratio >= soft ? "soft_brake" : "normal";
  return {
    usedTokens,
    contextWindow,
    ratio,
    level,
    shouldBrakeSubagents: level !== "normal",
    shouldHandoffAndCompact: level !== "normal",
  };
}

/** `0.725 → "72.5%"`（交接单与叙事共用一种写法，避免两处不一致）。 */
export function formatPercent(ratio: number): string {
  const value = Number.isFinite(ratio) ? ratio : 0;
  return `${(value * 100).toFixed(1)}%`;
}

// ───────────────────────────── 稳定节点 ─────────────────────────────

/**
 * `summary` 是否停在**稳定叙事节点**上——判定顺序与理由一一对应（先看能不能信，
 * 再看工具在不在飞，最后看有没有落点）。
 *
 * 稳定 = `available` ∧ `pendingTools === 0` ∧ （有待审门 ∨ 无在跑作业 ∨ 有未解冲突）。
 * 只有 jobs 在飞、又没有门也没有冲突时才是「半路」——那时压缩会把在手的
 * subagent 结果丢进压缩摘要，而不是落在可复核的门禁/边界上。
 */
export function stableCheckpointReason(summary: RunSummary): string {
  if (!summary.available) {
    return "摘要不可用（不知道当前是不是稳定节点）";
  }
  if (summary.pendingTools > 0) {
    return `工具仍在飞（pendingTools=${summary.pendingTools}）`;
  }
  if (summary.gates.length > 0) {
    return `停在待审门上（${gateList(summary)}）`;
  }
  if (summary.jobs.length === 0) {
    return `干净阶段边界（stage=${summary.stage === "" ? "unknown" : summary.stage}，无在跑作业）`;
  }
  if (summary.conflicts.length > 0) {
    return `停在冲突边界上（${summary.conflicts.map((item) => item.id || "?").join(", ")}）`;
  }
  return `作业在飞且没有门禁（jobs=${summary.jobs.length}）：不是稳定节点`;
}

export function isStableCheckpoint(summary: RunSummary): boolean {
  if (!summary.available) {
    return false;
  }
  if (summary.pendingTools > 0) {
    return false;
  }
  return summary.gates.length > 0 || summary.jobs.length === 0 || summary.conflicts.length > 0;
}

function gateList(summary: RunSummary): string {
  return summary.gates.map((gate) => `${gate.id}@${gate.gate}`).join(", ") || "?";
}

// ───────────────────────────── 交接单 ─────────────────────────────

/** 交接单在 VFS 里的固定路径（每次覆盖：只保留**最新**局面，不留历史噪音）。 */
export const HANDOFF_ARTIFACT_PATH = "artifacts/handoff-latest.md";

export interface HandoffInput {
  runId: string;
  summary: RunSummary;
  /** 上下文压力（缺省 = 没探测过，交接单如实写「未探测」）。 */
  pressure?: ContextPressureView | null;
  /** 本局 VFS 已有的制品（`vfsList` 的输出，原样列出）。 */
  artifacts?: readonly string[];
  /** 触发原因（`soft_brake` / `hard_brake` / `manual` …）。 */
  reason?: string;
  /** 注入时间源（测试用；缺省 `new Date()`）。 */
  now?: Date;
}

export interface HandoffWrite {
  ok: boolean;
  path: string;
  markdown: string;
  bytes?: number;
  sha256?: string;
  error?: string;
}

/** 写 VFS 需要的最小面（`YggDaemon` 结构上满足；测试用普通对象即可）。 */
export interface HandoffDaemon {
  vfsPut(
    artifactPath: string,
    content: string,
  ): Promise<{ ok: boolean; bytes?: number; sha256?: string; error?: string }>;
}

/** 表格单元：禁换行、竖线转义（Markdown 表格的分隔符）、空值写 `-`。 */
function cell(text: string, max = 96): string {
  const flat = String(text ?? "")
    .replace(/[\r\n]+/g, " ")
    .replace(/\|/g, "\\|")
    .replace(/\s+/g, " ")
    .trim();
  if (flat === "") {
    return "-";
  }
  return flat.length <= max ? flat : `${flat.slice(0, Math.max(0, max - 1))}…`;
}

function contextLine(pressure: ContextPressureView | null | undefined): string {
  if (pressure === null || pressure === undefined) {
    return "未探测（宿主没有报 `getContextUsage()`）";
  }
  return (
    `${pressure.usedTokens} / ${pressure.contextWindow}` +
    ` (${formatPercent(pressure.ratio)})` +
    ` · 水位 ${pressure.level}` +
    (pressure.shouldBrakeSubagents ? " · 已刹车（不再起新一波 subagent）" : "")
  );
}

/** 活跃作业表（`summary.jobs` 本身就是「在跑的作业」，不在这里过滤阶段）。 */
function jobsSection(summary: RunSummary): string[] {
  const lines: string[] = ["## 2. 活跃作业", ""];
  if (summary.jobs.length === 0) {
    lines.push("(无在跑作业)");
    lines.push("");
    return lines;
  }
  lines.push("| id | stage | phase | rev | lane | branch | base | executor | planExecutor | title |");
  lines.push("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |");
  for (const job of summary.jobs) {
    lines.push(
      `| ${cell(job.id, 24)} | ${cell(job.stage, 24)} | ${cell(job.phase, 24)} | ${job.rev}` +
        ` | ${job.lane > 0 ? job.lane : "-"} | ${cell(job.branch, 64)} | ${cell(job.base, 40)}` +
        ` | ${cell(job.executor ?? "", 64)} | ${cell(job.planExecutor ?? "", 64)} | ${cell(job.title)} |`,
    );
  }
  lines.push("");
  return lines;
}

/** 待审门表（批准的时机与理由都写清：门是人的决定，不是自动续跑的入口）。 */
function gatesSection(summary: RunSummary): string[] {
  const lines: string[] = ["## 3. 待审门禁", ""];
  if (summary.gates.length === 0) {
    lines.push("(无待审门)");
    lines.push("");
    return lines;
  }
  lines.push("| id | gate | rev | title |");
  lines.push("| --- | --- | --- | --- |");
  for (const gate of summary.gates) {
    lines.push(`| ${cell(gate.id, 24)} | ${cell(gate.gate, 24)} | ${gate.rev} | ${cell(gate.title)} |`);
  }
  lines.push("");
  lines.push("> 门禁**不由本扩展批准**：压缩后照旧先读回执，再决定 approve / reject。");
  lines.push("");
  return lines;
}

function conflictsSection(summary: RunSummary): string[] {
  const lines: string[] = ["## 4. 未解冲突", ""];
  if (summary.conflicts.length === 0) {
    lines.push("(无未解冲突)");
    lines.push("");
    return lines;
  }
  lines.push("| id | peer | note |");
  lines.push("| --- | --- | --- |");
  for (const conflict of summary.conflicts) {
    lines.push(`| ${cell(conflict.id, 24)} | ${cell(conflict.peer, 48)} | ${cell(conflict.note)} |`);
  }
  lines.push("");
  lines.push("> 冲突只能靠 `requirement.resolve` 消解，且会阻塞派活与收口。");
  lines.push("");
  return lines;
}

function artifactsSection(input: HandoffInput): string[] {
  const listed = [...(input.artifacts ?? [])];
  if (!listed.some((entry) => entry.startsWith(HANDOFF_ARTIFACT_PATH))) {
    listed.unshift(`${HANDOFF_ARTIFACT_PATH} (本文)`);
  }
  const lines: string[] = ["## 5. 本局 VFS 制品", ""];
  for (const artifact of listed) {
    lines.push(`- ${cell(artifact, 160)}`);
  }
  lines.push("");
  return lines;
}

/**
 * 下一步的**精确命令**：压缩后主 Agent 照抄即可，不必重新推理一遍局面。
 * 顺序有意：先读交往单（重新对齐事实），再门禁（人的决定），再派活/收口。
 */
export function nextStepInstructions(runId: string, summary: RunSummary): string[] {
  const run = runId === "" ? "<runId>" : runId;
  const lines: string[] = [
    `1. 先读回交接单（就是本文）：\`ygg_session({ action: "vfs_read", runId: "${run}", path: "${HANDOFF_ARTIFACT_PATH}" })\``,
  ];
  let index = 2;
  if (!summary.available) {
    lines.push(
      `${index}. 摘要不可用——先看当前菜单再决定：\`ygg_session({ action: "dispatch", runId: "${run}", event: "map.show" })\``,
    );
    index += 1;
  }
  for (const gate of summary.gates) {
    lines.push(
      `${index}. \`${gate.id}\` 停在 \`${gate.gate}\` 门（rev ${gate.rev}）：先读上一条回执，` +
        `再 \`ygg_session({ action: "approve_gate", runId: "${run}", reqId: "${gate.id}", note: "<一句话理由>" })\`` +
        ` 或 \`reject_gate\`（打回要写清缺什么）。`,
    );
    index += 1;
  }
  const actionable = summary.jobs.filter(
    (job) => job.stage === "plan" || job.stage === "development" || job.stage === "analysis",
  );
  if (actionable.length > 0) {
    const ids = actionable.map((job) => `${job.id}/${job.stage}`).join(", ");
    lines.push(
      `${index}. 还有 ${actionable.length} 个作业可以推进（${ids}）：` +
        `\`ygg_run_lanes({ runId: "${run}" })\``,
    );
    index += 1;
  }
  if (summary.stage === "review") {
    lines.push(
      `${index}. 已到 \`review\`：\`ygg_session({ action: "settle_close", runId: "${run}", ` +
        `evolutionSummary: "<本轮复盘>", lesson: "<经验>", score: 3 })\``,
    );
    index += 1;
  } else if (summary.stage === "coordinate" && summary.jobs.length === 0 && summary.gates.length === 0) {
    lines.push(
      `${index}. 停在 \`coordinate\` 且无在跑作业：\`ygg_session({ action: "dispatch", runId: "${run}", event: "coordinate.submit" })\`` +
        `（未满足收口前置条件时会回一条拒绝，照它补齐）。`,
    );
    index += 1;
  }
  if (summary.conflicts.length > 0) {
    lines.push(
      `${index}. 冲突未解，先 \`ygg_session({ action: "dispatch", runId: "${run}", event: "requirement.resolve", payload: "${summary.conflicts[0]?.id ?? "r1"}|<处理说明>" })\`。`,
    );
    index += 1;
  }
  lines.push(
    `${index}. 不要重复派活：` + "`ygg_run_lanes` 已经是幂等的（同一泳道同一分支），但已完成的需求不必再跑一遍。",
  );
  index += 1;
  // (c) resume 等价承接：换 harness / 换上下文窗口后没有会话 id 可续，
  // 唯一能跨 resume 传递的就是制品。所以下一步必须点名「先读哪一份制品再续」。
  lines.push(
    `${index}. **换 harness / 换上下文窗口时的续跑承接（continuation）**：本次没有可精确 \`resume\` 的` +
      ` 会话 id 时，不要凭记忆重跑——先读 \`artifacts/handoff-latest.md\`（本文）与每个作业的` +
      ` \`artifacts/<rN>-<stage>-brief.md\`（上一位执行者**实际收到**的 prompt，逐字节落盘）、` +
      ` 再读 \`artifacts/<rN>-<stage>.md\`（它的产出），然后才 \`ygg_run_lanes\`。` +
      ` 每一份 brief 末尾都有 \`## 续跑承接（Resume Handover）\`，写明 basis / 前任执行者 / 未落地清单 / 硬约束 / 已归档制品。`,
  );
  return lines;
}

/**
 * 结构化交接单（Markdown）。
 *
 * 每一节都只写**可核对的事实**：run id / 阶段 / 用量 / 作业行 / 门禁 / 冲突 /
 * 制品 / 下一步命令。没有「大概」「可能」这种压完就失真的话。
 */
export function buildHandoffMarkdown(input: HandoffInput): string {
  const { runId, summary } = input;
  const now = input.now ?? new Date();
  const lines: string[] = [];
  lines.push("# Yggdrasill 运行交接单（handoff）");
  lines.push("");
  lines.push(`- run：\`${runId === "" ? "(unknown)" : runId}\``);
  lines.push(`- 阶段：\`${summary.available ? (summary.stage === "" ? "unknown" : summary.stage) : "unknown"}\``);
  lines.push(`- 上下文：${contextLine(input.pressure)}`);
  lines.push(`- 时间：${now.toISOString()}`);
  if (input.reason !== undefined && input.reason !== "") {
    lines.push(`- 触发：${cell(input.reason, 160)}`);
  }
  lines.push(
    `- 计数：jobs=${summary.jobs.length} · gates=${summary.gates.length}` +
      ` · conflicts=${summary.conflicts.length} · pendingProposals=${summary.pendingProposals}` +
      ` · pendingTools=${summary.pendingTools}`,
  );
  if (!summary.available) {
    lines.push("");
    lines.push(
      `> ⚠ 摘要不可用：${summary.degraded ?? "未知原因"}。下面的表格是**空**的，不代表没有作业——` +
        "先 `step map.show` 重新取一次现场。",
    );
  }
  lines.push("");
  lines.push(...jobsSection(summary));
  lines.push(...gatesSection(summary));
  lines.push(...conflictsSection(summary));
  lines.push(...artifactsSection(input));
  lines.push("## 6. 压缩后从这里继续（精确到命令）");
  lines.push("");
  lines.push(...nextStepInstructions(runId, summary));
  lines.push("");
  return lines.join("\n");
}

/** 写交接单进本局 VFS（`artifacts/handoff-latest.md`，每次覆盖）。 */
export async function writeHandoffToVfs(
  daemon: HandoffDaemon,
  input: HandoffInput,
): Promise<HandoffWrite> {
  const markdown = buildHandoffMarkdown(input);
  const result = await daemon.vfsPut(HANDOFF_ARTIFACT_PATH, markdown);
  if (!result.ok) {
    return {
      ok: false,
      path: HANDOFF_ARTIFACT_PATH,
      markdown,
      error: result.error ?? "vfs_put 失败",
    };
  }
  const out: HandoffWrite = { ok: true, path: HANDOFF_ARTIFACT_PATH, markdown };
  if (result.bytes !== undefined) {
    out.bytes = result.bytes;
  }
  if (result.sha256 !== undefined) {
    out.sha256 = result.sha256;
  }
  return out;
}

// ───────────────────────────── 压缩后的续跑消息 ─────────────────────────────

/** 自动续跑消息的 `customType`（渲染器与测试共用同一个字面量）。 */
export const HANDOFF_RESUME_CUSTOM_TYPE = "ygg-handoff-resume";

/**
 * 压缩完成后送回上下文的那条消息——它必须**自带全部局面**，因为压缩已经把
 * 之前的会话摘要掉了，模型能看到的只有这条消息 + 交接单。
 */
export function buildResumeMessage(runId: string, summary: RunSummary, markdown: string): string {
  const steps = nextStepInstructions(runId, summary);
  return [
    `[Yggdrasill 续跑] 本局 \`${runId === "" ? "(unknown)" : runId}\` 的上下文已在检查点压缩；` +
      "运行状态没有丢，下面这份交接单就是当前局面。",
    "",
    markdown.trim(),
    "",
    "---",
    "下一步（从这里接）：",
    steps.join("\n"),
    "",
    "纪律不变：门禁（approve / reject）是人的决定；回执整段读，不截断；一次只派一个事件。",
  ].join("\n");
}

/**
 * 我们主动压缩时给摘要模型的指令：状态块必须**逐字**留在摘要里。
 * 摘要模型会把它当成「必须保留的事实」，而不是可自由改写的叙述。
 */
export function buildCompactInstructions(runId: string, markdown: string): string {
  return [
    "这是一次 Yggdrasill 运行中的**检查点压缩**：会话正文可以概括，但运行状态不能丢。",
    `摘要开头必须原样保留下面这段运行状态块（run \`${runId}\` 的实际局面），一字不改；`,
    "然后再概括此前的工作叙事（做过什么、结论是什么、还有什么没做完）。",
    "",
    "===== 必须原样保留的状态块（开始） =====",
    markdown.trim(),
    "===== 必须原样保留的状态块（结束） =====",
  ].join("\n");
}
