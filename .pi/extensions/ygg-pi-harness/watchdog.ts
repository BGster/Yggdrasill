/**
 * watchdog.ts —— 自动收波看门狗（r1/D-MA16）
 *
 * 一句话：会话空闲期进程内定时器轮询 daemon `summary`，用「可收波谓词」发现待处理
 * 作业后**原样复用** `ygg_run_lanes` 跑完整波（claim → subagent → 归档 → 探针 →
 * proposal.submit → proposal.accept），天然停在门禁/待决提案前；同一定时器兼管
 * daemon 存活（每 tick 一次 `summary()`，传输失败由 `daemon.ts` 既有的单次重启
 * 逻辑自愈）。宿主零改动。
 *
 * ## spike 结论（r1 实测，2026-11-11，`--mode rpc` 空闲探针）
 *
 * `ctx.executeTool` 只存在于 ExtensionToolContext（工具 `execute()` 内），
 * **回合外的 handler ctx 上是 `undefined`**——空闲定时器无法直接编排。
 * 降级路径（方案 §5 预案，spike 实证可用）：定时器经工厂级
 * `pi.sendMessage({triggerTurn: true})` 拉起一个**确定性自唤醒回合**，让主代理
 * 调 `ygg_run_lanes`——事件链与回执与手发逐字节同构，代价是每波多耗一个模型回合。
 *
 * ## 三条边界（刻意不做的，与全扩展「三不」同源）
 *
 * - **永不批门禁**：本模块产出的动作只有「唤醒收波回合」与「goal 模式门禁唤醒」，
 *   事件全集就是 `ygg_run_lanes` 既有链 ⊆ `{tool.call(worktree_claim|lane_probe),
 *   proposal.submit, proposal.accept}`——`gate.approve`/`gate.reject` 不在看门狗
 *   任何路径上（guide 模式的 accept 被票拒 = 权威停点，不重试）；
 * - **永不越过冲突**：`conflicts` 非空 → 谓词直接 `none`；
 * - **不是第三个压缩入口**：硬水位（`pressure().level === "hard_brake"`）只抑制
 *   派波，软水位照跑——压缩仍只有 `handoff_compact` 与编排刹车两个显式入口。
 *
 * ## 防失控预算（无人值守的风险闸，纯核进测试面）
 *
 * - per `(reqId, rev, stage)` 连续 `WAVE_FAILURES_PARK`（2）次整波失败 → 停车；
 *   rev 变化（amend/reject 打回）换键自动复位；
 * - 全局连续 `WAVE_FAILURES_AUTO_OFF`（3）波全败 → 看门狗自关（fail-closed，
 *   `/ygg auto status` 可见），交接单如实记录。
 */

import type { RunSummary, SummaryGate, SummaryJob, YggDaemon } from "./daemon.ts";
import type { OrchestrationResult } from "./orchestrator.ts";

// ───────────────────────────── 纯核：状态与谓词 ─────────────────────────────

/** per-job 停车阈值：同一 (req, rev, stage) 连续整波失败次数。 */
export const WAVE_FAILURES_PARK = 2;
/** 全局自关阈值：连续全败波数。 */
export const WAVE_FAILURES_AUTO_OFF = 3;

/** 看门狗可序列化状态（KV 形状：不持有定时器 / 客户端 / 闭包）。 */
export interface AutoPilotState {
  enabled: boolean;
  /** 已触发（发出唤醒）的波次计数。 */
  waveCount: number;
  /** 全局连续全败波数（任一作业成功即清零）。 */
  consecutiveWaveFailures: number;
  /** 停车表：`"<reqId>:<rev>:<stage>"` → 连续整波失败次数。 */
  parked: Record<string, number>;
}

export function initialAutoPilotState(): AutoPilotState {
  return { enabled: false, waveCount: 0, consecutiveWaveFailures: 0, parked: {} };
}

/** 停车表键（rev 进键：amend/reject 打回后 rev 变化 = 自然复位）。 */
export function collectKey(job: Pick<SummaryJob, "id" | "rev" | "stage">): string {
  return `${job.id}:${job.rev > 0 ? job.rev : 1}:${job.stage === "" ? "plan" : job.stage}`;
}

