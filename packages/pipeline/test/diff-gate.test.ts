// diff-gate.test.ts — A4 白名单校验的规则表（纯内存，无 fs / 无子进程）。
// 每条用例对应「什么变更放行 / 什么拒绝 / 什么需要 --allow-lua」的一行规则。

import { describe, expect, it } from "vitest";
import { diffStory, type DiffOptions } from "../src/diff-gate.js";
import type { StoryMod } from "../src/types.js";

const BASE_MOD_LUA = `-- 守灯塔人（测试基线）
local T = ygg.data("ygg:scene/texts")
`;

const BASE: StoryMod = {
  slug: "lighthouse-keeper",
  title: "暴风雨夜的灯塔",
  modLua: BASE_MOD_LUA,
  scenes: {
    scenes: { reef: "礁石上。", storm: "暴风雨夜，灯塔的光柱在浪涛上摇晃。" },
    endings: { alone: "【结局守候】你在灯下守到天明。", dawn: "【结局晨光】天亮了。" },
    prompts: { rescue: "向信天翁抛出救生索吗？" },
    options: { rope: "抛出绳索", wait: "守在灯下" },
    beats: { rescue: "你冒雨下到礁石。" },
    tuning: { lampOil: 2 },
  },
};

/** 深拷贝基线（structuredClone：Node 20+，测试数据纯 JSON）。 */
function base(): StoryMod {
  return structuredClone(BASE);
}

/** 读某一节（StoryMod.scenes 是 Record<string, unknown>，测试内收窄）。 */
function section(story: StoryMod, name: string): Record<string, unknown> {
  return (story.scenes[name] ?? {}) as Record<string, unknown>;
}

/** 整体替换某一节（不动基线对象）。 */
function patch(story: StoryMod, name: string, entries: Record<string, unknown>): StoryMod {
  story.scenes = { ...story.scenes, [name]: entries };
  return story;
}

function run(next: StoryMod, options: Partial<DiffOptions> = {}) {
  return diffStory(BASE, next, { allowLua: false, ...options });
}

function codes(issues: Array<{ code: string }>): string[] {
  return issues.map((issue) => issue.code);
}

describe("diff 白名单（A4）：放行面", () => {
  it("只改一个场景文本 → 放行，变更摘要 = 该叶子路径", () => {
    const next = patch(base(), "scenes", {
      ...section(BASE, "scenes"),
      storm: "暴风雨夜，光柱摇晃得更凶。",
    });
    const outcome = run(next);
    expect(codes(outcome.issues)).toEqual([]);
    expect(outcome.changed).toEqual(["scenes.scenes.storm"]);
    expect(outcome.structural).toEqual([]);
  });

  it("多节各改一点 → 摘要按六节固定序 + 键字典序，跨节稳定", () => {
    const next = base();
    patch(next, "endings", { ...section(BASE, "endings"), dawn: "【结局晨光】天光大亮。" });
    patch(next, "scenes", { ...section(BASE, "scenes"), reef: "礁石上多了一只脚印。" });
    patch(next, "tuning", { lampOil: 3 });
    const outcome = run(next);
    expect(codes(outcome.issues)).toEqual([]);
    // 六节序 scenes → endings → … → tuning；节内键字典序。
    expect(outcome.changed).toEqual([
      "scenes.scenes.reef",
      "scenes.endings.dawn",
      "scenes.tuning.lampOil",
    ]);
  });

  it("tuning 数值变更属内容变更（非结构）→ 放行", () => {
    const next = patch(base(), "tuning", { lampOil: 9 });
    const outcome = run(next);
    expect(codes(outcome.issues)).toEqual([]);
    expect(outcome.changed).toEqual(["scenes.tuning.lampOil"]);
  });

  it("空 diff → REVISE_NOOP 警告放行（不是错误）", () => {
    const outcome = run(base());
    expect(codes(outcome.issues)).toEqual(["REVISE_NOOP"]);
    expect(outcome.issues[0]?.severity).toBe("warning");
    expect(outcome.changed).toEqual([]);
  });
});

describe("diff 白名单（A4）：mod.lua 冻结", () => {
  it("改 mod.lua 且未授权 → REVISE_LUA_FROZEN 拒绝", () => {
    const next = base();
    next.modLua = `${BASE_MOD_LUA}-- 偷偷加一行\n`;
    const outcome = run(next);
    expect(codes(outcome.issues)).toEqual(["REVISE_LUA_FROZEN"]);
    expect(outcome.issues[0]?.file).toBe("mod.lua");
    expect(outcome.issues[0]?.severity).toBe("error");
    expect(outcome.changed).toContain("modLua"); // 变更事实如实记录（即使被拒）
  });

  it("--allow-lua 时 mod.lua 变更放行", () => {
    const next = base();
    next.modLua = `${BASE_MOD_LUA}-- 授权改动\n`;
    const outcome = run(next, { allowLua: true });
    expect(codes(outcome.issues)).toEqual([]);
    expect(outcome.changed).toEqual(["modLua"]);
  });
});

