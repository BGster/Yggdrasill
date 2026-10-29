// provider.ts — LLM provider 抽象（FR-11）+ 双协议实现（D-13）。
// 每协议两档：complete（一次性等完整 JSON）与 completeStream（SSE 增量聚合，
//   聚合结果与 complete 等价；onProgress 供界面显示生成进度）。
// openai-chat：POST {base}/chat/completions，response_format json_schema strict
//   携严格兼容子集，网关 400 拒绝时自动降级重发一次（纯 prompt 指令约束）。
// anthropic-messages：POST {base}/v1/messages，tools input_schema + tool_choice 强制。
// 流式两协议共用 sse.ts 的分帧器；超时信号覆盖整个响应生命周期（读流期间同样计时）。
// 客户端 ajv 是唯一权威判据（research Architecture Insight 1）——两个协议的
// wire 约束都是尽力优化，不是正确性来源。
// 凭据红线：apiKey 只进请求头；错误信息只含 HTTP 状态码，不含头/键值。

import { readSseEvents } from "./sse.js";

export type LlmProtocol = "openai-chat" | "anthropic-messages";

export interface ProviderDeps {
  baseUrl: string;
  apiKey: string;
  model: string;
  maxTokens?: number;
  fetchImpl?: typeof fetch;
  /** 单轮超时毫秒（默认 REQUEST_TIMEOUT_MS；测试注入小值验证读流期计时）。 */
  requestTimeoutMs?: number;
}

export interface StructuredRequest {
  system: string;
  user: string;
}

export interface ProviderReply {
  /** 未经校验的 story JSON（权威校验在 schema-gate 的 ajv 面）。 */
  story: unknown;
  usage: { inputTokens: number; outputTokens: number };
}

/** 流式分片回调载荷：text = 本分片新增文本（收尾分片可为空串，只载用量）。 */
export interface StreamDelta {
  text: string;
  inputTokens?: number;
  outputTokens?: number;
}

/** 进度回调（每收到一个流式分片调用一次；累计 text 即最终 story 文本）。 */
export type ProgressHandler = (delta: StreamDelta) => void;

export interface LlmProvider {
  readonly protocol: LlmProtocol;
  complete(request: StructuredRequest): Promise<ProviderReply>;
  /**
   * 流式版本：SSE 增量回调 onProgress，返回的 ProviderReply 与 complete 完全一致
   * （story 由分片文本聚合后走同一条 parseStoryJson / usage 映射）。
   */
  completeStream(request: StructuredRequest, onProgress: ProgressHandler): Promise<ProviderReply>;
}

/** 单轮 max_tokens 软上限（代决 4：成本红线 = 软警告 + 累计计数，M1 无硬闸）。 */
export const DEFAULT_MAX_TOKENS = 16_384;

/** 单轮请求超时（LLM 延迟主导，分钟级预算内的单轮上限）。 */
const REQUEST_TIMEOUT_MS = 120_000;

/** wire 严格兼容子集（D-13）：OpenAI strict 模式不支持 pattern/minLength/自由 map。 */
export const STORY_WIRE_SCHEMA = {
  type: "object",
  additionalProperties: false,
  required: ["slug", "title", "modLua", "scenes"],
  properties: {
    slug: { type: "string" },
    title: { type: "string" },
    modLua: { type: "string" },
    scenes: { type: "object" },
  },
} as const;

/** openai-chat 的 response_format（strict 档；降级重发时整块去掉）。 */
const OPENAI_RESPONSE_FORMAT = {
  type: "json_schema",
  json_schema: { name: "story_mod", strict: true, schema: STORY_WIRE_SCHEMA },
} as const;

export function createProvider(protocol: LlmProtocol, deps: ProviderDeps): LlmProvider {
  const doFetch = deps.fetchImpl ?? fetch;
  const maxTokens = deps.maxTokens ?? DEFAULT_MAX_TOKENS;
  const timeoutMs = deps.requestTimeoutMs ?? REQUEST_TIMEOUT_MS;
  if (protocol === "anthropic-messages") {
    return {
      protocol,
      complete: (request) => completeAnthropic(doFetch, deps, maxTokens, timeoutMs, request),
      completeStream: (request, onProgress) =>
        streamAnthropic(doFetch, deps, maxTokens, timeoutMs, request, onProgress),
    };
  }
  return {
    protocol,
    complete: (request) => completeOpenAiChat(doFetch, deps, maxTokens, timeoutMs, request),
    completeStream: (request, onProgress) =>
      streamOpenAiChat(doFetch, deps, maxTokens, timeoutMs, request, onProgress),
  };
}