/**
 * 已交付未验收：`summary.subagentStatus` 以 `delivered` 开头（render.lua:156 的
 * `delivered:p<N>(<exec>)`）。这一位是防重复收波的关键判别——提案已交付，accept
 * 被票拒（guide）后作业停在 delivered 态，**拒绝即设计停点，不重试**。
 */
export function isDeliveredJob(job: SummaryJob): boolean {
  return (job.subagentStatus ?? "").startsWith("delivered");
}

/** 看门狗决策：`wave` = 拉起收波回合；`wake` = goal 门禁唤醒；`none` = 本 tick 不动。 */
export interface AutoDecision {
  kind: "wave" | "wake" | "none";
  /** 一句话原因（进 tick 日志与波次日志的「触发原因」段）。 */
  reason: string;
  /** `kind === "wave"` 时的可收波作业清单（保持 summary.jobs 原序）。 */
  jobs: SummaryJob[];
  /** `kind === "wake"` 时的待审门禁（取 gates[0]）。 */
  gate: SummaryGate | null;
}

/**
 * 可收波谓词（摘要级，纯函数）：
 *
 * `summary.available ∧ lifecycle === "resident" ∧ conflicts === [] ∧
 *  ∃ job ∈ collectibleJobs(summary): ¬delivered ∧ ¬parked`
 *
 * - `lifecycle` 缺失（旧 mod）→ 保守不收（r9 之前的 mod 没有常驻语义）；
 * - `delivered:*` 排除天然防重复提交；
 * - 门禁/待决提案本身不收波——波只由 plan/development 作业驱动，落在门上就停。
 */
export function nextAutoAction(
  summary: RunSummary,
  state: AutoPilotState,
  opts?: { wakeEnabled?: boolean },
): AutoDecision {
  if (!summary.available) {
    return { kind: "none", reason: "摘要不可用（不猜）", jobs: [], gate: null };
  }
  if (summary.lifecycle !== "resident") {
    return {
      kind: "none",
      reason: `lifecycle=${summary.lifecycle ?? "(缺失)"} ≠ resident（旧 mod，保守不收）`,
      jobs: [],
      gate: null,
    };
  }
  if (summary.conflicts.length > 0) {
    return {
      kind: "none",
      reason: `存在 ${summary.conflicts.length} 个未决冲突（永不越过）`,
      jobs: [],
      gate: null,
    };
  }
  const collectible = collectJobs(summary, state);
  if (collectible.length > 0) {
    return {
      kind: "wave",
      reason: collectible.map((job) => `${job.id}:${job.stage}`).join(","),
      jobs: collectible,
      gate: null,
    };
  }
  // 无可收作业：goal 模式 ∧ 显式 opt-in ∧ 有待审门禁 → 在场唤醒（主代理裁决，不是扩展）。
  if (
    opts?.wakeEnabled === true &&
    summary.play === "goal" &&
    summary.gates.length > 0
  ) {
    return {
      kind: "wake",
      reason: `goal 模式待审门禁 ${summary.gates[0].id} ${summary.gates[0].gate} rev${summary.gates[0].rev}`,
      jobs: [],
      gate: summary.gates[0],
    };
  }
  return {
    kind: "none",
    reason:
      summary.gates.length > 0
        ? `停在门禁（${summary.gates.length} 个）等待裁决`
        : "无可收波作业",
    jobs: [],
    gate: null,
  };
}

/** 可收波作业清单：阶段谓词（orchestrator 单源）排除 delivered 与停车键。 */
export function collectJobs(summary: RunSummary, state?: AutoPilotState): SummaryJob[] {
  if (!summary.available) {
    return [];
  }
  return summary.jobs.filter(
    (job) =>
      isCollectibleJobStage(job) &&
      !isDeliveredJob(job) &&

      (state === undefined || (state.parked[collectKey(job)] ?? 0) < WAVE_FAILURES_PARK),
  );
}

// 阶段谓词从 orchestrator 单源导入会引入 node:fs 等运行时依赖面（orchestrator 顶层
// import fs/os/path/child_process）——纯核零依赖原则下改为钉住同一常量集，
// 一致性由 harness.test.ts 的 parity 用例钉住（单侧改动 → 测试红）。
const ACTIVE_STAGES = new Set(["plan", "development"]);

