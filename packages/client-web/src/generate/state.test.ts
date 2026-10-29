// state.test.ts — 生成面板纯状态机：起跑清结果面 / 事件折叠 / 终态不可回退。
import { describe, expect, it } from "vitest";
import {
  failGenerate,
  initialGenerateState,
  reduceGenerateEvent,
  startGenerate,
} from "./state";

const preview = {
  type: "preview" as const,
  title: "暴风雨夜的灯塔",
  scenes: [{ id: "storm", text: "暴风雨夜。" }],
  endings: [{ id: "dawn", text: "【结局晨光】" }],
};

describe("reduceGenerateEvent", () => {
  it("progress 更新计数（含重试轮次）", () => {
    let state = startGenerate(initialGenerateState());
    state = reduceGenerateEvent(state, {
      type: "progress",
      round: 2,
      tokens: 8432,
      scenes: 3,
      endings: 1,
    });
    expect(state).toMatchObject({ phase: "running", round: 2, tokens: 8432, scenes: 3, endings: 1 });
  });

  it("preview 更新预览面（不覆盖计数）", () => {
    let state = startGenerate(initialGenerateState());
    state = reduceGenerateEvent(state, {
      type: "progress",
      round: 1,
      tokens: 500,
      scenes: 1,
      endings: 0,
    });
    state = reduceGenerateEvent(state, preview);
    expect(state).toMatchObject({
      phase: "running",
      tokens: 500,
      title: "暴风雨夜的灯塔",
      previewScenes: [{ id: "storm", text: "暴风雨夜。" }],
      previewEndings: [{ id: "dawn", text: "【结局晨光】" }],
    });
  });

  it("done → phase=done + slug", () => {
    const state = reduceGenerateEvent(startGenerate(initialGenerateState()), {
      type: "done",
      slug: "lighthouse-keeper",
    });
    expect(state).toMatchObject({ phase: "done", slug: "lighthouse-keeper" });
  });

  it("error → phase=error + 文案与 issues", () => {
    const state = reduceGenerateEvent(startGenerate(initialGenerateState()), {
      type: "error",
      message: "重试额度耗尽",
      issues: [{ gate: "playtest", code: "PLAYTEST", file: "mod.lua", message: "未达验收锚" }],
    });
    expect(state.phase).toBe("error");
    expect(state.message).toBe("重试额度耗尽");
    expect(state.issues).toHaveLength(1);
  });

  it("终态之后的事件一律忽略（迟到的 progress 不能把 done 翻回 running）", () => {
    const done = reduceGenerateEvent(startGenerate(initialGenerateState()), {
      type: "done",
      slug: "a",
    });
    expect(
      reduceGenerateEvent(done, { type: "progress", round: 1, tokens: 9, scenes: 0, endings: 0 }),
    ).toBe(done);
    expect(reduceGenerateEvent(done, preview)).toBe(done);

    const errored = failGenerate(startGenerate(initialGenerateState()), "boom");
    expect(reduceGenerateEvent(errored, { type: "done", slug: "b" })).toBe(errored);
  });
});

describe("起跑与失败面", () => {
  it("startGenerate 清掉上一轮结果（重跑不残留旧预览与旧错误）", () => {
    const finished = reduceGenerateEvent(startGenerate(initialGenerateState()), {
      type: "done",
      slug: "old-slug",
    });
    const restarted = startGenerate(finished);
    expect(restarted).toMatchObject({
      phase: "running",
      slug: null,
      tokens: 0,
      scenes: 0,
      title: null,
      previewScenes: [],
      message: null,
      issues: [],
    });
  });

  it("failGenerate 落失败态", () => {
    const state = failGenerate(startGenerate(initialGenerateState()), "网络错误");
    expect(state).toMatchObject({ phase: "error", message: "网络错误", issues: [] });
  });
});
