// 存读档往返测试（经 KernelBridge mock 的 save/load 面）。
import { describe, expect, it } from "vitest";
import { ReplayKernel } from "./kernel";

describe("ReplayKernel save/load 往返", () => {
  it("中盘存档 → 读回：restored + 待决选择恢复，续玩接原分支", () => {
    const player = new ReplayKernel();
    player.start();
    player.dispatch("ygg.choice_selected", "1"); // 岔路（分支 a 中盘）

    const saveEnvelope = player.save();
    expect(saveEnvelope.ok).toBe(true);
    const saveJson = JSON.stringify(saveEnvelope.save);

    // 读档方：全新内核实例（模拟重开进程后读档）。
    const restorer = new ReplayKernel();
    restorer.start();
    const reply = restorer.load(saveJson);
    expect(reply.ok).toBe(true);
    expect(reply.restored).toBe(true);
    expect(reply.pendingChoice?.prompt).toContain("岔路");

    // 恢复后续玩：同一存档位继续 1 → 结局A。
    const ending = restorer.dispatch("ygg.choice_selected", "1");
    expect(ending.narrative?.at(-1)).toMatch(/^【结局A】/);
  });

  it("结局位存档也可读回（ending 落档）", () => {
    const player = new ReplayKernel();
    player.start();
    player.dispatch("ygg.choice_selected", "2");
    player.dispatch("ygg.choice_selected", "2"); // 结局B
    const saveJson = JSON.stringify(player.save().save);

    const restorer = new ReplayKernel();
    restorer.start();
    const reply = restorer.load(saveJson);
    expect(reply.ok).toBe(true);
    expect(reply.pendingChoice ?? null).toBeNull();
    // 结局位再存档：kv.ending = B（存档本体保真）。
    expect(player.save().save?.kv.entries["ending"]).toBe("B");
  });

  it("非 JSON 存档 → 真实内核 VALIDATION 原文回包", () => {
    const kernel = new ReplayKernel();
    kernel.start();
    const reply = kernel.load("not json");
    expect(reply.ok).toBe(false);
    expect(reply.error?.code).toBe("VALIDATION");
    expect(reply.error?.message).toContain("load rejected");
  });

  it("JSON 合法但不匹配已采集进度 → VALIDATION（mock 边界明示）", () => {
    const kernel = new ReplayKernel();
    kernel.start();
    const reply = kernel.load(JSON.stringify({ format_version: 1, engine_version: "0.1.0" }));
    expect(reply.ok).toBe(false);
    expect(reply.error?.code).toBe("VALIDATION");
    expect(reply.error?.message).toContain("不匹配");
  });

  it("存档往返零漂移：save → load → save 逐字段一致", () => {
    const kernel = new ReplayKernel();
    kernel.start();
    kernel.dispatch("ygg.choice_selected", "2");
    const before = kernel.save().save;
    kernel.load(JSON.stringify(before));
    const after = kernel.save().save;
    expect(after).toEqual(before);
  });
});