function isCollectibleJobStage(job: SummaryJob): boolean {
  return ACTIVE_STAGES.has(job.stage);
}

/**
 * 波次结算（纯函数，返回新状态）：per-key 成功清停车、失败累加；全败波累加全局
 * 计数并可能自关。键列表由调用方从 `OrchestrationJobResult` 归约（有 error = 失败）。
 */
export function recordWaveOutcome(
  state: AutoPilotState,
  outcomes: readonly { key: string; ok: boolean }[],
): AutoPilotState {
  const parked: Record<string, number> = { ...state.parked };
  let anyOk = false;
  for (const { key, ok } of outcomes) {
    if (ok) {
      delete parked[key];
      anyOk = true;
    } else {
      parked[key] = (parked[key] ?? 0) + 1;
    }
  }
  const allFailed = outcomes.length > 0 && !anyOk;
  const consecutiveWaveFailures = allFailed ? state.consecutiveWaveFailures + 1 : 0;
  return {
    enabled: consecutiveWaveFailures < WAVE_FAILURES_AUTO_OFF,
    waveCount: state.waveCount + 1,
    consecutiveWaveFailures,
    parked,
  };
}

// ───────────────────────────── 纯核：唤醒消息（唯一出口） ─────────────────────────────

/** 自唤醒回合的自定义消息类型（压缩自续跑 `ygg-handoff-resume` 的同型先例）。 */
export const AUTO_WAVE_CUSTOM_TYPE = "ygg-auto-wave";

/**
 * 收波唤醒：让**主代理**（不是扩展）在回合内调 `ygg_run_lanes`。
 * 指令逐字钉死「只调这一个工具」——回执与事件链与手发同构。
 */
export function buildWaveWakeMessage(runId: string, jobs: readonly SummaryJob[]): string {
  const list = jobs.map((job) => `${job.id}(${job.stage} rev${job.rev > 0 ? job.rev : 1})`).join("、");
  return (
    `[ygg 看门狗·自动收波] 检测到可收波作业：${list}（run ${runId}）。` +
    "请立即调用 ygg_run_lanes 完成本波编排，不要调用其他任何工具；" +
    "若编排报告无可编排作业或被拒绝，用一句话确认即可，不要重试。"
  );
}

/**
 * goal 模式门禁唤醒（默认关，`YGG_AUTO_WAKE=on` 才启用）：门禁裁决永远是主代理/
 * 人的决定，看门狗只递「有待审门禁」这件事本身。
 */
export function buildGateWakeMessage(runId: string, gate: SummaryGate): string {
  return (
    `[ygg 看门狗·门禁唤醒] goal 模式下有待审门禁：${gate.id} ${gate.gate} rev${gate.rev} ` +
    `（${gate.title}，run ${runId}）。请按你的裁决处理（ygg_session 的 approve_gate / reject_gate）。`
  );
}

// ───────────────────────────── 波次日志（无人值守场景的全量回执面） ─────────────────────────────

/** 波次日志路径：`artifacts/auto-wave-<NNNN>.md`（四位序号；与交接单同类的宿主侧归档）。 */
export function waveLogPath(wave: number): string {
  return `artifacts/auto-wave-${String(Math.max(1, wave)).padStart(4, "0")}.md`;
}

/**
 * 波次日志 markdown：触发原因 + 逐作业逐回执的 narrative **全文**（永不截断）+ 停点判定。
 * 「三不之永不截断回执」在无人值守场景的落点：主会话不在场时，下个在场回合经交接单
 * 指针完整可达（DESIGN §3.5）。index.ts 的 `ygg_run_lanes` 消费波票后调它落 VFS。
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

// ───────────────────────────── 薄 IO 壳 ─────────────────────────────

/** 默认轮询间隔（`YGG_AUTO_INTERVAL_MS` 可调，钳位 1s–60s）。 */
export const DEFAULT_AUTO_INTERVAL_MS = 5_000;
export const MIN_AUTO_INTERVAL_MS = 1_000;
export const MAX_AUTO_INTERVAL_MS = 60_000;

