// state.ts — 生成面板的纯状态机（A2/A3）：把 SSE 事件序列折叠成界面状态。
// 抽成纯函数是为了让「重试轮次清零 / done 带 slug / error 落文案」这些容易被
// 事件顺序绕晕的规则可单测（React 只负责把状态画出来）。
import type { GenerateEvent, GenerateIssueView } from "./protocol";

export type GeneratePhase = "idle" | "running" | "done" | "error";

export interface GenerateEntryView {
  id: string;
  text: string;
}

export interface GenerateState {
  phase: GeneratePhase;
  /** 当前轮次（progress 事件带来；重试轮次出现时 > 1）。 */
  round: number;
  /** 累计粗估 token（服务端口径：本次生成全程累加）。 */
  tokens: number;
  scenes: number;
  endings: number;
  title: string | null;
  previewScenes: GenerateEntryView[];
  previewEndings: GenerateEntryView[];
  /** 完成后的产物 slug（done 事件）。 */
  slug: string | null;
  /** 失败文案（error 事件或客户端侧异常）。 */
  message: string | null;
  issues: GenerateIssueView[];
}

export function initialGenerateState(): GenerateState {
  return {
    phase: "idle",
    round: 1,
    tokens: 0,
    scenes: 0,
    endings: 0,
    title: null,
    previewScenes: [],
    previewEndings: [],
    slug: null,
    message: null,
    issues: [],
  };
}

/** 起跑：保留输入面状态，清掉上一轮的结果面（重跑不残留旧预览/旧错误）。 */
export function startGenerate(state: GenerateState): GenerateState {
  return { ...initialGenerateState(), phase: "running", round: state.round };
}

/** 客户端侧异常（fetch 失败 / 非 2xx / 流未收尾）：落失败态。 */
export function failGenerate(state: GenerateState, message: string): GenerateState {
  return { ...state, phase: "error", message, issues: [] };
}

/**
 * 折叠一条事件。终态（done / error）之后到达的事件一律忽略——SSE 尾部可能因
 * 代理缓冲重排，不能让迟到的 progress 把 done 态翻回 running。
 */
export function reduceGenerateEvent(state: GenerateState, event: GenerateEvent): GenerateState {
  if (state.phase === "done" || state.phase === "error") {
    return state;
  }
  switch (event.type) {
    case "progress":
      return {
        ...state,
        phase: "running",
        round: event.round,
        tokens: event.tokens,
        scenes: event.scenes,
        endings: event.endings,
      };
    case "preview":
      return {
        ...state,
        phase: "running",
        title: event.title,
        previewScenes: event.scenes,
        previewEndings: event.endings,
      };
    case "done":
      return { ...state, phase: "done", slug: event.slug, message: null, issues: [] };
    case "error":
      return { ...state, phase: "error", message: event.message, issues: event.issues };
  }
}
