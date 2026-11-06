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
import { checkToolCallFirewall } from "./firewall.ts";
import {
  orchestrateActiveJobs,
  type ExecuteSubagent,
  type OrchestrationResult,
} from "./orchestrator.ts";

const STATUS_KEY = "ygg-harness";
const DEFAULT_MOD_DIR = ".ygg/mods/pi-agent-seed";
const DEFAULT_RUNS_ROOT = ".ygg/runs";

/** 每个 runId 一个常驻客户端；`activeRunId` 是页脚展示的那一个。 */
const daemons = new Map<string, YggDaemon>();
let activeRunId = "";
let lastSummary: RunSummary | null = null;

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
async function clientFor(
  ctx: ExtensionContext,
  runId: string,
  overrides: { modDir?: string; runsRoot?: string } = {},
): Promise<YggDaemon> {
  const cwd = ctx.cwd;
  let client = daemons.get(runId);
  if (client === undefined) {
    client = new YggDaemon({
      cwd,
      runId,
      modDir: overrides.modDir ?? path.join(cwd, DEFAULT_MOD_DIR),
      runsRoot: overrides.runsRoot ?? path.join(cwd, DEFAULT_RUNS_ROOT),
      agentBin: agentBinPath(cwd),
    });
    daemons.set(runId, client);
  }
  activeRunId = runId;
  if (!client.alive) {
    await client.start();
    if (client.mode === "cli") {
      ctx.ui.notify(
        `ygg-agent 不支持 --daemon，已降级为单发模式：${client.degradedReason}`,
        "warning",
      );
    }
  }
  return client;
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

// ───────────────────────────── kickoff ─────────────────────────────

interface KickoffInput {
  runId: string;
  title: string;
  description: string;
  goalAuth: string;
  requirements: { title: string; description: string }[];
}

/**
 * 开局：起 daemon → 登记 goal 授权 → 切 goal → `task.intake` → 逐条
 * `requirement.add` → `requirement.dispatch`。
 *
 * 顺序是有意的：`goal.authorization` 先于 `play.mode=goal`（goal 局收口时
 * `review.note` 会校这条记录），两者都先于 `task.intake`。
 */
async function kickoffSession(
  ctx: ExtensionContext,
  client: YggDaemon,
  input: KickoffInput,
): Promise<string> {
  const entries: { label: string; receipt: StepReceipt }[] = [];

  if (input.goalAuth.trim() !== "") {
    const auth = await client.dispatch("config.set", `goal.authorization=${input.goalAuth.trim()}`);
    entries.push({ label: "config.set goal.authorization", receipt: auth });
  }
  const mode = await client.dispatch("config.set", "play.mode=goal");
  entries.push({ label: "config.set play.mode=goal", receipt: mode });

  const intake = await client.dispatch("task.intake", `${input.title}|${input.description}`);
  entries.push({ label: "task.intake", receipt: intake });

  for (const requirement of input.requirements) {
    const added = await client.dispatch(
      "requirement.add",
      `${requirement.title}|${requirement.description}`,
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
  const archiveArgs = [
    `parent=${input.sessionId}`,
    `batch=${input.runId}`,
  ].join(";");
  const archivedSessions = await client.dispatch(
    "tool.call",
    `harness.archive_sessions|${archiveArgs}`,
  );
  entries.push({ label: "tool.call harness.archive_sessions", receipt: archivedSessions });

  const evolutionPayload = [
    artifact,
    input.evolutionSummary.trim() === "" ? "(no candidates this round)" : input.evolutionSummary,
    `c=${input.candidates.trim() === "" ? "0/0" : input.candidates.trim()}`,
    ...(input.classes.trim() === "" ? [] : [`cls=${input.classes.trim()}`]),
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
  for (const job of result.jobs) {
    lines.push(
      `### ${job.id} · ${job.stage} → ${job.expectedGate} gate` +
        ` | lane ${job.lane > 0 ? job.lane : "-"} | ${job.branch === "" ? "-" : job.branch}`,
    );
    if (job.worktreePath !== null) {
      lines.push(`worktree: ${job.worktreePath}`);
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
    activeRunId = "";
    lastSummary = null;
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
          lines.push(
            `  - ${job.id} ${job.stage} rev${job.rev} lane=${job.lane} ${job.branch === "" ? "-" : job.branch}`,
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
        activeRunId = "";
        lastSummary = null;
        ctx.ui.setStatus(STATUS_KEY, undefined);
        ctx.ui.notify(`ygg: 已关闭 ${stopped.length} 个守护进程（${stopped.join(", ") || "none"}）`, "info");
        return;
      }
      ctx.ui.notify(`ygg: 未知子命令「${sub}」（可用：status / stop）`, "warning");
    },
  });

  // ④ 会话面：kickoff / step / gate / vfs / settle_close。
  pi.registerTool({
    name: "ygg_session",
    label: "Ygg Session",
    description:
      "驱动 Yggdrasill 会话（`.ygg/mods/pi-agent-seed`）：开局、单步派发、过门禁、读写本局 VFS、收口结算。" +
      "所有回执的 narrative 会整段返回（不截断）。",
    promptSnippet:
      "Drive one Yggdrasill run: kickoff (goal mode + intake + requirements), step (dispatch any event), " +
      "approve_gate / reject_gate (plan_gate / accept_gate), vfs_write / vfs_read / vfs_list, settle_close.",
    promptGuidelines: [
      "Use ygg_session kickoff when the user asks to start a Yggdrasill run; pass runId, title, description, goalAuth and requirements.",
      "Use ygg_session step to dispatch exactly one event and read the full receipt; never batch events in a shell loop.",
      "Use ygg_session approve_gate only after reading the receipt that raised the gate — approving is a human/coordinator decision.",
      "Use ygg_session settle_close when every requirement is done and the run is in review; it archives the evolution review and settles.",
    ],
    parameters: Type.Object({
      action: StringEnum([
        "kickoff",
        "step",
        "approve_gate",
        "reject_gate",
        "vfs_write",
        "vfs_read",
        "vfs_list",
        "settle_close",
      ] as const, { description: "要执行的会话动作" }),
      runId: Type.Optional(Type.String({ description: "本局标识（同一 runId 复用同一个守护进程）" })),
      title: Type.Optional(Type.String({ description: "kickoff：本局标题（task.intake 的第一段）" })),
      description: Type.Optional(Type.String({ description: "kickoff：本局描述（task.intake 的第二段）" })),
      goalAuth: Type.Optional(
        Type.String({ description: "kickoff：goal 授权的一句话说明（写入 config.set goal.authorization）" }),
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
      event: Type.Optional(Type.String({ description: "step：要派发的事件名（如 requirement.dispatch）" })),
      payload: Type.Optional(
        Type.Union([Type.String(), Type.Number()], {
          description: "step：事件载荷（字符串或 JSON 数字；菜单选择必须是数字）",
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

      if (params.action === "step") {
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
        const text = [
          formatReceipt(`${event} ${payloadText(params.payload)}`, receipt),
          statusLine(runId, lastSummary),
        ].join("\n\n");
        return { content: [{ type: "text", text }], details: { receipt, summary: lastSummary } };
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
        const outcome = await ctx.executeTool("Agent", args, {
          ...(signal !== undefined ? { signal } : {}),
        });
        const text = outcome.result.content
          .map((part) => (part.type === "text" ? part.text : ""))
          .join("\n");
        if (outcome.isError) {
          return { text, ok: false, error: "Agent 工具返回错误" };
        }
        return { text, ok: true };
      };

      onUpdate?.({
        content: [{ type: "text", text: `ygg_run_lanes: 正在编排 ${runId} 的活跃作业…` }],
        details: {},
      });

      const result = await orchestrateActiveJobs(client, executeSubagent, {
        cwd: ctx.cwd,
        ...(model !== undefined ? { model } : {}),
        ...(thinking !== undefined ? { thinking } : {}),
        ...(params.verification !== undefined ? { verification: params.verification } : {}),
        ...(typeof params.maxLanes === "number" ? { maxLanes: params.maxLanes } : {}),
        logger: (line) => {
          onUpdate?.({ content: [{ type: "text", text: line }], details: {} });
        },
      });

      lastSummary = result.summary ?? lastSummary;
      setStatus(ctx, lastSummary);
      const text = formatOrchestration(result);
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