/** 间隔解析：非法/缺省 → 默认值；越界钳位（1s–60s），fail-safe 不 fail-closed（观测面）。 */
export function clampAutoIntervalMs(value: unknown, fallback = DEFAULT_AUTO_INTERVAL_MS): number {
  const num = typeof value === "string" ? Number(value.trim()) : value;
  if (typeof num !== "number" || !Number.isFinite(num) || num <= 0) {
    return fallback;
  }
  return Math.min(MAX_AUTO_INTERVAL_MS, Math.max(MIN_AUTO_INTERVAL_MS, num));
}

/** `YGG_AUTO_WAKE=on`（大小写不敏感）→ true；其余一律关（默认关）。 */
export function parseAutoWake(value: unknown): boolean {
  return typeof value === "string" && value.trim().toLowerCase() === "on";
}

/** 主代理发送面的最小形状（工厂级 `pi.sendMessage`；测试用记录器顶替）。 */
export interface WakeSender {
  sendMessage(
    message: { customType: string; content: string; display?: boolean },
    options?: { triggerTurn?: boolean },
  ): void;
}

/** 上下文压力探测的最小形状（index.ts 的 `contextPressure(ctx)` 结构性满足）。 */
export interface PressureProbe {
  (): { level: "normal" | "soft_brake" | "hard_brake" } | null;
}

/** 可注入时钟（测试用手动 tick；缺省 `setInterval`）。 */
export interface WatchdogTimer {
  set(fn: () => void, ms: number): WatchdogHandle;
  clear(handle: WatchdogHandle): void;
}

/** 定时器句柄：不透明令牌（只由 `WatchdogTimer.set` 产出、`clear` 消费）。 */
export interface WatchdogHandle {
  /** 宿主原生定时器对象（NodeJS.Timeout / 测试桩句柄；本模块从不触摸它）。 */
  readonly native: unknown;
}

function defaultTimer(): WatchdogTimer {
  return {
    set: (fn, ms) => ({ native: setInterval(fn, ms) }),
    clear: (handle) => clearInterval(handle.native as ReturnType<typeof setInterval>),
  };
}

/** 唤醒发出后的宽限：等自唤醒回合真正起跑（tick 观察到 `isIdle() === false`）前不再发。 */
const WAKE_GRACE_MS = 20_000;

/** 看门狗壳：定时器 + 摘要轮询 + 唤醒发送。纯核之上的全部 IO 都在这里。 */
export interface AutoWatchdogOptions {
  client: YggDaemon;
  sender: WakeSender;
  runId: string;
  /** 会话空闲判定（回合内的一切——手发编排、唤醒回合——都让它为 false）。 */
  isIdle: () => boolean;
  /** 上下文压力探测（null = 没接探针；hard_brake 抑制派波）。 */
  pressure?: PressureProbe;
  wakeEnabled: boolean;
  intervalMs?: number;
  timer?: WatchdogTimer;
  logger?: (line: string) => void;
}

export class AutoWatchdog {
  state: AutoPilotState = initialAutoPilotState();
  private readonly opts: AutoWatchdogOptions;
  private handle: WatchdogHandle | null = null;
  private waitingWake = false;
  private wakeSentAt = 0;
  /**
   * 波票闏：已发出收波唤醒、等 `ygg_run_lanes` 消费。未消费前 tick 不再发新波——
   * 自唤醒回合没调工具（模型不遵指令）也不无限重发（fail-closed：一张票卡住即停发，
   * `/ygg auto status` 可见，由人决断）。
   */
  private waveArmed = false;
  private lastTickReason = "未启动";
  private readonly intervalMs: number;
  private readonly timer: WatchdogTimer;

  constructor(opts: AutoWatchdogOptions) {
    this.opts = opts;
    this.intervalMs = clampAutoIntervalMs(opts.intervalMs);
    this.timer = opts.timer ?? defaultTimer();
  }

  get lastReason(): string {
    return this.lastTickReason;
  }

  /** 生效间隔（钳位后；`/ygg auto on` 确认与 status 用）。 */
  get interval(): number {
    return this.intervalMs;
  }

  start(): void {
    if (this.handle !== null) {
      return;
    }
    this.state = { ...this.state, enabled: true };
    this.lastTickReason = "已启动";
    this.handle = this.timer.set(() => {
      void this.tick();
    }, this.intervalMs);
    this.log(`看门狗启动（间隔 ${this.intervalMs}ms，唤醒=${this.opts.wakeEnabled ? "on" : "off"}）`);
  }

