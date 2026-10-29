// sse.ts — SSE（text/event-stream）分帧与增量解析（provider 双协议流式共用）。
// 只做协议层：把任意切分的字节块还原成 { event, data } 事件序列，不解释 data
// 语义（JSON 含义归各 provider 分支）。两块硬骨头都在这里：
//   1. 分片缓冲必须跨块保留——网关常在一个 JSON 行中间断开，chunk 边界不可预测；
//   2. 多行 data 按规范以 \n 连接成一个事件载荷（单事件多行是合法 SSE）。

export interface SseEvent {
  /** `event:` 字段（缺省 null；openai-chat 不带，anthropic-messages 必带）。 */
  event: string | null;
  /** 该事件的 data 载荷（多行 data: 已按规范以 \n 连接）。 */
  data: string;
}

/** 增量分帧器：喂入解码后的文本，吐出凑齐的事件（纯同步，易测）。 */
export class SseParser {
  private buffer = "";
  private dataLines: string[] = [];
  private eventName: string | null = null;

  /** 喂入一段文本（允许在任意位置切断），返回本段凑齐的完整事件。 */
  push(text: string): SseEvent[] {
    this.buffer += text;
    const events: SseEvent[] = [];
    for (;;) {
      const newline = this.buffer.indexOf("\n");
      if (newline < 0) {
        break; // 未收齐一行：留在 buffer 等下一块（残缺分片缓冲）。
      }
      const line = this.buffer.slice(0, newline).replace(/\r$/, ""); // CRLF → LF
      this.buffer = this.buffer.slice(newline + 1);
      this.consumeLine(line, events);
    }
    return events;
  }

  /** 流结束：冲刷未以空行收尾的残余事件（网关提前关流时的容错）。 */
  flush(): SseEvent[] {
    const events: SseEvent[] = [];
    const rest = this.buffer.replace(/\r$/, "");
    this.buffer = "";
    if (rest.length > 0) {
      this.consumeLine(rest, events);
    }
    this.dispatch(events);
    return events;
  }

  private consumeLine(line: string, events: SseEvent[]): void {
    if (line === "") {
      this.dispatch(events); // 空行 = 事件边界
      return;
    }
    if (line.startsWith(":")) {
      return; // 注释行（心跳 `: keep-alive`），无语义
    }
    const colon = line.indexOf(":");
    const field = colon < 0 ? line : line.slice(0, colon);
    let value = colon < 0 ? "" : line.slice(colon + 1);
    if (value.startsWith(" ")) {
      value = value.slice(1); // 规范：字段值与冒号之间最多一个空格
    }
    if (field === "data") {
      this.dataLines.push(value);
    } else if (field === "event") {
      this.eventName = value;
    }
    // id / retry 与本管线的语义无关（无断线重连），忽略。
  }

  private dispatch(events: SseEvent[]): void {
    if (this.dataLines.length > 0) {
      events.push({ event: this.eventName, data: this.dataLines.join("\n") });
    }
    this.dataLines = [];
    this.eventName = null;
  }
}

/**
 * 把 fetch 的响应体转成事件序列。
 * 读流期间同样受调用方传入的 AbortSignal 约束：分片读到一半被 abort 时，
 * reader.read() 会 reject，异常原样上抛（由 provider 归入超时/中止语义）。
 * 提前结束（如 openai 的 [DONE]）时 cancel 连接，避免网关侧挂住不收尾。
 */
export async function* readSseEvents(
  body: ReadableStream<Uint8Array>,
): AsyncGenerator<SseEvent> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  const parser = new SseParser();
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) {
        break;
      }
      // stream: true → 多字节字符被切断时留待下一块拼接（不产生替换字符）。
      for (const event of parser.push(decoder.decode(value, { stream: true }))) {
        yield event;
      }
    }
    for (const event of parser.flush()) {
      yield event;
    }
  } finally {
    try {
      await reader.cancel();
    } catch {
      // 已自然结束或被 abort：取消是无害收尾，不覆盖原始异常。
    }
  }
}
