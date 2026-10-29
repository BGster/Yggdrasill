// protocol.test.ts — /__generate wire 契约的客户端侧解析：形状不符 / 未知类型 /
// 坏 JSON 一律返回 null（忽略该事件，不打断整条流）；合法事件逐字段保真。
import { describe, expect, it } from "vitest";
import { parseGenerateEvent } from "./protocol";

describe("parseGenerateEvent — 合法事件", () => {
  it("progress", () => {
    const event = parseGenerateEvent('{"type":"progress","round":2,"tokens":8432,"scenes":3,"endings":1}');
    expect(event).toEqual({ type: "progress", round: 2, tokens: 8432, scenes: 3, endings: 1 });
  });

  it("preview（title 可为 null，条目逐字保真）", () => {
    const payload = {
      type: "preview",
      title: null,
      scenes: [{ id: "storm", text: "暴风雨夜。" }],
      endings: [],
    };
    expect(parseGenerateEvent(JSON.stringify(payload))).toEqual(payload);
  });

  it("done", () => {
    expect(parseGenerateEvent('{"type":"done","slug":"lighthouse-keeper"}')).toEqual({
      type: "done",
      slug: "lighthouse-keeper",
    });
  });

  it("error（issues 只留可展示四字段）", () => {
    const payload = {
      type: "error",
      message: "重试额度耗尽",
      issues: [{ gate: "lint", code: "YGG-R1", file: "mod.lua:3", message: "emit before choice" }],
    };
    expect(parseGenerateEvent(JSON.stringify(payload))).toEqual(payload);
  });
});

describe("parseGenerateEvent — 忽略面", () => {
  it("坏 JSON / 非对象 → null", () => {
    expect(parseGenerateEvent("not json")).toBeNull();
    expect(parseGenerateEvent("")).toBeNull();
    expect(parseGenerateEvent("[]")).toBeNull();
    expect(parseGenerateEvent("null")).toBeNull();
  });

  it("未知类型 → null（前向兼容：新事件不影响旧面板）", () => {
    expect(parseGenerateEvent('{"type":"heartbeat"}')).toBeNull();
    expect(parseGenerateEvent('{"noType":1}')).toBeNull();
  });

  it("字段缺失或类型不符 → null（不是半信半疑地渲染）", () => {
    expect(parseGenerateEvent('{"type":"progress","tokens":1}')).toBeNull();
    expect(parseGenerateEvent('{"type":"progress","round":1,"tokens":"1","scenes":0,"endings":0}')).toBeNull();
    expect(parseGenerateEvent('{"type":"preview","title":1,"scenes":[],"endings":[]}')).toBeNull();
    expect(parseGenerateEvent('{"type":"preview","title":"x","scenes":[{"id":1}],"endings":[]}')).toBeNull();
    expect(parseGenerateEvent('{"type":"done","slug":""}')).toBeNull();
    expect(parseGenerateEvent('{"type":"error","message":"x","issues":[{"gate":"lint"}]}')).toBeNull();
  });

  it("事件里的额外字段被丢弃（wire 面收敛，不透传内部字段）", () => {
    const event = parseGenerateEvent(
      '{"type":"done","slug":"a","outDir":"/Users/secret/path","apiKey":"leak"}',
    );
    expect(event).toEqual({ type: "done", slug: "a" });
  });
});