  stop(reason = "手动停止"): void {
    if (this.handle !== null) {
      this.timer.clear(this.handle);
      this.handle = null;
    }
    this.state = { ...this.state, enabled: false };
    this.waitingWake = false;
    this.lastTickReason = reason;
    this.log(`看门狗停止（${reason}）`);
  }

  /** 波次结算（`ygg_run_lanes` 收尾回调；同时消费波票）。 */
  recordWave(outcomes: readonly { key: string; ok: boolean }[]): void {
    this.state = recordWaveOutcome(this.state, outcomes);
    this.waveArmed = false;
    if (!this.state.enabled) {
      this.stop(`连续 ${WAVE_FAILURES_AUTO_OFF} 波全败，看门狗自关（fail-closed）`);
    }
  }

  /** 领取波票（防重）：已有未消费波票 → false（本 tick 不再发）。 */
  armWave(): boolean {
    if (this.waveArmed) {
      return false;
    }
    this.waveArmed = true;
    return true;
  }

  get isWaveArmed(): boolean {
    return this.waveArmed;
  }

  /** 手动测试入口：跑一次判定-发送循环（与定时器回调同体）。 */
  async tick(now = Date.now()): Promise<AutoDecision> {
    if (!this.state.enabled) {
      this.lastTickReason = "未启用";
      return { kind: "none", reason: "未启用", jobs: [], gate: null };
    }
    // 单飞互斥（与手发共享同一把「会话在忙」闸）：唤醒回合/手发工具调用期间 isIdle=false。
    if (this.waitingWake) {
      if (!this.opts.isIdle()) {
        this.waitingWake = false; // 唤醒回合已起跑，宽限解除
      } else if (now - this.wakeSentAt < WAKE_GRACE_MS) {
        this.lastTickReason = "唤醒已发出，等自唤醒回合起跑";
        return { kind: "none", reason: this.lastTickReason, jobs: [], gate: null };
      } else {
        this.waitingWake = false; // 宽限超时（唤醒回合没起跑），恢复判定
      }
    }
    if (!this.opts.isIdle()) {
      this.lastTickReason = "会话在忙（手发/编排进行中），跳过本 tick";
      return { kind: "none", reason: this.lastTickReason, jobs: [], gate: null };
    }
    // 每 tick 一次 summary：既是轮询数据源，也是 daemon 存活心跳（死了由 daemon.ts 重启）。
    const summary = await this.opts.client.summary();
    const decision = nextAutoAction(summary, this.state, {
      wakeEnabled: this.opts.wakeEnabled,
    });
    if (decision.kind === "none") {
      this.lastTickReason = decision.reason;
      return decision;
    }
    // 压缩边界：硬水位不派波（与手发硬刹车同语义）；软水位照跑、落地后重写交接单。
    const pressure = this.opts.pressure?.() ?? null;
    if (decision.kind === "wave" && pressure?.level === "hard_brake") {
      this.lastTickReason = "hard_brake：不派波（看门狗不成为压缩第三入口）";
      return { kind: "none", reason: this.lastTickReason, jobs: decision.jobs, gate: null };
    }
    const wakeMessage =
      decision.kind === "wave"
        ? buildWaveWakeMessage(this.opts.runId, decision.jobs)
        : buildGateWakeMessage(this.opts.runId, decision.gate as SummaryGate);
    if (decision.kind === "wave" && !this.armWave()) {
      this.lastTickReason = "已有未消费波票（等 ygg_run_lanes），不再发新波";
      return { kind: "none", reason: this.lastTickReason, jobs: decision.jobs, gate: null };
    }
    this.opts.sender.sendMessage(
      { customType: AUTO_WAVE_CUSTOM_TYPE, content: wakeMessage, display: true },
      { triggerTurn: true },
    );
    this.waitingWake = true;
    this.wakeSentAt = now;
    this.lastTickReason =
      decision.kind === "wave"
        ? `已发收波唤醒（第 ${this.state.waveCount + 1} 波）：${decision.reason}`
        : `已发门禁唤醒：${decision.reason}`;
    this.log(this.lastTickReason);
    return decision;
  }

  private log(line: string): void {
    this.opts.logger?.(line);
  }
}
