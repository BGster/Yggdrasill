// provider-stream.test.ts — provider 流式档（completeStream）契约测：
// ①openai-chat SSE 分片解析（多行 data / [DONE] / 任意位置切断的残缺分片）
// ②anthropic-messages 事件流解析（input_json_delta / message_delta usage）
// ③聚合等价性（同一 mock 序列下流式 == 非流式）④进度文本累计 == 最终 story 文本
// ⑤HTTP 错误只携状态码（凭据红线）⑥读流期间同样受超时约束。
// 全程 mock fetchImpl 注入 ReadableStream，不触网。
import { describe, expect, it } from "vitest";
import type { ProgressHandler, StreamDelta } from "../src/provider.js";
import { createProvider, ProviderHttpError, STORY_WIRE_SCHEMA } from "../src/provider.js";

const STORY = {
  slug: "demo",
  title: "演示",
  modLua: "-- demo",
  scenes: { scenes: {}, endings: {}, prompts: {}, options: {}, beats: {}, tuning: {} },
};
const STORY_JSON = JSON.stringify(STORY);

const encoder = new TextEncoder();

interface Call {
  url: string;
  init: RequestInit;
}

/** 把 SSE 全文等距切片：块边界可落在任意位置（JSON 中间 / data 行中间）。 */
function sliceText(text: string, size: number): string[] {
  const chunks: string[] = [];
  for (let i = 0; i < text.length; i += size) {
    chunks.push(text.slice(i, i + size));
  }
  return chunks;
}

/** 注入式 fetch：把 chunks 依次灌进响应体（text/event-stream）。 */
function sseFetch(chunks: string[], calls: Call[], status = 200): typeof fetch {
  return (async (url: unknown, init?: RequestInit) => {
    calls.push({ url: String(url), init: init ?? {} });
    const stream = new ReadableStream<Uint8Array>({
      start(controller) {
        for (const chunk of chunks) {
          controller.enqueue(encoder.encode(chunk));
        }
        controller.close();
      },
    });
    return new Response(stream, { status, headers: { "content-type": "text/event-stream" } });
  }) as unknown as typeof fetch;
}

/** openai-chat 的 SSE 文本：content 增量分片 + 可选收尾 usage + [DONE]。 */
function openAiSse(
  pieces: string[],
  usage?: { prompt_tokens: number; completion_tokens: number },
): string {
  const lines = pieces.map(
    (piece) => `data: ${JSON.stringify({ choices: [{ delta: { content: piece } }] })}\n\n`,
  );
  if (usage !== undefined) {
    lines.push(`data: ${JSON.stringify({ choices: [], usage })}\n\n`);
  }
  lines.push("data: [DONE]\n\n");
  return lines.join("");
}

/** anthropic-messages 的事件流文本：tool_use JSON 片段 + usage。 */
function anthropicSse(
  fragments: string[],
  usage: { input_tokens: number; output_tokens: number },
  options: { preamble?: string } = {},
): string {
  const event = (name: string, data: unknown) => `event: ${name}\ndata: ${JSON.stringify(data)}\n\n`;
  const parts = [
    event("message_start", {
      type: "message_start",
      message: { usage: { input_tokens: usage.input_tokens, output_tokens: 1 } },
    }),
    event("ping", { type: "ping" }),
  ];
  if (options.preamble !== undefined) {
    // 前言文本块：tool_choice 强制下仍可能出现，退不进 story 聚合。
    parts.push(
      event("content_block_start", {
        type: "content_block_start",
        index: 0,
        content_block: { type: "text", text: "" },
      }),
      event("content_block_delta", {
        type: "content_block_delta",
        index: 0,
        delta: { type: "text_delta", text: options.preamble },
      }),
      event("content_block_stop", { type: "content_block_stop", index: 0 }),
    );
  }
  const toolIndex = options.preamble === undefined ? 0 : 1;
  parts.push(
    event("content_block_start", {
      type: "content_block_start",
      index: toolIndex,
      content_block: { type: "tool_use", id: "tu_1", name: "story_mod", input: {} },
    }),
    ...fragments.map((fragment) =>
      event("content_block_delta", {
        type: "content_block_delta",
        index: toolIndex,
        delta: { type: "input_json_delta", partial_json: fragment },
      }),
    ),
    event("content_block_stop", { type: "content_block_stop", index: toolIndex }),
    event("message_delta", {
      type: "message_delta",
      delta: { stop_reason: "tool_use" },
      usage: { output_tokens: usage.output_tokens },
    }),
    event("message_stop", { type: "message_stop" }),
  );
  return parts.join("");
}

