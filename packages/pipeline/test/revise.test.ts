// revise.test.ts — A4 编排：磁盘回读 → 定向修改 → diff 白名单 → 三道门 → 覆盖落盘。
// 与 repair.test.ts 同款注入面：mock provider（脚本化 story 序列）+ fake runner
// （ygg-lint / playtest 子进程的确定性替身），单测不真跑 cargo、不碰真 LLM。

import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { repoRoot } from "../src/env.js";
import type { LlmProvider, ProviderReply, StructuredRequest } from "../src/provider.js";
import { generate } from "../src/repair.js";
import { revise } from "../src/revise.js";
import { storyMetaPath } from "../src/story-bundle.js";
import type { CommandRunner, GateIssue, ReviseResult, StoryMod } from "../src/types.js";

/** 基线 story：lint 干净 + playtest 可通关形态（与 repair.test 的 GOOD_STORY 同族）。 */
const BASE_STORY: StoryMod = {
  slug: "revise-tale",
  title: "重试演示",
  modLua: '-- revise demo\nlocal T = ygg.data("ygg:scene/texts")\n',
  scenes: {
    scenes: { begin: "起点。", detour: "岔路。" },
    endings: { a: "【结局甲】终。" },
    prompts: {},
    options: {},
    beats: {},
    tuning: { steps: 3 },
  },
};

const BUNDLE_FILES = [
  "mod.lua",
  "pack.json",
  "manifest.json",
  "data/scenes.json",
  "schemas/scene-texts.json",
] as const;

function clone(story: StoryMod): StoryMod {
  return structuredClone(story);
}

function section(story: StoryMod, name: string): Record<string, unknown> {
  return (story.scenes[name] ?? {}) as Record<string, unknown>;
}

function patch(story: StoryMod, name: string, entries: Record<string, unknown>): StoryMod {
  story.scenes = { ...story.scenes, [name]: entries };
  return story;
}

/** mock provider：按脚本依次返回；记录收到的 user prompt。 */
function scriptedProvider(script: unknown[]): LlmProvider & { prompts: string[] } {
  const prompts: string[] = [];
  let index = 0;
  const complete = async (request: StructuredRequest): Promise<ProviderReply> => {
    prompts.push(request.user);
    const story = script[Math.min(index, script.length - 1)]!;
    index += 1;
    return { story, usage: { inputTokens: 11, outputTokens: 22 } };
  };
  return {
    protocol: "openai-chat",
    prompts,
    complete,
    async completeStream(request, onProgress) {
      const reply = await complete(request);
      onProgress({ text: JSON.stringify(reply.story) });
      return reply;
    },
  };
}

/** fake runner：本切片只走 ygg-lint 与 playtest 两条子进程面。 */
function fakeRunner(options: { playtestOk: boolean; lintJson?: string }): CommandRunner {
  return {
    async run(command, args) {
      const joined = [command, ...args].join(" ");
      if (joined.includes("ygg-lint")) {
        const diagnostics = options.lintJson ?? "[]";
        const hasError = (JSON.parse(diagnostics) as GateIssue[]).some(
          (issue) => issue.severity === "error",
        );
        return { exitCode: hasError ? 1 : 0, stdout: diagnostics, stderr: "" };
      }
      if (joined.includes("playtest")) {
        return {
          exitCode: options.playtestOk ? 0 : 1,
          stdout: JSON.stringify({
            ok: options.playtestOk,
            mod: "tmp",
            steps: 3,
            reached: options.playtestOk ? "ending" : "deadend",
            narrativeSample: ["序章"],
            warnings: [],
            eventsProcessed: 5,
            errors: options.playtestOk ? [] : ["走查未达验收锚（reached=deadend）"],
          }),
          stderr: "",
        };
      }
      throw new Error(`fakeRunner 未识别的命令：${joined}`);
    },
  };
}

interface Sandbox {
  outRoot: string;
  dir: string;
  cleanup: () => void;
}

