// 信封解析契约测试（parseEnvelope）。
import { describe, expect, it } from "vitest";
import { parseEnvelope } from "./kernel";

describe("parseEnvelope", () => {
  it("解析成功事件信封（字段保真）", () => {
    const raw = JSON.stringify({
      ok: true,
      event: "event.start",
      narrative: ["序章", "酒馆"],
      pendingChoice: { prompt: "带灯吗？", options: ["接过提灯", "轻装出发"] },
      eventsProcessed: 3,
      eventsDerived: 2,
      eventsDropped: 0,
      revision: 5,
    });
    const envelope = parseEnvelope(raw);
    expect(envelope.ok).toBe(true);
    expect(envelope.event).toBe("event.start");
    expect(envelope.narrative).toEqual(["序章", "酒馆"]);
    expect(envelope.pendingChoice?.options).toHaveLength(2);
    expect(envelope.revision).toBe(5);
  });

  it("解析错误信封（ok:false + error.code/message）", () => {
    const envelope = parseEnvelope(
      JSON.stringify({ ok: false, error: { code: "VALIDATION", message: "选项不存在：有效范围 1..2" } }),
    );
    expect(envelope.ok).toBe(false);
    expect(envelope.error?.code).toBe("VALIDATION");
    expect(envelope.error?.message).toContain("选项不存在");
  });

  it("非法 JSON 抛人读中文错误", () => {
    expect(() => parseEnvelope("not json")).toThrow(/信封不是合法 JSON/);
  });

  it("非对象信封（数组/标量）抛错", () => {
    expect(() => parseEnvelope("[1,2]")).toThrow(/信封必须是 JSON 对象/);
    expect(() => parseEnvelope("42")).toThrow(/信封必须是 JSON 对象/);
  });

  it("缺 ok 布尔字段的信封抛错", () => {
    expect(() => parseEnvelope(JSON.stringify({ event: "event.x" }))).toThrow(/缺少布尔字段 ok/);
  });

  it("错误信封缺 error.code / error.message 抛错", () => {
    expect(() => parseEnvelope(JSON.stringify({ ok: false, error: { code: 1 } }))).toThrow(
      /error\.code \/ error\.message/,
    );
  });
});