function collectProgress(): {
  onProgress: ProgressHandler;
  deltas: StreamDelta[];
  text: () => string;
} {
  const deltas: StreamDelta[] = [];
  return {
    onProgress: (delta) => deltas.push(delta),
    deltas,
    text: () => deltas.map((delta) => delta.text).join(""),
  };
}

/** 取 promise 的错误（成功则 null）：错误面断言不必借 then/catch 混排。 */
async function captureError(promise: Promise<unknown>): Promise<Error | null> {
  try {
    await promise;
    return null;
  } catch (cause) {
    return cause as Error;
  }
}

function providerOf(protocol: "openai-chat" | "anthropic-messages", fetchImpl: typeof fetch) {
  return createProvider(protocol, {
    baseUrl: "https://llm.example.com/v1",
    apiKey: "test-key",
    model: "test-model",
    fetchImpl,
  });
}

/** 非流式替身：把同一份语义结果走 JSON 响应返回（等价性对照）。 */
function jsonFetch(body: unknown, calls: Call[], status = 200): typeof fetch {
  return (async (url: unknown, init?: RequestInit) => {
    calls.push({ url: String(url), init: init ?? {} });
    return new Response(JSON.stringify(body), { status });
  }) as unknown as typeof fetch;
}

describe("completeStream — openai-chat", () => {
  it("SSE 增量聚合 == 非流式结果（含 usage 映射）", async () => {
    const pieces = sliceText(STORY_JSON, 13);
    const calls: Call[] = [];
    const provider = providerOf(
      "openai-chat",
      sseFetch([openAiSse(pieces, { prompt_tokens: 100, completion_tokens: 200 })], calls),
    );
    const progress = collectProgress();
    const reply = await provider.completeStream({ system: "s", user: "u" }, progress.onProgress);

    expect(reply.story).toEqual(STORY);
    expect(reply.usage).toEqual({ inputTokens: 100, outputTokens: 200 });
    // ④ 进度文本累计严格等于最终 story 文本（收尾分片 text 为空串）。
    expect(progress.text()).toBe(STORY_JSON);
    expect(progress.deltas.at(-1)).toEqual({
      text: "",
      inputTokens: 100,
      outputTokens: 200,
    });
    // 请求体：流式开关 + 严格子集 + 用量回传。
    expect(calls[0]!.url).toBe("https://llm.example.com/v1/chat/completions");
    const body = JSON.parse(String(calls[0]!.init.body)) as {
      stream?: boolean;
      stream_options?: { include_usage?: boolean };
      response_format?: { json_schema?: { schema?: unknown } };
    };
    expect(body.stream).toBe(true);
    expect(body.stream_options).toEqual({ include_usage: true });
    expect(body.response_format?.json_schema?.schema).toEqual(STORY_WIRE_SCHEMA);
    const headers = calls[0]!.init.headers as Record<string, string>;
    expect(headers.authorization).toMatch(/^Bearer /);
  });

  it("分片在任意位置切断（残缺分片缓冲）仍能聚合", async () => {
    const raw = openAiSse(sliceText(STORY_JSON, 40), { prompt_tokens: 7, completion_tokens: 9 });
    // 按 5 字符硬切：块边界落在 JSON 行中间、`data:` 前缀中间、`\n\n` 中间都覆盖。
    const calls: Call[] = [];
    const provider = providerOf("openai-chat", sseFetch(sliceText(raw, 5), calls));
    const progress = collectProgress();
    const reply = await provider.completeStream({ system: "s", user: "u" }, progress.onProgress);

    expect(reply.story).toEqual(STORY);
    expect(progress.text()).toBe(STORY_JSON);
  });

  it("多行 data 事件按规范拼接（\\n 作 token 间空白）", async () => {
    // 同一个 JSON 对象被拆到两条 data: 行（SSE 合法形态），拼接点是 token 边界。
    const payload = JSON.stringify({ choices: [{ delta: { content: STORY_JSON } }] });
    const cut = '{"choices":'.length;
    const raw =
      `data: ${payload.slice(0, cut)}\n` +
      `data: ${payload.slice(cut)}\n\n` +
      "data: [DONE]\n\n";
    const calls: Call[] = [];
    const provider = providerOf("openai-chat", sseFetch(sliceText(raw, 11), calls));
    const progress = collectProgress();
    const reply = await provider.completeStream({ system: "s", user: "u" }, progress.onProgress);

    expect(reply.story).toEqual(STORY);
    expect(progress.text()).toBe(STORY_JSON);
  });

  it("网关 400 拒绝严格档 → 降级重发一次（去掉 response_format / stream_options）", async () => {
    const calls: Call[] = [];
    const chunks = [openAiSse([STORY_JSON])];
    let round = 0;
    const fetchImpl = (async (url: unknown, init?: RequestInit) => {
      calls.push({ url: String(url), init: init ?? {} });
      round += 1;
      if (round === 1) {
        return new Response("null", { status: 400 });
      }
      return sseFetch(chunks, []) (url as string, init);
    }) as unknown as typeof fetch;

    const provider = providerOf("openai-chat", fetchImpl);
    const reply = await provider.completeStream({ system: "s", user: "u" }, () => {});
    expect(reply.story).toEqual(STORY);
    expect(calls).toHaveLength(2);
    const second = JSON.parse(String(calls[1]!.init.body)) as {
      stream?: boolean;
      response_format?: unknown;
      stream_options?: unknown;
    };
    expect(second.stream).toBe(true); // 流式必需字段保留
    expect(second.response_format).toBeUndefined();
    expect(second.stream_options).toBeUndefined();
  });

  it("流内无 content 分片 → 报错（不静默返回空 story）", async () => {
    const calls: Call[] = [];
    const provider = providerOf("openai-chat", sseFetch(["data: [DONE]\n\n"], calls));
    await expect(provider.completeStream({ system: "s", user: "u" }, () => {})).rejects.toThrow(
      /delta\.content/,
    );
  });

  it("非法 JSON 分片 → 报错且不回显分片原文", async () => {
    const leak = "secret-generated-text";
    const calls: Call[] = [];
    const provider = providerOf("openai-chat", sseFetch([`data: {${leak}\n\n`], calls));
    const error = await captureError(
      provider.completeStream({ system: "s", user: "u" }, () => {}),
    );
    expect(error?.message).toMatch(/不是合法 JSON/);
    expect(error?.message).not.toContain(leak); // 分片原文不进错误信息
  });
});