/** 用 generate 真实落盘一个基线 bundle（顺带验证 title 锚点确实被写）。 */
async function seedBundle(): Promise<Sandbox> {
  const outRoot = mkdtempSync(join(tmpdir(), "ygg-revise-test-"));
  const result = await generate({
    story: "一个小故事",
    maxRetries: 0,
    repoRoot: repoRoot(),
    outRoot,
    provider: scriptedProvider([BASE_STORY]),
    runner: fakeRunner({ playtestOk: true }),
    log: () => {},
  });
  expect(result.ok).toBe(true);
  return {
    outRoot,
    dir: join(outRoot, BASE_STORY.slug),
    cleanup: () => rmSync(outRoot, { recursive: true, force: true }),
  };
}

/** bundle 五件套快照（判断「磁盘是否一字不动」）。 */
function snapshot(dir: string): Record<string, string> {
  const files: Record<string, string> = {};
  for (const relative of BUNDLE_FILES) {
    files[relative] = readFileSync(join(dir, relative), "utf8");
  }
  return files;
}

async function runRevise(
  sandbox: Sandbox,
  next: unknown,
  options: { allowLua?: boolean } = {},
): Promise<{ result: ReviseResult; prompts: string[] }> {
  const provider = scriptedProvider([next]);
  const result = await revise({
    slug: BASE_STORY.slug,
    feedback: "把起点写得更阴冷一些",
    repoRoot: repoRoot(),
    outRoot: sandbox.outRoot,
    allowLua: options.allowLua,
    provider,
    runner: fakeRunner({ playtestOk: true }),
    log: () => {},
  });
  return { result, prompts: provider.prompts };
}

