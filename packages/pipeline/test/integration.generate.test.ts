// integration.generate.test.ts — M1 闭环集成验收：mock provider（确定性
// story）+ 真实三道门子进程（ajv 实校 / cargo run ygg-lint / cargo run
// playtest）。真实 LLM 不在环——provider 契约由 mock 承载，story 质量门由
// 真实工具链承担。单独脚本：pnpm -C packages/pipeline run test:integration
// （需 Rust 工具链；首个 cargo run 会触发构建，testTimeout 300s）。

import { execFile } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { describe, expect, it } from "vitest";
import { repoRoot } from "../src/env.js";
import type { LlmProvider, ProviderReply, StructuredRequest } from "../src/provider.js";
import { generate } from "../src/repair.js";
import type { CommandRunner } from "../src/types.js";

/** 确定性 mini story：lint 干净（括号 goto / 无 emit-before-choice / kv 句柄）
 *  + playtest 可通关（choice 1 → 【结局行）。 */
const INTEGRATION_MOD_LUA = `-- integration tale（mock provider 产物）
local T = ygg.data("ygg:scene/texts")
ygg.narrative.node("node:begin", function()
  ygg.narrate(T.scenes.begin)
  ygg.kv.set("stage", "begin")
  ygg.choice(T.prompts.door, { T.options.enter, T.options.leave })
end)
ygg.on("ygg.choice_selected", function(index)
  if index == 1 then
    ygg.narrate(T.endings.brave)
  else
    ygg.narrate(T.endings.cautious)
  end
end)
ygg.on("event.start", function()
  ygg.narrative["goto"]("node:begin")
end)
`;

const INTEGRATION_STORY = {
  slug: "integration-tale",
  title: "集成验收小故事",
  modLua: INTEGRATION_MOD_LUA,
  scenes: {
    scenes: { begin: "山门前有一条小路。" },
    endings: {
      brave: "【结局X】你推门而入，故事开始。",
      cautious: "【结局Y】你转身离开，山门在身后合拢。",
    },
    prompts: { door: "推开山门吗？" },
    options: { enter: "推门", leave: "离开" },
    beats: {},
    tuning: {},
  },
};

function mockProvider(): LlmProvider {
  return {
    protocol: "openai-chat",
    async complete(request: StructuredRequest): Promise<ProviderReply> {
      void request;
      return { story: INTEGRATION_STORY, usage: { inputTokens: 1, outputTokens: 1 } };
    },
  };
}

function execRunner(): CommandRunner {
  const run = promisify(execFile);
  return {
    async run(command, args, options) {
      try {
        const { stdout, stderr } = await run(command, args, {
          cwd: options.cwd,
          maxBuffer: 16 * 1024 * 1024,
        });
        return { exitCode: 0, stdout, stderr };
      } catch (error) {
        const err = error as { code?: number | string; stdout?: string; stderr?: string };
        if (typeof err.code === "number") {
          return { exitCode: err.code, stdout: err.stdout ?? "", stderr: err.stderr ?? "" };
        }
        throw error;
      }
    },
  };
}

describe("M1 闭环集成（mock provider + 真实三道门）", () => {
  it("generate 全绿：五件套落盘 + playtest reached=ending", async () => {
    const outRoot = mkdtempSync(join(tmpdir(), "ygg-integration-"));
    try {
      const result = await generate({
        story: "一个推门抉择的小故事（集成验收用）",
        maxRetries: 0,
        repoRoot: repoRoot(),
        outRoot,
        provider: mockProvider(),
        runner: execRunner(),
        log: (message) => console.log(message),
      });
      expect(result.ok).toBe(true);
      expect(result.attempts).toBe(1);
      for (const file of ["mod.lua", "pack.json", "manifest.json", "data/scenes.json", "schemas/scene-texts.json"]) {
        expect(existsSync(join(result.outDir, file))).toBe(true);
      }
      // 生成物与手写 Mod 同构的直接证据：manifest 与 scenes.json 字节自洽。
      const manifest = JSON.parse(readFileSync(join(result.outDir, "manifest.json"), "utf8")) as {
        entries: Array<{ path: string; byteLength: number }>;
      };
      const scenesBytes = readFileSync(join(result.outDir, "data/scenes.json"));
      expect(manifest.entries[0]!.byteLength).toBe(scenesBytes.byteLength);
    } finally {
      rmSync(outRoot, { recursive: true, force: true });
    }
  });
});