describe("completeStream — anthropic-messages", () => {
  it("input_json_delta 聚合 + message_delta usage == 非流式结果", async () => {
    const fragments = sliceText(STORY_JSON, 9);
    const calls: Call[] = [];
    const provider = providerOf(
      "anthropic-messages",
      sseFetch([anthropicSse(fragments, { input_tokens: 111, output_tokens: 222 })], calls),
    );
    const progress = collectProgress();
    const reply = await provider.completeStream({ system: "s", user: "u" }, progress.onProgress);

    expect(reply.story).toEqual(STORY);
    expect(reply.usage).toEqual({ inputTokens: 111, outputTokens: 222 });
    expect(progress.text()).toBe(STORY_JSON);
    // message_start 的 output_tokens 只是占位值（1），必须被 message_delta 覆盖。
    expect(progress.deltas.at(-1)).toEqual({
      text: "",
      inputTokens: 111,
      outputTokens: 222,
    });
    const body = JSON.parse(String(calls[0]!.init.body)) as {
      stream?: boolean;
      tools?: Array<{ input_schema: unknown }>;
      tool_choice?: unknown;
    };
    expect(body.stream).toBe(true);
    expect(body.tools?.[0]?.input_schema).toEqual(STORY_WIRE_SCHEMA);
    expect(body.tool_choice).toEqual({ type: "tool", name: "story_mod" });
  });

  it("前言 text_delta 不进 story 聚合，也不污染进度文本", async () => {
    const fragments = sliceText(STORY_JSON, 21);
    const calls: Call[] = [];
    const provider = providerOf(
      "anthropic-messages",
      sseFetch(
        [
          anthropicSse(
            fragments,
            { input_tokens: 5, output_tokens: 6 },
            { preamble: "我先想想。" },
          ),
        ],
        calls,
      ),
    );
    const progress = collectProgress();
    const reply = await provider.completeStream({ system: "s", user: "u" }, progress.onProgress);

    expect(reply.story).toEqual(STORY);
    expect(progress.text()).toBe(STORY_JSON);
    expect(progress.text()).not.toContain("我先想想");
  });

  it("缺少 tool_use 块 → 报错（与非流式同措辞）", async () => {
    const calls: Call[] = [];
    const raw =
      'event: message_start\ndata: {"type":"message_start","message":{"usage":{"input_tokens":1}}}\n\n' +
      'event: message_stop\ndata: {"type":"message_stop"}\n\n';
    const provider = providerOf("anthropic-messages", sseFetch([raw], calls));
    await expect(provider.completeStream({ system: "s", user: "u" }, () => {})).rejects.toThrow(
      /tool_use/,
    );
  });

  it("error 事件 → 只携 error.type（message 可能回显上下文，不进错误信息）", async () => {
    const calls: Call[] = [];
    const raw =
      'event: message_start\ndata: {"type":"message_start","message":{"usage":{"input_tokens":1}}}\n\n' +
      'event: error\ndata: {"type":"error","error":{"type":"overloaded_error","message":"secret-marker"}}\n\n';
    const provider = providerOf("anthropic-messages", sseFetch([raw], calls));
    const error = await captureError(
      provider.completeStream({ system: "s", user: "u" }, () => {}),
    );
    expect(error?.message).toContain("overloaded_error");
    expect(error?.message).not.toContain("secret-marker");
  });
});

