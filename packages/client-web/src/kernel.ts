// ============================================================================
// kernel.ts — 内核 JSON 信封契约（A1 线格式）+ KernelBridge 接口 + ReplayKernel
//
// B2 阶段的唯一内核依赖面：真实 wasm 绑定（B1/B3）落地前，UI 与测试全部
// 跑在 ReplayKernel 上——它逐字回放 fixtures/adventure-transcript.json
// （真实内核 dispatch 回包采集，见 meta.generator）。信封键序 = serde_json
// 字典序；域错误永不 throw，一律 `{ok:false, error:{code,message}}`。
// ============================================================================
// **绑定型文件**：本文件绑定 `packages/mods/test-adventure` 的 agent 契约（事件名 / 结局前缀硬编码在字面量里）；换 Mod 需同步修改。

import transcriptJson from "./fixtures/adventure-transcript.json";

/** 待决选择（choice-as-state：prompt + 选项文案序）。 */
export interface PendingChoice {
  prompt: string;
  options: string[];
}

/** 信封错误（稳定字符串码，下游按码分支不解析 message）。 */
interface EnvelopeError {
  code: string;
  message: string;
}

/**
 * 内核存档本体（`save` 信封的 `save` 字段 = `load` 的 payload 原文）。
 * 结构由内核 SaveFile 定型：引擎版本门 + 实体表 + 持久 KV + 待决选择。
 */
interface SaveData {
  format_version: number;
  engine_version: string;
  entities: unknown;
  kv: { entries: Record<string, unknown>; revision: number };
  pending_choice: PendingChoice | null;
}

/**
 * JSON 信封（A1）。事件路径：ok/event/narrative/pendingChoice/
 * eventsProcessed/eventsDerived/eventsDropped/revision + 加性可选 warnings；
 * 保留命令各自附加字段（save/restored/state 诊断面/log/drained）。
 */
export interface Envelope {
  ok: boolean;
  event?: string;
  narrative?: string[];
  pendingChoice?: PendingChoice | null;
  eventsProcessed?: number;
  eventsDerived?: number;
  eventsDropped?: number;
  revision?: number;
  warnings?: string[];
  error?: EnvelopeError;
  // ── 保留命令附加面（save/load/state/log/drain）─────────────────────
  save?: SaveData;
  restored?: boolean;
  drained?: unknown[];
  log?: unknown[];
  engineVersion?: string;
  kvRevision?: number;
  snapshotBytes?: number;
}

/**
 * 内核桥——UI 唯一可见的内核面。B2 实现 = ReplayKernel（fixture 回放）；
 * B3 落地 wasm 绑定后同接口换实现，UI 零改动。
 *
 * 约定：除 `start()` 外全部方法要求游戏已开始（UI 由开始态门控按钮）。
 */
export interface KernelBridge {
  /** 派发序章（event.start）。返回首屏信封（叙事 + 待决选择）。 */
  start(): Envelope;
  /** 通用派发入口（ygg.choice_selected 等；payload = JSON 标量字符串）。 */
  dispatch(cmd: string, payload: string): Envelope;
  /** 存档：返回 `{ok, save}` 信封——`save` 字段即存档本体（JSON.stringify 后作 load payload）。 */
  save(): Envelope;
  /** 读档：payload = save 信封 `save` 字段的 JSON 原文；失败回 VALIDATION 错误信封。 */
  load(saveJson: string): Envelope;
}

// ── 信封解析 ─────────────────────────────────────────────────────────────

/**
 * 信封字符串 → 结构化信封。驱动信封永不非法（契约面）；非法输入（编程
 * 错误/传输损坏）在此抛出人读中文错误，由调用方落错误呈现面。
 */
export function parseEnvelope(raw: string): Envelope {
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch (err) {
    throw new Error(`信封不是合法 JSON：${err instanceof Error ? err.message : String(err)}`);
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("信封必须是 JSON 对象");
  }
  const obj = value as Record<string, unknown>;
  if (typeof obj.ok !== "boolean") {
    throw new Error("信封缺少布尔字段 ok");
  }
  if (!obj.ok) {
    const error = obj.error;
    if (
      typeof error !== "object" ||
      error === null ||
      !("code" in error) ||
      !("message" in error) ||
      typeof error.code !== "string" ||
      typeof error.message !== "string"
    ) {
      throw new Error("错误信封缺少 error.code / error.message");
    }
  }
  // SAFETY: 上面已门禁 ok:boolean 与错误信封 error.code/message:string；
  // 其余可选字段是否齐全交由消费方自行判空（信封字段全部可选）。
  return obj as unknown as Envelope;
}