/** HTTP 失败（只携状态码，不带响应体——防网关回显键值面）。 */
export class ProviderHttpError extends Error {
  constructor(
    readonly protocol: string,
    readonly status: number,
  ) {
    super(`LLM 请求失败（${protocol}）：HTTP ${status}`);
    this.name = "ProviderHttpError";
  }
}

async function postJson(
  doFetch: typeof fetch,
  url: string,
  headers: Record<string, string>,
  payload: unknown,
  protocol: string,
  timeoutMs: number,
): Promise<unknown> {
  const response = await doFetch(url, {
    method: "POST",
    headers,
    body: JSON.stringify(payload),
    signal: AbortSignal.timeout(timeoutMs),
  });
  if (!response.ok) {
    throw new ProviderHttpError(protocol, response.status);
  }
  return (await response.json()) as unknown;
}

/**
 * 流式入口：POST 后只判状态码，响应体交给调用方读。
 * 超时信号挂在 fetch 上即覆盖整个响应生命周期——读流期间到点同样 abort，
 * reader.read() 随之 reject（不需要另起计时器）。
 */
async function postStream(
  doFetch: typeof fetch,
  url: string,
  headers: Record<string, string>,
  payload: unknown,
  protocol: string,
  timeoutMs: number,
): Promise<ReadableStream<Uint8Array>> {
  const response = await doFetch(url, {
    method: "POST",
    headers,
    body: JSON.stringify(payload),
    signal: AbortSignal.timeout(timeoutMs),
  });
  if (!response.ok) {
    throw new ProviderHttpError(protocol, response.status);
  }
  if (response.body === null) {
    throw new Error(`LLM 流式响应（${protocol}）缺少响应体`);
  }
  return response.body;
}

/** openai-chat 请求体公共段（流式/非流式共用：model/max_tokens/messages 单一来源）。 */
function openAiPayload(
  deps: ProviderDeps,
  maxTokens: number,
  request: StructuredRequest,
  extra: Record<string, unknown>,
): Record<string, unknown> {
  return {
    model: deps.model,
    max_tokens: maxTokens,
    messages: [
      { role: "system", content: request.system },
      { role: "user", content: request.user },
    ],
    ...extra,
  };
}

/** anthropic-messages 请求体（流式/非流式共用：tool 契约单一来源）。 */
function anthropicPayload(
  deps: ProviderDeps,
  maxTokens: number,
  request: StructuredRequest,
  stream: boolean,
): Record<string, unknown> {
  return {
    model: deps.model,
    max_tokens: maxTokens,
    ...(stream ? { stream: true } : {}),
    system: request.system,
    messages: [{ role: "user", content: request.user }],
    tools: [
      {
        name: "story_mod",
        description: "产出一个 Yggdrasill 故事 Mod（slug/title/modLua/scenes 四键结构化输出）",
        input_schema: STORY_WIRE_SCHEMA,
      },
    ],
    tool_choice: { type: "tool", name: "story_mod" },
  };
}

/** openai-chat 流式分片（只声明本管线用到的字段）。 */
interface OpenAiChunk {
  choices?: Array<{ delta?: { content?: unknown } }>;
  usage?: { prompt_tokens?: number; completion_tokens?: number };
}

/** anthropic-messages 流式事件（只声明本管线用到的字段）。 */
interface AnthropicStreamEvent {
  type?: string;
  index?: number;
  message?: { usage?: { input_tokens?: number; output_tokens?: number } };
  content_block?: { type?: string; name?: string };
  delta?: { type?: string; text?: string; partial_json?: string };
  usage?: { input_tokens?: number; output_tokens?: number };
  error?: { type?: string };
}

/** 流式分片 JSON 解析（错误信息不带 data 原文——分片内容是模型生成文本）。 */
function parseChunkJson<T>(data: string, label: string): T {
  try {
    return JSON.parse(data) as T;
  } catch {
    throw new Error(`LLM 流式分片（${label}）不是合法 JSON`);
  }
}