describe("流式/非流式聚合等价性（同一 mock 语义）", () => {
  it("openai-chat：story 与 usage 完全一致", async () => {
    const nonStream = await providerOf(
      "openai-chat",
      jsonFetch(
        {
          choices: [{ message: { content: STORY_JSON } }],
          usage: { prompt_tokens: 42, completion_tokens: 58 },
        },
        [],
      ),
    ).complete({ system: "s", user: "u" });
    const stream = await providerOf(
      "openai-chat",
      sseFetch(
        [openAiSse(sliceText(STORY_JSON, 17), { prompt_tokens: 42, completion_tokens: 58 })],
        [],
      ),
    ).completeStream({ system: "s", user: "u" }, () => {});
    expect(stream).toEqual(nonStream);
  });

  it("anthropic-messages：story 与 usage 完全一致", async () => {
    const nonStream = await providerOf(
      "anthropic-messages",
      jsonFetch(
        {
          content: [{ type: "tool_use", input: STORY }],
          usage: { input_tokens: 33, output_tokens: 44 },
        },
        [],
      ),
    ).complete({ system: "s", user: "u" });
    const stream = await providerOf(
      "anthropic-messages",
      sseFetch(
        [anthropicSse(sliceText(STORY_JSON, 23), { input_tokens: 33, output_tokens: 44 })],
        [],
      ),
    ).completeStream({ system: "s", user: "u" }, () => {});
    expect(stream).toEqual(nonStream);
  });
});

describe("错误与超时（凭据红线 / 生命周期）", () => {
  it("HTTP 401 → ProviderHttpError 只携状态码，不含响应体与密钥", async () => {
    const calls: Call[] = [];
    const fetchImpl = (async (url: unknown, init?: RequestInit) => {
      calls.push({ url: String(url), init: init ?? {} });
      return new Response('{"error":{"message":"secret-marker"}}', { status: 401 });
    }) as unknown as typeof fetch;
    const provider = providerOf("openai-chat", fetchImpl);
    const error = await captureError(
      provider.completeStream({ system: "s", user: "u" }, () => {}),
    );
    expect(error).toBeInstanceOf(ProviderHttpError);
    expect((error as ProviderHttpError).status).toBe(401);
    expect(error?.message).toContain("401");
    expect(error?.message).not.toContain("secret-marker");
    expect(error?.message).not.toContain("test-key");
  });

  it("读流期间到点同样超时（响应头已到、分片不再推进）", async () => {
    let captured: AbortSignal | undefined;
    const fetchImpl = (async (_url: unknown, init?: RequestInit) => {
      const signal = init?.signal ?? undefined;
      captured = signal ?? undefined;
      const stream = new ReadableStream<Uint8Array>({
        start(controller) {
          controller.enqueue(
            encoder.encode('data: {"choices":[{"delta":{"content":"{\\"slug\\":\\"a\\"}"}}]}\n\n'),
          );
          // 故意不 close：模拟网关挂住（分片读到一半）。
          signal?.addEventListener("abort", () => {
            controller.error(new DOMException("The operation was aborted.", "AbortError"));
          });
        },
      });
      return new Response(stream, { status: 200 });
    }) as unknown as typeof fetch;

    const provider = createProvider("openai-chat", {
      baseUrl: "https://llm.example.com/v1",
      apiKey: "test-key",
      model: "test-model",
      fetchImpl,
      requestTimeoutMs: 20, // 注入短超时：默认 120s 不可测
    });
    await expect(provider.completeStream({ system: "s", user: "u" }, () => {})).rejects.toThrow();
    expect(captured?.aborted).toBe(true);
  });
});