describe("diff 白名单（A4）：结构变更（六节键集）", () => {
  it("新增场景键 → REVISE_STRUCTURE 拒绝", () => {
    const next = patch(base(), "scenes", { ...section(BASE, "scenes"), wreck: "沉船里。" });
    const outcome = run(next);
    expect(codes(outcome.issues)).toEqual(["REVISE_STRUCTURE"]);
    expect(outcome.issues[0]?.file).toBe("data/scenes.json");
    expect(outcome.structural).toEqual(["scenes.scenes.wreck"]);
  });

  it("删除场景键 → REVISE_STRUCTURE 拒绝（冻结的 mod.lua 引用会悬空）", () => {
    const { reef: _reef, ...rest } = section(BASE, "scenes");
    const next = patch(base(), "scenes", rest);
    const outcome = run(next);
    expect(codes(outcome.issues)).toEqual(["REVISE_STRUCTURE"]);
    expect(outcome.structural).toEqual(["scenes.scenes.reef"]);
  });

  it("新增结局键 → 同样按结构变更拒绝（六节一律，不只 scenes 节）", () => {
    const next = patch(base(), "endings", {
      ...section(BASE, "endings"),
      storm: "【结局风暴】灯灭了。",
    });
    const outcome = run(next);
    expect(codes(outcome.issues)).toEqual(["REVISE_STRUCTURE"]);
    expect(outcome.structural).toEqual(["scenes.endings.storm"]);
  });

  it("--allow-lua 同时解锁结构变更", () => {
    const next = base();
    patch(next, "scenes", { ...section(BASE, "scenes"), wreck: "沉船里。" });
    patch(next, "endings", { ...section(BASE, "endings"), storm: "【结局风暴】灯灭了。" });
    const outcome = run(next, { allowLua: true });
    expect(codes(outcome.issues)).toEqual([]);
    expect(outcome.structural).toEqual(["scenes.scenes.wreck", "scenes.endings.storm"]);
  });

  it("结构变更与 mod.lua 变更叠加 → 两条错误一起报（聚合面同构）", () => {
    const next = base();
    next.modLua = `${BASE_MOD_LUA}-- 改\n`;
    patch(next, "scenes", { ...section(BASE, "scenes"), wreck: "沉船里。" });
    const outcome = run(next);
    expect(codes(outcome.issues).sort()).toEqual(["REVISE_LUA_FROZEN", "REVISE_STRUCTURE"]);
  });
});

describe("diff 白名单（A4）：身份锚点冻结", () => {
  it("title 被改 → REVISE_IDENTITY 拒绝", () => {
    const next = base();
    next.title = "灯塔的另一夜";
    const outcome = run(next);
    expect(codes(outcome.issues)).toEqual(["REVISE_IDENTITY"]);
    expect(outcome.issues[0]?.message).toContain("title");
    expect(outcome.changed).toEqual(["title"]); // 变更事实如实记录（即使被拒）
  });

  it("slug 被改 → REVISE_IDENTITY 拒绝", () => {
    const next = base();
    next.slug = "lighthouse-keeper-2";
    const outcome = run(next);
    expect(codes(outcome.issues)).toEqual(["REVISE_IDENTITY"]);
    expect(outcome.issues[0]?.message).toContain("slug");
    expect(outcome.changed).toEqual(["slug"]);
  });

  it("title 锚点未知（A1-A3 期产物）→ 跳过 title 校验，且不计入变更摘要", () => {
    const next = base();
    next.title = "锚点未知时采用的标题";
    const outcome = run(next, { titleKnown: false });
    expect(codes(outcome.issues)).toEqual(["REVISE_NOOP"]); // 无错误；仅余「无变更」警告
    expect(outcome.issues[0]?.severity).toBe("warning");
    expect(outcome.changed).toEqual([]); // title 无基线可比 → 不进摘要
  });

  it("锚点未知不影响 slug 冻结（目录名恒可信）", () => {
    const next = base();
    next.slug = "other-slug";
    const outcome = run(next, { titleKnown: false });
    expect(codes(outcome.issues)).toEqual(["REVISE_IDENTITY"]);
  });
});
