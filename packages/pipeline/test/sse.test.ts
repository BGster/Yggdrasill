// sse.test.ts — SSE 分帧器单测：协议层只做分帧（不解释 data 语义），
// 重点覆盖网关分片的两块硬骨头：跨块缓冲、多行 data 拼接。
import { describe, expect, it } from "vitest";
import { readSseEvents, SseParser } from "../src/sse.js";

describe("SseParser（SSE 分帧）", () => {
  it("未收齐一行时不吐事件，凑齐行 + 空行才派发", () => {
    const parser = new SseParser();
    expect(parser.push('data: {"a"')).toEqual([]);
    expect(parser.push(':1}\n')).toEqual([]); // 行完整但事件未收尾
    expect(parser.push("\n")).toEqual([{ event: null, data: '{"a":1}' }]);
  });

  it("多行 data 按规范以 \\n 连接成一个事件载荷", () => {
    const parser = new SseParser();
    expect(parser.push("data: line-1\ndata: line-2\n\n")).toEqual([
      { event: null, data: "line-1\nline-2" },
    ]);
  });

  it("event 字段随事件携带，下个事件重置", () => {
    const parser = new SseParser();
    expect(parser.push("event: message_start\ndata: {}\n\n")).toEqual([
      { event: "message_start", data: "{}" },
    ]);
    expect(parser.push("data: {}\n\n")).toEqual([{ event: null, data: "{}" }]);
  });

  it("注释行（心跳）被忽略且不打断事件", () => {
    const parser = new SseParser();
    expect(parser.push(": keep-alive\n\ndata: x\n\n")).toEqual([{ event: null, data: "x" }]);
  });

  it("CRLF 跨块切断仍能正确分帧（\\r 与 \\n 分属两块）", () => {
    const parser = new SseParser();
    expect(parser.push("data: crlf\r")).toEqual([]);
    expect(parser.push("\n\r\n")).toEqual([{ event: null, data: "crlf" }]);
  });

  it("流结束时 flush 冲刷未以空行收尾的残余事件（网关提前关流）", () => {
    const parser = new SseParser();
    expect(parser.push("data: tail")).toEqual([]);
    expect(parser.flush()).toEqual([{ event: null, data: "tail" }]);
    expect(parser.flush()).toEqual([]); // 幂等：无残余不再吐事件
  });
});

describe("readSseEvents（ReadableStream → 事件序列）", () => {
  const encoder = new TextEncoder();

  function byteStream(chunks: Uint8Array[]): ReadableStream<Uint8Array> {
    return new ReadableStream<Uint8Array>({
      start(controller) {
        for (const chunk of chunks) {
          controller.enqueue(chunk);
        }
        controller.close();
      },
    });
  }

  it("多字节字符被切断在块边界时不会损坏（TextDecoder 流式解码）", async () => {
    const bytes = encoder.encode("data: 中文载荷\n\n");
    // 逐字节喂入：UTF-8 三字节序列必然被切开。
    const chunks = Array.from(bytes, (byte) => Uint8Array.of(byte));
    const events = [];
    for await (const event of readSseEvents(byteStream(chunks))) {
      events.push(event);
    }
    expect(events).toEqual([{ event: null, data: "中文载荷" }]);
  });

  it("按 [DONE] 提前结束读取（break 不挂住流）", async () => {
    const stream = byteStream([
      encoder.encode("data: a\n\n"),
      encoder.encode("data: [DONE]\n\n"),
      encoder.encode("data: late\n\n"),
    ]);
    const seen: string[] = [];
    for await (const event of readSseEvents(stream)) {
      if (event.data === "[DONE]") {
        break;
      }
      seen.push(event.data);
    }
    expect(seen).toEqual(["a"]);
  });
});