/** 稳定错误码速查（driver.rs `code` 模块面；判断一律比对字符串）：
 *  `OK` / `VALIDATION` / `FUEL_EXHAUSTED` / `SANDBOX_DENIED` / `KV_LIMIT` /
 *  `ENGINE_MISMATCH` / `INTERNAL`。本包不导出常量表——消费方按码字符串比对即可。
 */

// ── 回放 fixture 结构 ────────────────────────────────────────────────────

/** fixture 单步：一次玩家可见 dispatch + 同刻 state/save/load 三信封。 */
interface FixtureStep {
  cmd: string;
  payload: string;
  envelope: Envelope;
  state: Envelope;
  save: Envelope;
  load: Envelope;
}

interface FixtureBranch {
  path: number[];
  steps: FixtureStep[];
}

interface TranscriptFixture {
  meta: Record<string, string>;
  branches: Record<string, FixtureBranch>;
  samples: {
    warnings: Envelope;
    choiceOutOfRange: Envelope;
    loadRejected: Envelope;
  };
}

// SAFETY: fixture 由真实内核采集脚本生成（结构见上方接口注释），键集
// 超出/缺额接口声明时 TS 无法验证——回放路径只消费声明的键，运行期以
// fixture 完整性测试兜底（两分支各步信封结构断言）。
const FIXTURE = transcriptJson as unknown as TranscriptFixture;

/** 结局叙事行前缀（与 CLI adventure.rs 的 ENDING_PREFIX 同源）。 */
export const ENDING_PREFIX = "【结局";

/** 深拷贝信封（回放返回物与 fixture 隔离，防调用方就地改写）。信封均为 JSON 值。 */
function cloneEnvelope(envelope: Envelope): Envelope {
  return structuredClone(envelope);
}

// ── ReplayKernel ─────────────────────────────────────────────────────────

/**
 * 回放内核：对真实内核采集 fixture 的确定性回放（离线 UI 开发/测试用）。
 *
 * 语义（对齐 CLI adventure.rs 主循环）：
 * - `start()` → 回放 `event.start` 首屏信封（两分支共享步 0）；重复 start
 *   幂等重放首屏（不推进——mock 无法重跑序章，重新开始 = 新建实例）；
 * - `dispatch("ygg.choice_selected", n)` → 以 payload 索引在分支步序中
 *   路由：分支 a（1→1→结局A）与分支 b（2→2→结局B）在前两步分岔；
 * - 越界/未收录选择 → 真实内核越界回包（VALIDATION「选项不存在」）；
 * - `save()` → 当前位的真实 save 信封；`load(payload)` → 匹配已采集的
 *   存档则跳转该位并回放真实 load 回包，否则 VALIDATION；
 * - 结局后选择 → VALIDATION「冒险已结束」（mock 收口语义；CLI 等价物
 *   是结局即退出主循环）。
 */
export class ReplayKernel implements KernelBridge {
  private branch: string | null = null;
  /** 已消费步数（0 = 未开始；d = steps[0..d-1] 已回放，当前位于步 d-1）。 */
  private depth = 0;

  start(): Envelope {
    if (this.depth > 0) {
      // 幂等重放首屏（不推进、不改分支——真实重开语义 = 新建实例）。
      return this.stepAt(0).envelope;
    }
    return this.advanceTo(0);
  }

  dispatch(cmd: string, payload: string): Envelope {
    if (this.depth === 0) {
      return mockEnvelope(
        "VALIDATION",
        "回放 mock：游戏尚未开始（先 start / 重新开始按钮）",
      );
    }
    if (cmd === "ygg.choice_selected") {
      return this.dispatchChoice(payload);
    }
    if (cmd === "state") {
      return cloneEnvelope(this.currentStep().state);
    }
    if (cmd === "save") {
      return cloneEnvelope(this.currentStep().save);
    }
    if (cmd === "log") {
      return { ok: true, log: [] };
    }
    if (cmd === "drain") {
      return { ok: true, drained: [] };
    }
    if (cmd === "event.start") {
      return mockEnvelope("VALIDATION", "回放 mock：event.start 已派发；重新开始请新建回放实例");
    }
    return mockEnvelope(
      "VALIDATION",
      `回放 mock：未收录的事件 \`${cmd}\`（离线回放仅覆盖 test-adventure 两分支）`,
    );
  }