async function completeOpenAiChat(
  doFetch: typeof fetch,
  deps: ProviderDeps,
  maxTokens: number,
  timeoutMs: number,
  request: StructuredRequest,
): Promise<ProviderReply> {
  const url = `${deps.baseUrl.replace(/\/+$/, "")}/chat/completions`;
  const headers = {
    "content-type": "application/json",
    authorization: `Bearer ${deps.apiKey}`,
  };
  const body = (withSchema: boolean) =>
    openAiPayload(
      deps,
      maxTokens,
      request,
      withSchema ? { response_format: OPENAI_RESPONSE_FORMAT } : {},
    );
  let payload: unknown;
  try {
    payload = await postJson(doFetch, url, headers, body(true), "openai-chat", timeoutMs);
  } catch (error) {
    if (error instanceof ProviderHttpError && error.status === 400) {
      // 降级重发一次：网关不支持 response_format（内网面未实证，research Q2）。
      payload = await postJson(doFetch, url, headers, body(false), "openai-chat", timeoutMs);
    } else {
      throw error;
    }
  }
  const typed = payload as {
    choices?: Array<{ message?: { content?: unknown } }>;
    usage?: { prompt_tokens?: number; completion_tokens?: number };
  };
  const content = typed.choices?.[0]?.message?.content;
  if (typeof content !== "string" || content.length === 0) {
    throw new Error("LLM 响应（openai-chat）缺少 choices[0].message.content");
  }
  return {
    story: parseStoryJson(content, "openai-chat"),
    usage: {
      inputTokens: typed.usage?.prompt_tokens ?? 0,
      outputTokens: typed.usage?.completion_tokens ?? 0,
    },
  };
}

/**
 * openai-chat 流式：SSE 分片取 choices[0].delta.content 增量，聚合出与非流式
 * 同一形态的结果（同样的 parseStoryJson 围栏容错）。
 * strict 档 = 流式 + 用量回传 + wire schema；网关 400 时整档降级重发一次
 * （与非流式同策略：一次降级，不叠加多级重试）。
 */
async function streamOpenAiChat(
  doFetch: typeof fetch,
  deps: ProviderDeps,
  maxTokens: number,
  timeoutMs: number,
  request: StructuredRequest,
  onProgress: ProgressHandler,
): Promise<ProviderReply> {
  const url = `${deps.baseUrl.replace(/\/+$/, "")}/chat/completions`;
  const headers = {
    "content-type": "application/json",
    authorization: `Bearer ${deps.apiKey}`,
  };
  const body = (strict: boolean) =>
    openAiPayload(deps, maxTokens, request, {
      stream: true,
      ...(strict
        ? { stream_options: { include_usage: true }, response_format: OPENAI_RESPONSE_FORMAT }
        : {}),
    });
  let stream: ReadableStream<Uint8Array>;
  try {
    stream = await postStream(doFetch, url, headers, body(true), "openai-chat", timeoutMs);
  } catch (error) {
    if (error instanceof ProviderHttpError && error.status === 400) {
      // 与非流式同一降级策略：网关不认识的可选字段（response_format /
      // stream_options）一律去掉，只保留流式必需字段。
      stream = await postStream(doFetch, url, headers, body(false), "openai-chat", timeoutMs);
    } else {
      throw error;
    }
  }

  let content = "";
  let usage: ProviderReply["usage"] = { inputTokens: 0, outputTokens: 0 };
  for await (const event of readSseEvents(stream)) {
    if (event.data === "[DONE]") {
      break; // 协议终止哨兵（其后再无有效分片）
    }
    const chunk = parseChunkJson<OpenAiChunk>(event.data, "openai-chat");
    const text = chunk.choices?.[0]?.delta?.content;
    if (typeof text === "string" && text.length > 0) {
      content += text;
      onProgress({ text });
    }
    const chunkUsage = chunk.usage;
    if (chunkUsage !== undefined && typeof chunkUsage.prompt_tokens === "number") {
      // 仅在网关回传用量时（stream_options.include_usage 生效）取最后一组。
      usage = {
        inputTokens: chunkUsage.prompt_tokens,
        outputTokens: chunkUsage.completion_tokens ?? 0,
      };
      onProgress({ text: "", ...usage }); // 收尾分片：只载用量
    }
  }
  if (content.length === 0) {
    throw new Error("LLM 流式响应（openai-chat）缺少 choices[0].delta.content");
  }
  return { story: parseStoryJson(content, "openai-chat"), usage };
}

