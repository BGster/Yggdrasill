// client.test.ts — 浏览器侧流式消费：SSE 分帧跨块缓冲、非法事件忽略、HTTP 错误文案。
// 注入 fetchImpl（不触网），响应体按任意长度硬切（块边界可落在事件中间）。
import { describe, expect, it } from "vitest";
import { GENERATE_ENDPOINT, runGenerate } from "./client";
import type { GenerateEvent } from "./protocol";

const encoder = new TextEncoder();

interface Call {
  url: string;
  init: RequestInit;
}

function streamResponse(chunks: string[], status = 200): Response {
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const chunk of chunks) {
        controller.enqueue(encoder.encode(chunk));
      }
      controller.close();
    },
  });
  return new Response(body, { status, headers: { "content-type": "text/event-stream" } });
}

function sseFetch(chunks: string[], calls: Call[]): typeof fetch {
  return (async (url: unknown, init?: RequestInit) => {
    calls.push({ url: String(url), init: init ?? {} });
    return streamResponse(chunks);
  }) as unknown as typeof fetch;
}

function slice(text: string, size: number): string[] {
  const chunks: string[] = [];
  for (let i = 0; i < text.length; i += size) {
    chunks.push(text.slice(i, i + size));
  }
  return chunks;
}

const EVENTS: GenerateEvent[] = [
  { type: "progress", round: 1, tokens: 1200, scenes: 0, endings: 0 },
  {
    type: "preview",
    title: "暴风雨夜的灯塔",
    scenes: [{ id: "storm", text: "暴风雨夜，灯塔的光柱在浪涛上摇晃。" }],
    endings: [],
  },
  { type: "progress", round: 1, tokens: 2400, scenes: 1, endings: 0 },
  { type: "done", slug: "lighthouse-keeper" },
];

const SSE_TEXT = EVENTS.map((event) => `data: ${JSON.stringify(event)}\n\n`).join("");

describe("runGenerate", () => {
  it("POST 剧本描述并逐事件回调（SSE 文本按 7 字符硬切仍完整）", async () => {
    const calls: Call[] = [];
    const received: GenerateEvent[] = [];
    await runGenerate("一个关于灯塔看守的悬疑短篇", (event) => received.push(event), {
      fetchImpl: sseFetch(slice(SSE_TEXT, 7), calls),
    });
    expect(calls).toHaveLength(1);
    expect(calls[0].url).toBe(GENERATE_ENDPOINT);
    expect(calls[0].init.method).toBe("POST");
    expect(JSON.parse(String(calls[0].init.body))).toEqual({
      story: "一个关于灯塔看守的悬疑短篇",
    });
    expect(received).toEqual(EVENTS);
  });

  it("非法 / 未知事件被忽略，不打断流", async () => {
    const noisy =
      "data: 这不是 JSON\n\n" +
      'data: {"type":"heartbeat"}\n\n' +
      `data: ${JSON.stringify(EVENTS[3])}\n\n`;
    const received: GenerateEvent[] = [];
    await runGenerate("x", (event) => received.push(event), {
      fetchImpl: sseFetch([noisy], []),
    });
    expect(received).toEqual([EVENTS[3]]);
  });

  it("非 2xx + JSON error → 抛错并带服务端文案（只含键名，不含值）", async () => {
    const fetchImpl = (async () =>
      new Response(JSON.stringify({ error: "缺少 LLM 环境变量：YGG_LLM_API_KEY" }), {
        status: 503,
      })) as unknown as typeof fetch;
    let thrown: Error | null = null;
    try {
      await runGenerate("x", () => {}, { fetchImpl });
    } catch (error) {
      thrown = error as Error;
    }
    expect(thrown?.message).toContain("YGG_LLM_API_KEY");
  });

  it("非 2xx 且响应体不是 JSON → 退化为状态码文案", async () => {
    const fetchImpl = (async () => new Response("<html>502</html>", { status: 502 })) as unknown as typeof fetch;
    let thrown: Error | null = null;
    try {
      await runGenerate("x", () => {}, { fetchImpl });
    } catch (error) {
      thrown = error as Error;
    }
    expect(thrown?.message).toContain("502");
  });
});
