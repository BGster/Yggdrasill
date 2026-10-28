// ReplayKernel 选择流状态机 + fixture 完整性测试。
import { describe, expect, it } from "vitest";
import { ENDING_PREFIX, ReplayKernel, parseEnvelope } from "./kernel";
import transcriptJson from "./fixtures/adventure-transcript.json";

const fixture = transcriptJson as {
  branches: Record<string, { path: number[]; steps: unknown[] }>;
};

describe("fixture 完整性（真实内核采集物）", () => {
  it("两分支各 3 步，路径与结局标识正确", () => {
    expect(Object.keys(fixture.branches).sort()).toEqual(["a", "b"]);
    expect(fixture.branches.a.path).toEqual([1, 1]);
    expect(fixture.branches.b.path).toEqual([2, 2]);
    for (const key of ["a", "b"] as const) {
      for (const step of fixture.branches[key].steps) {
        const s = step as Record<string, { ok?: boolean }>;
        expect(s.envelope?.ok).toBe(true);
        expect(s.state?.ok).toBe(true);
        expect(s.save?.ok).toBe(true);
        expect(s.load?.ok).toBe(true);
      }
    }
  });
});

describe("ReplayKernel 选择流状态机", () => {
  it("start → 首屏：序章/酒馆叙事 + 酒馆选择面", () => {
    const kernel = new ReplayKernel();
    const envelope = kernel.start();
    expect(envelope.ok).toBe(true);
    expect(envelope.event).toBe("event.start");
    expect(envelope.narrative?.[0]).toContain("序章·雨夜");
    expect(envelope.pendingChoice?.prompt).toContain("铜提灯");
    expect(envelope.pendingChoice?.options).toEqual(["接过提灯", "轻装出发"]);
  });

  it("分支 a（1→1）：岔路 → 结局A，结局后无待决选择", () => {
    const kernel = new ReplayKernel();
    kernel.start();
    const crossroad = kernel.dispatch("ygg.choice_selected", "1");
    expect(crossroad.ok).toBe(true);
    expect(crossroad.narrative?.[0]).toContain("接过提灯");
    expect(crossroad.pendingChoice?.prompt).toContain("岔路");
    const ending = kernel.dispatch("ygg.choice_selected", "1");
    expect(ending.ok).toBe(true);
    const last = ending.narrative?.at(-1) ?? "";
    expect(last.startsWith(`${ENDING_PREFIX}A`)).toBe(true);
    expect(ending.pendingChoice ?? null).toBeNull();
  });

  it("分支 b（2→2）：轻装 → 客房 → 结局B", () => {
    const kernel = new ReplayKernel();
    kernel.start();
    kernel.dispatch("ygg.choice_selected", "2");
    const ending = kernel.dispatch("ygg.choice_selected", "2");
    const last = ending.narrative?.at(-1) ?? "";
    expect(last.startsWith(`${ENDING_PREFIX}B`)).toBe(true);
  });

  it("结局后再选择 → VALIDATION 收口语义", () => {
    const kernel = new ReplayKernel();
    kernel.start();
    kernel.dispatch("ygg.choice_selected", "1");
    kernel.dispatch("ygg.choice_selected", "1");
    const after = kernel.dispatch("ygg.choice_selected", "1");
    expect(after.ok).toBe(false);
    expect(after.error?.code).toBe("VALIDATION");
  });

  it("越界索引 → 真实内核 VALIDATION 回包（选项不存在）", () => {
    const kernel = new ReplayKernel();
    kernel.start();
    const reply = kernel.dispatch("ygg.choice_selected", "9");
    expect(reply.ok).toBe(false);
    expect(reply.error?.code).toBe("VALIDATION");
    expect(reply.error?.message).toBe("选项不存在：有效范围 1..2");
  });

  it("重复 start 幂等重放首屏（不推进）", () => {
    const kernel = new ReplayKernel();
    const first = kernel.start();
    const second = kernel.start();
    expect(second).toEqual(first);
    // 幂等重放后选择流不受影响。
    const crossroad = kernel.dispatch("ygg.choice_selected", "2");
    expect(crossroad.pendingChoice?.prompt).toContain("岔路");
  });

  it("保留命令：state / save / log / drain 信封形态", () => {
    const kernel = new ReplayKernel();
    kernel.start();
    const state = kernel.dispatch("state", "");
    expect(state.ok).toBe(true);
    expect(state.engineVersion).toBe("0.1.0");
    expect(state.snapshotBytes).toBeGreaterThan(0);
    const save = kernel.dispatch("save", "");
    expect(save.ok).toBe(true);
    expect(save.save?.engine_version).toBe("0.1.0");
    expect(kernel.dispatch("log", "")).toEqual({ ok: true, log: [] });
    expect(kernel.dispatch("drain", "")).toEqual({ ok: true, drained: [] });
  });

  it("未收录事件 / 未开始派发 → VALIDATION（mock 边界明示）", () => {
    const kernel = new ReplayKernel();
    expect(kernel.dispatch("event.x", "0").ok).toBe(false);
    kernel.start();
    const reply = kernel.dispatch("event.unknown", "0");
    expect(reply.ok).toBe(false);
    expect(reply.error?.code).toBe("VALIDATION");
    expect(reply.error?.message).toContain("未收录");
  });

  it("回放返回物与 fixture 隔离（深拷贝）", () => {
    const kernel = new ReplayKernel();
    const first = kernel.start();
    first.narrative?.push("（污染行）");
    const again = kernel.start();
    expect(again.narrative?.some((line) => line.includes("污染行"))).toBe(false);
  });

  it("采集信封可经 parseEnvelope 复检（契约面一致）", () => {
    const kernel = new ReplayKernel();
    const envelope = parseEnvelope(JSON.stringify(kernel.start()));
    expect(envelope.ok).toBe(true);
  });
});