async function completeAnthropic(
  doFetch: typeof fetch,
  deps: ProviderDeps,
  maxTokens: number,
  timeoutMs: number,
  request: StructuredRequest,
): Promise<ProviderReply> {
  const url = `${deps.baseUrl.replace(/\/+$/, "")}/v1/messages`;
  const payload = await postJson(
    doFetch,
    url,
    {
      "content-type": "application/json",
      "x-api-key": deps.apiKey,
      "anthropic-version": "2023-06-01",
    },
    anthropicPayload(deps, maxTokens, request, false),
    "anthropic-messages",
    timeoutMs,
  );
  const typed = payload as {
    content?: Array<{ type: string; input?: unknown }>;
    usage?: { input_tokens?: number; output_tokens?: number };
  };
  const toolUse = typed.content?.find((block) => block.type === "tool_use");
  if (toolUse === undefined || typeof toolUse.input !== "object" || toolUse.input === null) {
    throw new Error("LLM 响应（anthropic-messages）缺少 tool_use 块或其 input");
  }
  return {
    story: toolUse.input,
    usage: {
      inputTokens: typed.usage?.input_tokens ?? 0,
      outputTokens: typed.usage?.output_tokens ?? 0,
    },
  };
}

/**
 * anthropic-messages 流式：tool_use 块的 input_json_delta 片段累加成 story JSON，
 * message_delta 的 usage 定格 output_tokens（message_start 的 output 是占位值）。
 * 只取首个 tool_use 块（与非流式 content.find(type==="tool_use") 同序）；
 * text_delta（tool_choice 强制下至多是前言）不进聚合也不上报进度——进度文本的
 * 累计必须严格等于最终 story 文本。
 */
async function streamAnthropic(
  doFetch: typeof fetch,
  deps: ProviderDeps,
  maxTokens: number,
  timeoutMs: number,
  request: StructuredRequest,
  onProgress: ProgressHandler,
): Promise<ProviderReply> {
  const url = `${deps.baseUrl.replace(/\/+$/, "")}/v1/messages`;
  const stream = await postStream(
    doFetch,
    url,
    {
      "content-type": "application/json",
      "x-api-key": deps.apiKey,
      "anthropic-version": "2023-06-01",
    },
    anthropicPayload(deps, maxTokens, request, true),
    "anthropic-messages",
    timeoutMs,
  );

  let toolIndex: number | null = null;
  let fragments = "";
  let sawToolUse = false;
  let usage: ProviderReply["usage"] = { inputTokens: 0, outputTokens: 0 };
  for await (const event of readSseEvents(stream)) {
    const payload = parseChunkJson<AnthropicStreamEvent>(event.data, "anthropic-messages");
    switch (payload.type ?? event.event) {
      case "message_start":
        usage.inputTokens = payload.message?.usage?.input_tokens ?? usage.inputTokens;
        break;
      case "content_block_start":
        if (payload.content_block?.type === "tool_use" && toolIndex === null) {
          toolIndex = payload.index ?? 0;
          sawToolUse = true;
        }
        break;
      case "content_block_delta": {
        const delta = payload.delta;
        const index = payload.index ?? 0;
        if (
          delta?.type === "input_json_delta" &&
          typeof delta.partial_json === "string" &&
          (toolIndex === null || index === toolIndex)
        ) {
          toolIndex ??= index;
          sawToolUse = true;
          fragments += delta.partial_json;
          onProgress({ text: delta.partial_json });
        }
        break;
      }
      case "message_delta":
        usage = {
          inputTokens: payload.usage?.input_tokens ?? usage.inputTokens,
          outputTokens: payload.usage?.output_tokens ?? usage.outputTokens,
        };
        onProgress({ text: "", ...usage }); // 收尾分片：用量在此定格
        break;
      case "error":
        // 只携 error.type：message 可能回显请求上下文，不进错误信息（凭据红线）。
        throw new Error(
          `LLM 流式响应（anthropic-messages）返回 error 事件：${payload.error?.type ?? "unknown"}`,
        );
      default:
        break; // ping / content_block_stop / message_stop：无聚合语义
    }
  }
  if (!sawToolUse) {
    throw new Error("LLM 响应（anthropic-messages）缺少 tool_use 块或其 input");
  }
  return { story: parseStoryJson(fragments, "anthropic-messages"), usage };
}

/** content → story JSON（容忍降级路径下的 markdown 围栏残留）。 */
export function parseStoryJson(content: string, label: string): unknown {
  const stripped = content
    .trim()
    .replace(/^```(?:json)?\s*\n?/, "")
    .replace(/\n?```\s*$/, "");
  try {
    return JSON.parse(stripped) as unknown;
  } catch (error) {
    throw new Error(
      `LLM 响应（${label}）不是合法 JSON：${error instanceof Error ? error.message : String(error)}`,
    );
  }
}