  save(): Envelope {
    if (this.depth === 0) {
      return mockEnvelope("VALIDATION", "回放 mock：游戏尚未开始，无可存档");
    }
    return cloneEnvelope(this.currentStep().save);
  }

  load(saveJson: string): Envelope {
    if (this.depth === 0) {
      return mockEnvelope("VALIDATION", "回放 mock：游戏尚未开始，无可读档");
    }
    // 匹配已采集的存档：任一分支任一步的 save 本体与 payload 深等即跳转。
    for (const [key, branch] of Object.entries(FIXTURE.branches)) {
      for (let i = 0; i < branch.steps.length; i++) {
        const step = branch.steps[i];
        if (JSON.stringify(step.save.save) === normalizeJson(saveJson)) {
          this.branch = key;
          this.depth = i + 1;
          // 真实 load 回包（内核读回同一存档的 reply），narrative 由
          // 调用方按 CLI 语义补「读档完成」行——此处保真原样返回。
          return cloneEnvelope(step.load);
        }
      }
    }
    // 未匹配：payload 非 JSON → 真实内核原文回包；否则 mock 侧匹配失败文案。
    try {
      JSON.parse(saveJson);
    } catch {
      return cloneEnvelope(FIXTURE.samples.loadRejected);
    }
    return mockEnvelope("VALIDATION", "load rejected: 回放 mock：存档与已采集的回放进度不匹配");
  }

  // ── 内部 ─────────────────────────────────────────────────────────────

  /** 选择路由：当前深度各分支的步 (cmd, payload) 匹配 payload 索引。 */
  private dispatchChoice(payload: string): Envelope {
    const candidates = Object.values(FIXTURE.branches)
      .flatMap((branch) => (this.depth < branch.steps.length ? [branch.steps[this.depth]] : []))
      .filter((step) => step.cmd === "ygg.choice_selected");
    if (candidates.length === 0) {
      // 已到结局（无更多步）——mock 收口语义。
      return mockEnvelope("VALIDATION", "选项不存在：冒险已结束（重新开始开始新的冒险）");
    }
    const matched = candidates.find((step) => step.payload === payload);
    if (matched) {
      const index = Object.values(FIXTURE.branches).findIndex(
        (branch) => branch.steps[this.depth] === matched,
      );
      this.branch = Object.keys(FIXTURE.branches)[index];
      return this.advanceTo(this.depth);
    }
    // 索引越界 → 真实内核越界回包（文案含有效范围，两选择点均为 1..2）。
    return cloneEnvelope(FIXTURE.samples.choiceOutOfRange);
  }

  private advanceTo(stepIndex: number): Envelope {
    const step = this.stepAt(stepIndex);
    this.depth = stepIndex + 1;
    return cloneEnvelope(step.envelope);
  }

  /** 取指定深度的步：未定分支用分支 a（首屏两分支逐字节一致）。 */
  private stepAt(stepIndex: number): FixtureStep {
    const key = this.branch ?? Object.keys(FIXTURE.branches)[0];
    const branch = FIXTURE.branches[key];
    if (stepIndex >= branch.steps.length) {
      throw new Error(`回放 mock：步序越界（${key}#${stepIndex}）`);
    }
    return branch.steps[stepIndex];
  }

  private currentStep(): FixtureStep {
    return this.stepAt(this.depth - 1);
  }
}

/** 归一化 JSON 文本（键序不敏感的字符串比较基线：parse → 再 stringify）。 */
function normalizeJson(text: string): string {
  try {
    return JSON.stringify(JSON.parse(text));
  } catch {
    return text;
  }
}

/** mock 侧错误信封（形态与 driver.rs error_envelope 一致）。 */
function mockEnvelope(code: string, message: string): Envelope {
  return { ok: false, error: { code, message } };
}
