// protocol.ts — POST /__revise 的 wire 契约（A5）：请求校验 + 响应形状 + 服务端
// 结果投影 + 客户端形状解析。
//
// 与 /__generate 的差别：revise 是**单轮**、无需流式进度（改的是几十 KB 既有文本），
// 所以走一条普通 JSON 请求/响应，而不是 SSE。
//
// 单一事实源：vite.plugin.revise.ts 运行时引用本文件的 parseReviseRequest /
// toReviseResponse 构造两端；src/revise/* 客户端消费同一份类型。所以在浏览器
// 侧与 vite 侧，请求/响应的字段名只有这一处定义。
//
// 边界：请求体只承载「哪个 slug + 一条意见 + 是否授权动 Lua」——不含任何凭据；
// 响应只承载 slug、变更路径清单、门诊断（不带 ajv params / hint / 上游原文）。

/** 修改意见长度上限（prompt 预算 + 挡住误粘长文）。 */
export const MAX_FEEDBACK_CHARS = 2000;

/** 端点路径（vite.plugin.revise.ts 与本文件共用；客户端从 client.ts 再导出）。 */
export const REVISE_ENDPOINT = "/__revise";

/**
 * slug 白名单：与 STORY_SCHEMA.slug 同一形态（小写字母/数字开头，仅
 * [a-z0-9-]，≤48 字符）。这既是「作品 id 契约」也是**路径安全边界**——
 * slug 直接拼进 packages/mods/generated/<slug>，必须杜绝 / 与 ..。
 */
export const SLUG_PATTERN = /^[a-z0-9][a-z0-9-]{0,47}$/;

export function isValidSlug(value: unknown): value is string {
  return typeof value === "string" && SLUG_PATTERN.test(value);
}

/** 请求体（客户端 → vit e dev 端点）。 */
export interface ReviseRequest {
  slug: string;
  feedback: string;
  /** 显式授权大改（放行 mod.lua 变更与 scenes 六节键集增删）。 */
  allowLua: boolean;
}

/** 失败面的一条诊断（行号已折进 file，与生成面板的 issues 展示同构）。 */
export interface ReviseIssueView {
  gate: string;
  code: string;
  file: string;
  message: string;
}

export interface ReviseSuccess {
  ok: true;
  slug: string;
  /** 变更路径清单（点分：modLua / scenes.<节>.<键>），确定性序。 */
  changes: string[];
  /** 空 diff（模型认为无需修改）：管线按警告放行，这里如实告知。 */
  noop: boolean;
}

export interface ReviseFailure {
  ok: false;
  issues: ReviseIssueView[];
}

export type ReviseResponse = ReviseSuccess | ReviseFailure;

/** 请求校验结果：失败时带 HTTP 状态码，端点直接照搬（不在插件里二次判断）。 */
export type ReviseRequestParse =
  | { ok: true; request: ReviseRequest }
  | { ok: false; status: number; error: string };

/**
 * 校验请求体（服务端权威面）：字段类型 / slug 白名单 / 意见非空与长度。
 * allowLua 只认字面 true——字符串 "true" 之类一律视为未授权（冻结是安全默认）。
 */
export function parseReviseRequest(body: unknown): ReviseRequestParse {
  if (typeof body !== "object" || body === null || Array.isArray(body)) {
    return { ok: false, status: 400, error: "请求体必须是 JSON 对象" };
  }
  const record = body as Record<string, unknown>;
  if (!isValidSlug(record.slug)) {
    return {
      ok: false,
      status: 400,
      error: "缺少或非法 slug（只允许小写字母、数字与连字符，≤48 字符）",
    };
  }
  const rawFeedback = record.feedback;
  if (typeof rawFeedback !== "string" || rawFeedback.trim().length === 0) {
    return { ok: false, status: 400, error: "缺少 feedback（修改意见）" };
  }
  const feedback = rawFeedback.trim();
  if (feedback.length > MAX_FEEDBACK_CHARS) {
    return {
      ok: false,
      status: 413,
      error: `修改意见过长（上限 ${MAX_FEEDBACK_CHARS} 字）`,
    };
  }
  return { ok: true, request: { slug: record.slug, feedback, allowLua: record.allowLua === true } };
}

/**
 * revise 结果的可投影子集：结构对齐 pipeline 的 ReviseResult（不写跨包类型依赖，
 * 由 TS 的结构化子类型保证「传真实 ReviseResult 也能编译」）。
 */
export interface ReviseOutcomeLike {
  ok: boolean;
  slug: string;
  changed: string[];
  issues: Array<{
    gate: string;
    code: string;
    file: string;
    message: string;
    line?: number;
    severity?: string;
  }>;
}

/** 门诊断 → wire 投影（行号折进 file；warning 不进失败面——它不解释失败）。 */
function toIssueView(issue: ReviseOutcomeLike["issues"][number]): ReviseIssueView {
  const location = issue.line !== undefined ? `${issue.file}:${issue.line}` : issue.file;
  return { gate: issue.gate, code: issue.code, file: location, message: issue.message };
}

/**
 * 服务端结果 → 响应体。失败面只留 error 级诊断；若过滤后为空（诊断面异常），
 * 退回全量——宁可展示一条 warning，也不给用户一个空白的错误框。
 */
export function toReviseResponse(outcome: ReviseOutcomeLike): ReviseResponse {
  if (outcome.ok) {
    return {
      ok: true,
      slug: outcome.slug,
      changes: [...outcome.changed],
      noop: outcome.changed.length === 0,
    };
  }
  const errors = outcome.issues.filter((issue) => issue.severity !== "warning");
  const shown = errors.length > 0 ? errors : outcome.issues;
  return { ok: false, issues: shown.map(toIssueView) };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isIssueView(value: unknown): value is ReviseIssueView {
  return (
    isRecord(value) &&
    typeof value.gate === "string" &&
    typeof value.code === "string" &&
    typeof value.file === "string" &&
    typeof value.message === "string"
  );
}

/**
 * 客户端侧响应形状解析：不符 → null（调用方转成一条可展示错误，不猜字段）。
 * 端点/中间层漂移时宁可报「形状不符」，也不把半个对象画进面板。
 */
export function parseReviseResponse(payload: unknown): ReviseResponse | null {
  if (!isRecord(payload) || typeof payload.ok !== "boolean") {
    return null;
  }
  if (payload.ok) {
    if (typeof payload.slug !== "string" || payload.slug.length === 0) {
      return null;
    }
    if (!Array.isArray(payload.changes) || !payload.changes.every((c) => typeof c === "string")) {
      return null;
    }
    if (typeof payload.noop !== "boolean") {
      return null;
    }
    return {
      ok: true,
      slug: payload.slug,
      changes: [...payload.changes],
      noop: payload.noop,
    };
  }
  if (!Array.isArray(payload.issues) || !payload.issues.every(isIssueView)) {
    return null;
  }
  return {
    ok: false,
    issues: payload.issues.map((issue) => ({
      gate: issue.gate,
      code: issue.code,
      file: issue.file,
      message: issue.message,
    })),
  };
}
