// provider.ts — LLM provider 抽象（FR-11）+ 双协议实现（D-13）。
// openai-chat：POST {base}/chat/completions，response_format json_schema strict
//   携严格兼容子集，网关 400 拒绝时自动降级重发一次（纯 prompt 指令约束）。
// anthropic-messages：POST {base}/v1/messages，tools input_schema + tool_choice 强制。
// 客户端 ajv 是唯一权威判据（research Architecture Insight 1）——两个协议的
// wire 约束都是尽力优化，不是正确性来源。
// 凭据红线：apiKey 只进请求头；错误信息只含 HTTP 状态码，不含头/键值。

export type LlmProtocol = "openai-chat" | "anthropic-messages";

export interface ProviderDeps {
  baseUrl: string;
  apiKey: string;
  model: string;
  maxTokens?: number;
  fetchImpl?: typeof fetch;
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

export interface LlmProvider {
  readonly protocol: LlmProtocol;
  complete(request: StructuredRequest): Promise<ProviderReply>;
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

export function createProvider(protocol: LlmProtocol, deps: ProviderDeps): LlmProvider {
  const doFetch = deps.fetchImpl ?? fetch;
  const maxTokens = deps.maxTokens ?? DEFAULT_MAX_TOKENS;
  return protocol === "anthropic-messages"
    ? { protocol, complete: (request) => completeAnthropic(doFetch, deps, maxTokens, request) }
    : { protocol, complete: (request) => completeOpenAiChat(doFetch, deps, maxTokens, request) };
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
): Promise<unknown> {
  const response = await doFetch(url, {
    method: "POST",
    headers,
    body: JSON.stringify(payload),
    signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
  });
  if (!response.ok) {
    throw new ProviderHttpError(protocol, response.status);
  }
  return (await response.json()) as unknown;
}

async function completeOpenAiChat(
  doFetch: typeof fetch,
  deps: ProviderDeps,
  maxTokens: number,
  request: StructuredRequest,
): Promise<ProviderReply> {
  const url = `${deps.baseUrl.replace(/\/+$/, "")}/chat/completions`;
  const headers = {
    "content-type": "application/json",
    authorization: `Bearer ${deps.apiKey}`,
  };
  const body = (withSchema: boolean) => ({
    model: deps.model,
    max_tokens: maxTokens,
    messages: [
      { role: "system", content: request.system },
      { role: "user", content: request.user },
    ],
    ...(withSchema
      ? {
          response_format: {
            type: "json_schema",
            json_schema: { name: "story_mod", strict: true, schema: STORY_WIRE_SCHEMA },
          },
        }
      : {}),
  });
  let payload: unknown;
  try {
    payload = await postJson(doFetch, url, headers, body(true), "openai-chat");
  } catch (error) {
    if (error instanceof ProviderHttpError && error.status === 400) {
      // 降级重发一次：网关不支持 response_format（内网面未实证，research Q2）。
      payload = await postJson(doFetch, url, headers, body(false), "openai-chat");
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

async function completeAnthropic(
  doFetch: typeof fetch,
  deps: ProviderDeps,
  maxTokens: number,
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
    {
      model: deps.model,
      max_tokens: maxTokens,
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
    },
    "anthropic-messages",
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