describe("revise（A4：定向修改 + diff 白名单 + 三道门）", () => {
  it("正常修改（只改一个场景文本）→ 通过，变更摘要正确，磁盘按新文本覆盖", async () => {
    const sandbox = await seedBundle();
    try {
      const next = patch(clone(BASE_STORY), "scenes", {
        ...section(BASE_STORY, "scenes"),
        begin: "起点是一片阴冷。",
      });
      const { result, prompts } = await runRevise(sandbox, next);

      expect(result.ok).toBe(true);
      expect(result.changed).toEqual(["scenes.scenes.begin"]);
      expect(result.issues).toEqual([]); // 非空 diff → 无 NOOP 警告
      expect(result.outDir).toBe(sandbox.dir);
      expect(result.tokenUsage).toEqual({ inputTokens: 11, outputTokens: 22 });

      const written = JSON.parse(
        readFileSync(join(sandbox.dir, "data/scenes.json"), "utf8"),
      ) as Record<string, Record<string, string>>;
      expect(written.scenes?.begin).toBe("起点是一片阴冷。");
      expect(written.scenes?.detour).toBe("岔路。"); // 未涉及的键逐字保留

      // prompt 面：只带现有 story（不放 few-shot 样本）+ 意见 + 冻结项。
      expect(prompts[0]).toContain("修改意见");
      expect(prompts[0]).toContain("岔路。"); // 现有基线全文在 prompt 里（冻结基准）
      expect(prompts[0]).toContain("冻结项");
      expect(prompts[0]).not.toContain("packages/mods/test-adventure"); // 无 few-shot
    } finally {
      sandbox.cleanup();
    }
  });

  it("LLM 改了 mod.lua 且未 allowLua → REVISE_LUA_FROZEN，磁盘一字不动", async () => {
    const sandbox = await seedBundle();
    try {
      const before = snapshot(sandbox.dir);
      const next = clone(BASE_STORY);
      next.modLua = `${BASE_STORY.modLua}-- 越权加行\n`;
      const { result } = await runRevise(sandbox, next);

      expect(result.ok).toBe(false);
      expect(result.outDir).toBe("");
      expect(result.issues[0]).toMatchObject({ gate: "revise", code: "REVISE_LUA_FROZEN" });
      expect(snapshot(sandbox.dir)).toEqual(before);
    } finally {
      sandbox.cleanup();
    }
  });

  it("allowLua 时 mod.lua 变更放行，覆盖落盘", async () => {
    const sandbox = await seedBundle();
    try {
      const next = clone(BASE_STORY);
      next.modLua = `${BASE_STORY.modLua}-- 授权加行\n`;
      const { result, prompts } = await runRevise(sandbox, next, { allowLua: true });

      expect(result.ok).toBe(true);
      expect(result.changed).toEqual(["modLua"]);
      expect(readFileSync(join(sandbox.dir, "mod.lua"), "utf8")).toContain("授权加行");
      expect(prompts[0]).toContain("已授权大改"); // 提示面同步解锁口径
    } finally {
      sandbox.cleanup();
    }
  });

  it("新增场景键（结构变更）未 allowLua → REVISE_STRUCTURE；allowLua 放行", async () => {
    const sandbox = await seedBundle();
    try {
      const before = snapshot(sandbox.dir);
      const next = patch(clone(BASE_STORY), "scenes", {
        ...section(BASE_STORY, "scenes"),
        wreck: "沉船。",
      });

      const rejected = await runRevise(sandbox, next);
      expect(rejected.result.ok).toBe(false);
      expect(rejected.result.issues[0]).toMatchObject({ code: "REVISE_STRUCTURE" });
      expect(snapshot(sandbox.dir)).toEqual(before);

      const allowed = await runRevise(sandbox, next, { allowLua: true });
      expect(allowed.result.ok).toBe(true);
      expect(allowed.result.changed).toEqual(["scenes.scenes.wreck"]);
      const written = JSON.parse(
        readFileSync(join(sandbox.dir, "data/scenes.json"), "utf8"),
      ) as Record<string, Record<string, string>>;
      expect(written.scenes?.wreck).toBe("沉船。");
    } finally {
      sandbox.cleanup();
    }
  });

  it("title 被改 → REVISE_IDENTITY 拒绝（title 锚点由 generate 写入）", async () => {
    const sandbox = await seedBundle();
    try {
      const before = snapshot(sandbox.dir);
      const next = clone(BASE_STORY);
      next.title = "换了个标题";
      const { result } = await runRevise(sandbox, next);

      expect(result.ok).toBe(false);
      expect(result.issues[0]).toMatchObject({ code: "REVISE_IDENTITY" });
      expect(snapshot(sandbox.dir)).toEqual(before);
      // 锚点仍在（拒绝路径不刷新）。
      expect(existsSync(storyMetaPath(sandbox.outRoot, BASE_STORY.slug))).toBe(true);
    } finally {
      sandbox.cleanup();
    }
  });

  it("无 title 锚点（A1-A3 期产物）→ 跳过 title 冻结，成功后补写锚点", async () => {
    const sandbox = await seedBundle();
    try {
      rmSync(join(sandbox.outRoot, ".meta"), { recursive: true, force: true });
      const next = clone(BASE_STORY);
      next.title = "锚点缺失时采用的新标题";
      next.scenes = { ...next.scenes, tuning: { steps: 4 } };

      const { result } = await runRevise(sandbox, next);
      expect(result.ok).toBe(true);
      expect(result.changed).toEqual(["scenes.tuning.steps"]); // title 不计入（无法判定）
      const meta = JSON.parse(
        readFileSync(storyMetaPath(sandbox.outRoot, BASE_STORY.slug), "utf8"),
      ) as { title: string };
      expect(meta.title).toBe("锚点缺失时采用的新标题");
    } finally {
      sandbox.cleanup();
    }
  });

  it("空 diff → 放行 + REVISE_NOOP 警告（磁盘内容不变）", async () => {
    const sandbox = await seedBundle();
    try {
      const { result } = await runRevise(sandbox, clone(BASE_STORY));

      expect(result.ok).toBe(true);
      expect(result.changed).toEqual([]);
      expect(result.issues).toHaveLength(1);
      expect(result.issues[0]).toMatchObject({
        gate: "revise",
        code: "REVISE_NOOP",
        severity: "warning",
      });
    } finally {
      sandbox.cleanup();
    }
  });

  it("story 契约失败 → 不进入 diff/门，磁盘一字不动", async () => {
    const sandbox = await seedBundle();
    try {
      const before = snapshot(sandbox.dir);
      const { result } = await runRevise(sandbox, { ...clone(BASE_STORY), slug: "Invalid Slug!" });

      expect(result.ok).toBe(false);
      expect(result.issues[0]).toMatchObject({ gate: "schema" });
      expect(snapshot(sandbox.dir)).toEqual(before);
    } finally {
      sandbox.cleanup();
    }
  });

  it("三道门复用：playtest 门失败 → 不落盘（现场保持原样）", async () => {
    const sandbox = await seedBundle();
    try {
      const before = snapshot(sandbox.dir);
      const next = patch(clone(BASE_STORY), "scenes", {
        ...section(BASE_STORY, "scenes"),
        begin: "起点改为死路。",
      });
      const provider = scriptedProvider([next]);
      const result = await revise({
        slug: BASE_STORY.slug,
        feedback: "把起点改成死路",
        repoRoot: repoRoot(),
        outRoot: sandbox.outRoot,
        provider,
        runner: fakeRunner({ playtestOk: false }),
        log: () => {},
      });

      expect(result.ok).toBe(false);
      expect(result.outDir).toBe("");
      expect(result.issues[0]).toMatchObject({ gate: "playtest", code: "PLAYTEST" });
      expect(snapshot(sandbox.dir)).toEqual(before);
    } finally {
      sandbox.cleanup();
    }
  });

  it("lint 门失败 → 不落盘，诊断透传 YGG-R* 码", async () => {
    const sandbox = await seedBundle();
    try {
      const before = snapshot(sandbox.dir);
      const next = patch(clone(BASE_STORY), "scenes", {
        ...section(BASE_STORY, "scenes"),
        begin: "起点。",
        detour: "岔路二。",
      });
      const provider = scriptedProvider([next]);
      const result = await revise({
        slug: BASE_STORY.slug,
        feedback: "改岔路",
        repoRoot: repoRoot(),
        outRoot: sandbox.outRoot,
        provider,
        runner: fakeRunner({
          playtestOk: true,
          lintJson: JSON.stringify([
            {
              file: "mod.lua",
              line: 2,
              column: 1,
              code: "YGG-R1",
              severity: "error",
              message: "emit before choice",
            },
          ]),
        }),
        log: () => {},
      });

      expect(result.ok).toBe(false);
      expect(result.issues[0]).toMatchObject({ gate: "lint", code: "YGG-R1", line: 2 });
      expect(snapshot(sandbox.dir)).toEqual(before);
    } finally {
      sandbox.cleanup();
    }
  });

  it("bundle 不存在 → 抛可读错误（配置面，不是门失败）", async () => {
    const outRoot = mkdtempSync(join(tmpdir(), "ygg-revise-missing-"));
    try {
      await expect(
        revise({
          slug: "no-such-mod",
          feedback: "随便改改",
          repoRoot: repoRoot(),
          outRoot,
          provider: scriptedProvider([BASE_STORY]),
          runner: fakeRunner({ playtestOk: true }),
          log: () => {},
        }),
      ).rejects.toThrow(/找不到要修改的 Mod bundle/);
    } finally {
      rmSync(outRoot, { recursive: true, force: true });
    }
  });

  it("pack.json 声明面不符（非管线产物）→ 抛可读错误", async () => {
    const outRoot = mkdtempSync(join(tmpdir(), "ygg-revise-foreign-"));
    try {
      const dir = join(outRoot, "foreign-mod");
      rmSync(dir, { recursive: true, force: true });
      const { mkdirSync } = await import("node:fs");
      mkdirSync(join(dir, "data"), { recursive: true });
      writeFileSync(join(dir, "pack.json"), JSON.stringify({ schemaVersion: 1, entries: [] }));
      await expect(
        revise({
          slug: "foreign-mod",
          feedback: "随便改改",
          repoRoot: repoRoot(),
          outRoot,
          provider: scriptedProvider([BASE_STORY]),
          runner: fakeRunner({ playtestOk: true }),
          log: () => {},
        }),
      ).rejects.toThrow(/不是管线生成的 Mod bundle/);
    } finally {
      rmSync(outRoot, { recursive: true, force: true });
    }
  });
});
