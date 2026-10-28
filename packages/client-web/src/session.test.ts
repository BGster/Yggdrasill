// 会话状态机（session reducer）测试。
import { describe, expect, it } from "vitest";
import type { Envelope } from "./kernel";
import {
  applyEventEnvelope,
  initialSession,
  optionIndexFromKey,
  withSystemLine,
} from "./session";

const okEnvelope: Envelope = {
  ok: true,
  event: "event.start",
  narrative: ["序章·雨夜", "酒馆·大堂"],
  pendingChoice: { prompt: "带灯吗？", options: ["接过提灯", "轻装出发"] },
  eventsProcessed: 3,
  eventsDerived: 2,
  eventsDropped: 0,
  revision: 5,
};

describe("applyEventEnvelope", () => {
  it("ok 信封：叙事按序入账 + 待决选择更新 + started", () => {
    const next = applyEventEnvelope(initialSession(), okEnvelope);
    expect(next.started).toBe(true);
    expect(next.entries.map((e) => e.kind)).toEqual(["narrative", "narrative"]);
    expect(next.entries[0]).toMatchObject({ kind: "narrative", text: "序章·雨夜" });
    expect(next.pendingChoice?.options).toHaveLength(2);
    expect(next.ended).toBe(false);
  });

  it("warnings → ⚠ 警告条目（排在叙事之后）", () => {
    const next = applyEventEnvelope(initialSession(), {
      ...okEnvelope,
      warnings: ["事件 `event.go` 的 handler 在 ygg.choice 之前产生了 2 条派生事件（ygg.emit），已按语义丢弃（choice 须为 handler 最后一个输出调用）"],
    });
    expect(next.entries.map((e) => e.kind)).toEqual(["narrative", "narrative", "warning"]);
    expect(next.entries[2]).toMatchObject({ kind: "warning" });
  });

  it("错误信封：错误条目（码+文案）且保留原待决选择", () => {
    const withChoice = applyEventEnvelope(initialSession(), okEnvelope);
    const next = applyEventEnvelope(withChoice, {
      ok: false,
      error: { code: "VALIDATION", message: "选项不存在：有效范围 1..2" },
    });
    const last = next.entries.at(-1);
    expect(last).toMatchObject({ kind: "error", code: "VALIDATION" });
    expect(next.pendingChoice?.prompt).toBe(withChoice.pendingChoice?.prompt);
    expect(next.ended).toBe(false);
  });

  it("结局叙事行：ended 收口 + 系统收口行 + 待决选择清空", () => {
    const next = applyEventEnvelope(initialSession(), {
      ok: true,
      narrative: ["地窖·灯光所及", "【结局A】灯下秘藏"],
      pendingChoice: null,
    });
    expect(next.ended).toBe(true);
    expect(next.pendingChoice).toBeNull();
    expect(next.entries.at(-1)).toMatchObject({ kind: "system", text: "—— 冒险结束 ——" });
  });

  it("结局后 pendingChoice 非空的信封不再点亮选择面", () => {
    const next = applyEventEnvelope(initialSession(), {
      ok: true,
      narrative: ["【结局B】平安一夜"],
      pendingChoice: { prompt: "不该出现", options: ["x"] },
    });
    expect(next.ended).toBe(true);
    expect(next.pendingChoice).toBeNull();
  });

  it("条目 id 单调递增（渲染 key 稳定）", () => {
    let state = applyEventEnvelope(initialSession(), okEnvelope);
    state = withSystemLine(state, "已存档");
    const ids = state.entries.map((e) => e.id);
    expect([...ids].sort((a, b) => a - b)).toEqual(ids);
    expect(new Set(ids).size).toBe(ids.length);
  });
});

describe("withSystemLine", () => {
  it("系统行入账且不动待决选择/结局态", () => {
    const withChoice = applyEventEnvelope(initialSession(), okEnvelope);
    const next = withSystemLine(withChoice, "状态 engine=0.1.0 revision=5");
    expect(next.entries.at(-1)).toMatchObject({ kind: "system", text: /状态 engine=/ });
    expect(next.pendingChoice).toEqual(withChoice.pendingChoice);
    expect(next.ended).toBe(false);
  });
});

describe("optionIndexFromKey（键盘 1-9 映射）", () => {
  const twoOptions = 2;

  it("有效数字映射 1-based 序号", () => {
    expect(optionIndexFromKey("1", twoOptions)).toBe(1);
    expect(optionIndexFromKey("2", twoOptions)).toBe(2);
    expect(optionIndexFromKey("9", 9)).toBe(9);
  });

  it("越界/非数字返回 null", () => {
    expect(optionIndexFromKey("0", twoOptions)).toBeNull();
    expect(optionIndexFromKey("3", twoOptions)).toBeNull();
    expect(optionIndexFromKey("9", twoOptions)).toBeNull();
    expect(optionIndexFromKey("a", twoOptions)).toBeNull();
    expect(optionIndexFromKey("Enter", twoOptions)).toBeNull();
    expect(optionIndexFromKey("", twoOptions)).toBeNull();
  });

  it("无待决选择（0 选项）一律 null", () => {
    expect(optionIndexFromKey("1", 0)).toBeNull();
  });
});
