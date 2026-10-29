// state.test.ts — 修改面板状态机：起跑清结果面 / 成功累积轮次 / 被拒落诊断 /
// 端点错误不递增轮次。
import { describe, expect, it } from "vitest";
import {
  applyReviseResponse,
  failRevise,
  initialReviseState,
  startRevise,
} from "./state";

describe("revise 状态机", () => {
  it("起始是 idle 且零轮次", () => {
    expect(initialReviseState()).toEqual({
      phase: "idle",
      changes: [],
      noop: false,
      message: null,
      issues: [],
      rounds: 0,
    });
  });

  it("起跑清掉上一轮摘要/诊断，保留轮次", () => {
    const afterSuccess = applyReviseResponse(startRevise(initialReviseState()), {
      ok: true,
      slug: "x1",
      changes: ["modLua"],
      noop: false,
    });
    expect(afterSuccess.rounds).toBe(1);

    const restarted = startRevise(afterSuccess);
    expect(restarted).toEqual({
      phase: "running",
      changes: [],
      noop: false,
      message: null,
      issues: [],
      rounds: 1,
    });
  });

  it("成功：changes 副本 + rounds 递增；noop 传递", () => {
    const running = startRevise(initialReviseState());
    const done = applyReviseResponse(running, {
      ok: true,
      slug: "x1",
      changes: ["scenes.scenes.storm", "scenes.endings.dawn"],
      noop: false,
    });
    expect(done.phase).toBe("done");
    expect(done.changes).toEqual(["scenes.scenes.storm", "scenes.endings.dawn"]);
    expect(done.rounds).toBe(1);
    expect(done.message).toBeNull();

    const noop = applyReviseResponse(startRevise(done), {
      ok: true,
      slug: "x1",
      changes: [],
      noop: true,
    });
    expect(noop.noop).toBe(true);
    expect(noop.rounds).toBe(2);
  });

  it("被拒（ok:false）：落诊断，不填 message，不递增轮次", () => {
    const running = startRevise(initialReviseState());
    const failed = applyReviseResponse(running, {
      ok: false,
      issues: [{ gate: "revise", code: "REVISE_LUA_FROZEN", file: "mod.lua", message: "冻结" }],
    });
    expect(failed.phase).toBe("error");
    expect(failed.message).toBeNull();
    expect(failed.issues).toHaveLength(1);
    expect(failed.rounds).toBe(0);
  });

  it("端点/网络错误：message 落文案，清上一轮结果面", () => {
    const done = applyReviseResponse(startRevise(initialReviseState()), {
      ok: true,
      slug: "x1",
      changes: ["modLua"],
      noop: false,
    });
    const failed = failRevise(startRevise(done), "修改端点返回 HTTP 503");
    expect(failed.phase).toBe("error");
    expect(failed.message).toBe("修改端点返回 HTTP 503");
    expect(failed.changes).toEqual([]);
    expect(failed.issues).toEqual([]);
    expect(failed.rounds).toBe(1);
  });
});
