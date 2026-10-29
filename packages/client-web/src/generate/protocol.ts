// protocol.ts — POST /__generate 的 wire 契约（A2/A3）：SSE 事件形状 + 客户端侧
// 形状校验解析。
//
// 单一事实源：vite.plugin.generate.ts（服务端）type-only 引用本文件的
// GenerateEvent 来构造事件；src/generate/* 客户端运行时消费同一联合类型，并对
// 每条 data 载荷做形状校验（服务端/代理/中间层任何环节漂移都只让该事件被忽略，
// 不会把面板打崩）。on-wire 字段名改动必须双侧同步。
//
// 事件序列（一次生成）：progress* preview* → done | error。

/** 生成失败时的门错误投影（只保留可展示字段：不带 ajv params 等原始数据面）。 */
export interface GenerateIssueView {
  gate: string;
  code: string;
  file: string;
  message: string;
}

/** 进度事件：累计粗估 token + 已解析出的文案条目数。 */
export interface GenerateProgressEvent {
  type: "progress";
  /** 当前轮次（1 起；重试轮次重新计数）。 */
  round: number;
  tokens: number;
  scenes: number;
  endings: number;
}

/** 预览事件：流式前缀能确定的标题与已闭合的场景/结局文案。 */
export interface GeneratePreviewEvent {
  type: "preview";
  title: string | null;
  scenes: Array<{ id: string; text: string }>;
  endings: Array<{ id: string; text: string }>;
}

/** 完成事件：产物已落盘 packages/mods/generated/<slug>/。 */
export interface GenerateDoneEvent {
  type: "done";
  slug: string;
}

/** 失败事件（重试额度耗尽或环境/工具面错误）。 */
export interface GenerateErrorEvent {
  type: "error";
  message: string;
  issues: GenerateIssueView[];
}

export type GenerateEvent =
  | GenerateProgressEvent
  | GeneratePreviewEvent
  | GenerateDoneEvent
  | GenerateErrorEvent;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function isText(value: unknown): value is string {
  return typeof value === "string";
}

function isEntryList(value: unknown): value is Array<{ id: string; text: string }> {
  return (
    Array.isArray(value) &&
    value.every((entry) => isRecord(entry) && isText(entry.id) && isText(entry.text))
  );
}

function isIssueList(value: unknown): value is GenerateIssueView[] {
  return (
    Array.isArray(value) &&
    value.every(
      (issue) =>
        isRecord(issue) &&
        isText(issue.gate) &&
        isText(issue.code) &&
        isText(issue.file) &&
        isText(issue.message),
    )
  );
}

/**
 * 解析一条 SSE data 载荷：形状不符 / 未知类型 / 非法 JSON → null。
 * 客户端拿到 null 直接忽略（不中断整条流，不抛错）。
 */
export function parseGenerateEvent(data: string): GenerateEvent | null {
  let payload: unknown;
  try {
    payload = JSON.parse(data) as unknown;
  } catch {
    return null;
  }
  if (!isRecord(payload)) {
    return null;
  }
  switch (payload.type) {
    case "progress":
      if (!isNumber(payload.round) || !isNumber(payload.tokens)) return null;
      if (!isNumber(payload.scenes) || !isNumber(payload.endings)) return null;
      return {
        type: "progress",
        round: payload.round,
        tokens: payload.tokens,
        scenes: payload.scenes,
        endings: payload.endings,
      };
    case "preview":
      if (!(payload.title === null || isText(payload.title))) return null;
      if (!isEntryList(payload.scenes) || !isEntryList(payload.endings)) return null;
      return {
        type: "preview",
        title: payload.title,
        scenes: payload.scenes.map((entry) => ({ id: entry.id, text: entry.text })),
        endings: payload.endings.map((entry) => ({ id: entry.id, text: entry.text })),
      };
    case "done":
      if (!isText(payload.slug) || payload.slug.length === 0) return null;
      return { type: "done", slug: payload.slug };
    case "error":
      if (!isText(payload.message) || !isIssueList(payload.issues)) return null;
      return {
        type: "error",
        message: payload.message,
        issues: payload.issues.map((issue) => ({
          gate: issue.gate,
          code: issue.code,
          file: issue.file,
          message: issue.message,
        })),
      };
    default:
      return null;
  }
}
