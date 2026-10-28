import { describe, expect, it } from "vitest";
import {
  createProvider,
  parseStoryJson,
  ProviderHttpError,
  STORY_WIRE_SCHEMA,
} from "../src/provider.js";

const STORY = {
  slug: "demo",
  title: "演示",
  modLua: "-- demo",
  scenes: { scenes: {}, endings: {}, prompts: {}, options: {}, beats: {}, tuning: {} },
};

function fetchJson(body: unknown, status = 200): typeof fetch {
  return (async () =>
    new Response(JSON.stringify(body), { status })) as unknown as typeof fetch;
}

function captureFetch(responses: Array<{ status: number; body: unknown }>): {
  fetchImpl: typeof fetch;
  calls: Array<{ url: string; init: RequestInit }>; // 注意：headers 含 apiKey，断言只看键存在，不落值。
} {
  const calls: Array<{ url: string; init: RequestInit }> = [];
  let index = 0;
  const fetchImpl = (async (url: unknown, init?: RequestInit) => {
    calls.push({ url: String(url), init: init ?? {} });
    const response = responses[Math.min(index, responses.length - 1)]!;
    index += 1;
    return new Response(JSON.stringify(response.body), { status: response.status });
  }) as unknown as typeof fetch;
  return { fetchImpl, calls };
}

describe("provider（D-13 双协议 + 优雅降级）", () => {
  it("openai-chat 正常路径：content 解析 + usage 映射 + wire schema 在请求体内", async () => {
    const { fetchImpl, calls } = captureFetch([
      {
        status: 200,
        body: {
          choices: [{ message: { content: JSON.stringify(STORY) } }],
          usage: { prompt_tokens: 100, completion_tokens: 200 },
        },
      },
    ]);
    const provider = createProvider("openai-chat", {
      baseUrl: "https://llm.example.com/v1/",
      apiKey: "test-key",
      model: "test-model",
      fetchImpl,
    });
    const reply = await provider.complete({ system: "s", user: "u" });
    expect(reply.story).toEqual(STORY);
    expect(reply.usage).toEqual({ inputTokens: 100, outputTokens: 200 });
    expect(calls[0]!.url).toBe("https://llm.example.com/v1/chat/completions");
    const body = JSON.parse(String(calls[0]!.init.body)) as {
      response_format?: { json_schema?: { schema?: unknown } };
      messages?: unknown[];
    };
    expect(body.response_format?.json_schema?.schema).toEqual(STORY_WIRE_SCHEMA);
    expect(body.messages).toHaveLength(2);
    const headers = calls[0]!.init.headers as Record<string, string>;
    expect(headers.authorization).toMatch(/^Bearer /); // 键值只经内存进头，不落日志。
  });

  it("openai-chat 网关 400 拒绝 response_format → 降级重发一次（不带 response_format）", async () => {
    const { fetchImpl, calls } = captureFetch([
      { status: 400, body: { error: { message: "response_format unsupported" } } },
      { status: 200, body: { choices: [{ message: { content: JSON.stringify(STORY) } }] } },
    ]);
    const provider = createProvider("openai-chat", {
      baseUrl: "https://llm.example.com/v1",
      apiKey: "k",
      model: "m",
      fetchImpl,
    });
    const reply = await provider.complete({ system: "s", user: "u" });
    expect(reply.story).toEqual(STORY);
    expect(calls).toHaveLength(2);
    const second = JSON.parse(String(calls[1]!.init.body)) as { response_format?: unknown };
    expect(second.response_format).toBeUndefined();
  });

  it("anthropic-messages：tool_use input 即 story + usage 映射", async () => {
    const { fetchImpl, calls } = captureFetch([
      {
        status: 200,
        body: {
          content: [{ type: "tool_use", input: STORY }],
          usage: { input_tokens: 111, output_tokens: 222 },
        },
      },
    ]);
    const provider = createProvider("anthropic-messages", {
      baseUrl: "http://localhost:7878",
      apiKey: "k",
      model: "kimi-k3",
      fetchImpl,
    });
    const reply = await provider.complete({ system: "s", user: "u" });
    expect(reply.story).toEqual(STORY);
    expect(reply.usage).toEqual({ inputTokens: 111, outputTokens: 222 });
    expect(calls[0]!.url).toBe("http://localhost:7878/v1/messages");
    const body = JSON.parse(String(calls[0]!.init.body)) as {
      tool_choice?: { type: string; name: string };
      tools?: Array<{ input_schema: unknown }>;
    };
    expect(body.tool_choice).toEqual({ type: "tool", name: "story_mod" });
    expect(body.tools?.[0]?.input_schema).toEqual(STORY_WIRE_SCHEMA);
  });

  it("anthropic-messages 缺 tool_use 块 → 报错（不静默）", async () => {
    const provider = createProvider("anthropic-messages", {
      baseUrl: "http://localhost:7878",
      apiKey: "k",
      model: "m",
      fetchImpl: fetchJson({ content: [{ type: "text", text: "hi" }] }),
    });
    await expect(provider.complete({ system: "s", user: "u" })).rejects.toThrow(/tool_use/);
  });

  it("parseStoryJson 容忍 markdown 围栏残留（降级路径）", () => {
    expect(parseStoryJson('```json\n{"a":1}\n```', "openai-chat")).toEqual({ a: 1 });
    expect(() => parseStoryJson("not json", "openai-chat")).toThrow(/合法 JSON/);
  });

  it("HTTP 401 → ProviderHttpError 只携状态码", async () => {
    const provider = createProvider("openai-chat", {
      baseUrl: "https://x/v1",
      apiKey: "k",
      model: "m",
      fetchImpl: fetchJson({}, 401),
    });
    await expect(provider.complete({ system: "s", user: "u" })).rejects.toThrow(ProviderHttpError);
  });
});
