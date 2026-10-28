import { existsSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import type { LlmProvider, ProviderReply, StructuredRequest } from "../src/provider.js";
import { generate } from "../src/repair.js";
import type { CommandRunner, GateIssue } from "../src/types.js";
import { repoRoot } from "../src/env.js";

const GOOD_STORY = {
  slug: "repair-tale",
  title: "重试演示",
  modLua: "-- demo mod",
  scenes: {
    scenes: { begin: "起点" },
    endings: { a: "【结局】终。" },
    prompts: {},
    options: {},
    beats: {},
    tuning: {},
  },
};

const BAD_STORY = { ...GOOD_STORY, slug: "Invalid Slug!" };

/** mock provider：按脚本依次返回；记录收到的 user prompt。 */
function scriptedProvider(script: unknown[]): LlmProvider & { prompts: string[] } {
  const prompts: string[] = [];
  let index = 0;
  return {
    protocol: "openai-chat",
    prompts,
    async complete(request: StructuredRequest): Promise<ProviderReply> {
      prompts.push(request.user);
      const story = script[Math.min(index, script.length - 1)]!;
      index += 1;
      return { story, usage: { inputTokens: 10, outputTokens: 20 } };
    },
  };
}

/** fake runner：ygg-lint 与 playtest 子进程的确定性替身（门契约形态）。 */
function fakeRunner(options: { lintExit?: number; lintJson?: string; playtestOk: boolean }): CommandRunner {
  return {
    async run(command, args) {
      const joined = [command, ...args].join(" ");
      if (joined.includes("ygg-lint")) {
        return {
          exitCode: options.lintExit ?? (options.lintJson && JSON.parse(options.lintJson).some((d: GateIssue) => d.severity === "error") ? 1 : 0),
          stdout: options.lintJson ?? "[]",
          stderr: "",
        };
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

function makeOptions(provider: LlmProvider, runner: CommandRunner) {
  const outRoot = mkdtempSync(join(tmpdir(), "ygg-repair-test-"));
  return {
    options: {
      story: "一个小故事",
      maxRetries: 2,
      repoRoot: repoRoot(), // 真实仓库根：generate 启动期读 packages/schema 单源 schema（fence 原 repoRoot: "/" 会 ENOENT，implement 修正）。
      outRoot,
      provider,
      runner,
      log: () => {},
    },
    outRoot,
    cleanup: () => rmSync(outRoot, { recursive: true, force: true }),
  };
}

describe("repair（FR-1/FR-7：三道门 + 有界重生成）", () => {
  it("首轮全绿：schema → lint → playtest 通过，五件套落盘，attempts=1", async () => {
    const provider = scriptedProvider([GOOD_STORY]);
    const { options, cleanup } = makeOptions(provider, fakeRunner({ playtestOk: true }));
    try {
      const result = await generate(options);
      expect(result.ok).toBe(true);
      expect(result.attempts).toBe(1);
      for (const file of ["mod.lua", "pack.json", "manifest.json", "data/scenes.json", "schemas/scene-texts.json"]) {
        expect(existsSync(join(result.outDir, file))).toBe(true);
      }
      expect(result.tokenUsage).toEqual({ inputTokens: 10, outputTokens: 20 });
    } finally {
      cleanup();
    }
  });

  it("story 契约失败 → 修复轮 prompt 携带错误清单，第二轮成功", async () => {
    const provider = scriptedProvider([BAD_STORY, GOOD_STORY]);
    const { options, cleanup } = makeOptions(provider, fakeRunner({ playtestOk: true }));
    try {
      const result = await generate(options);
      expect(result.ok).toBe(true);
      expect(result.attempts).toBe(2);
      expect(provider.prompts[1]).toContain("验收门错误清单");
      expect(provider.prompts[1]).toContain("上一轮产出");
    } finally {
      cleanup();
    }
  });

  it("额度耗尽（门失败型）：ok=false + attempts=maxRetries+1 + 最后产物现场保留（FR-7 × D-15）", async () => {
    const provider = scriptedProvider([GOOD_STORY]); // 契约过审、playtest 门持续失败。
    const { options, cleanup } = makeOptions(provider, fakeRunner({ playtestOk: false }));
    try {
      const result = await generate(options);
      expect(result.ok).toBe(false);
      expect(result.attempts).toBe(3); // maxRetries 2 + 首轮。
      expect(result.issues.length).toBeGreaterThan(0);
      expect(existsSync(join(result.outDir, "mod.lua"))).toBe(true);
    } finally {
      cleanup();
    }
  });

  it("额度耗尽（契约失败型）：最后一轮未过 story 契约 → 不落盘，只返回 issues（D-15：outDir 完整五件套或不存在）", async () => {
    const provider = scriptedProvider([BAD_STORY]);
    const { options, outRoot, cleanup } = makeOptions(provider, fakeRunner({ playtestOk: true }));
    try {
      const result = await generate(options);
      expect(result.ok).toBe(false);
      expect(result.attempts).toBe(3);
      expect(result.issues[0]).toMatchObject({ gate: "schema" });
      expect(result.outDir).toBe("");
      expect(readdirSync(outRoot)).toEqual([]); // 无任何产物落盘。
    } finally {
      cleanup();
    }
  });

  it("playtest 门失败（deadend）→ issue 带 PLAYTEST 码反喂", async () => {
    const provider = scriptedProvider([GOOD_STORY]);
    const { options, cleanup } = makeOptions(provider, fakeRunner({ playtestOk: false }));
    try {
      const result = await generate(options);
      expect(result.ok).toBe(false);
      expect(result.issues[0]).toMatchObject({ gate: "playtest", code: "PLAYTEST" });
    } finally {
      cleanup();
    }
  });

  it("lint 门失败（YGG-R1）→ issue 透传 ygg-lint 诊断", async () => {
    const provider = scriptedProvider([GOOD_STORY]);
    const lintJson = JSON.stringify([
      {
        file: "mod.lua",
        line: 3,
        column: 3,
        code: "YGG-R1",
        severity: "error",
        message: "emit before choice",
        fix: "move to choice_selected",
      },
    ]);
    const { options, cleanup } = makeOptions(provider, fakeRunner({ lintJson, playtestOk: true }));
    try {
      const result = await generate(options);
      expect(result.ok).toBe(false);
      expect(result.issues[0]).toMatchObject({ gate: "lint", code: "YGG-R1", line: 3 });
      // lint 门失败时 playtest 门不运行（序位：前两道静态廉价）。
      expect(provider.prompts).toHaveLength(3); // 1 + maxRetries 2。
    } finally {
      cleanup();
    }
  });
});
