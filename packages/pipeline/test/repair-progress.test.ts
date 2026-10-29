// repair-progress.test.ts — generate 流式进度面（A2）契约测：
// ①给了 onProgress 就走 completeStream（provider.complete 不再被调用）；
// ②不给 onProgress 时与旧行为逐位一致（只走 complete）——向后兼容的锚；
// ③进度单调（累计字符只增不减）④收尾分片（只载用量）也回调，故终态预览完整；
// ⑤重试轮次 round 递增、累计口径跨轮。
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { repoRoot } from "../src/env.js";
import type {
  LlmProvider,
  ProgressHandler,
  ProviderReply,
  StructuredRequest,
} from "../src/provider.js";
import { generate } from "../src/repair.js";
import type { CommandRunner, GateIssue, GenerateProgress } from "../src/types.js";

const GOOD_STORY = {
  slug: "progress-tale",
  title: "进度演示",
  modLua: "-- demo mod",
  scenes: {
    scenes: { begin: "起点文案", detour: "岔路文案" },
    endings: { a: "【结局】终。" },
    prompts: {},
    options: {},
    beats: {},
    tuning: {},
  },
};

const BAD_STORY = { ...GOOD_STORY, slug: "Invalid Slug!" };

/** mock provider：流式按 40 字符切片，收尾再补一枚「只载用量」的空文本分片。 */
function streamingProvider(stories: unknown[]): {
  provider: LlmProvider;
  calls: { complete: number; completeStream: number };
} {
  const calls = { complete: 0, completeStream: 0 };
  let index = 0;
  const next = () => {
    const story = stories[Math.min(index, stories.length - 1)];
    index += 1;
    return story;
  };
  const provider: LlmProvider = {
    protocol: "openai-chat",
    async complete(_request: StructuredRequest): Promise<ProviderReply> {
      calls.complete += 1;
      return { story: next(), usage: { inputTokens: 10, outputTokens: 20 } };
    },
    async completeStream(
      _request: StructuredRequest,
      onProgress: ProgressHandler,
    ): Promise<ProviderReply> {
      calls.completeStream += 1;
      const story = next();
      const text = JSON.stringify(story, null, 2);
      for (let i = 0; i < text.length; i += 40) {
        onProgress({ text: text.slice(i, i + 40) });
      }
      onProgress({ text: "", inputTokens: 10, outputTokens: 20 }); // 收尾分片
      return { story, usage: { inputTokens: 10, outputTokens: 20 } };
    },
  };
  return { provider, calls };
}

/** fake runner：按命令行形态区分两道门（lint → ygg-lint，playtest → example）。 */
function gateRunner(options: { playtestOk: boolean; lintIssues?: GateIssue[] }): CommandRunner {
  return {
    async run(command, args) {
      const joined = [command, ...args].join(" ");
      if (joined.includes("ygg-lint")) {
        const issues = options.lintIssues ?? [];
        return { exitCode: issues.length > 0 ? 1 : 0, stdout: JSON.stringify(issues), stderr: "" };
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
      throw new Error(`gateRunner 未识别的命令：${joined}`);
    },
  };
}

function makeOptions(provider: LlmProvider, runner: CommandRunner) {
  const outRoot = mkdtempSync(join(tmpdir(), "ygg-progress-test-"));
  return {
    options: {
      story: "一个小故事",
      maxRetries: 1,
      repoRoot: repoRoot(), // 真实仓库根：generate 启动期读 packages/schema 单源 schema。
      outRoot,
      provider,
      runner,
      log: () => {},
    },
    cleanup: () => rmSync(outRoot, { recursive: true, force: true }),
  };
}

describe("generate 流式进度（A2）", () => {
  it("给了 onProgress → 走 completeStream（complete 不被调用），进度单调且终态预览完整", async () => {
    const { provider, calls } = streamingProvider([GOOD_STORY]);
    const { options, cleanup } = makeOptions(provider, gateRunner({ playtestOk: true }));
    const progress: GenerateProgress[] = [];
    try {
      const result = await generate({ ...options, onProgress: (item) => progress.push(item) });
      expect(calls.completeStream).toBe(1);
      expect(calls.complete).toBe(0);
      expect(result.ok).toBe(true);

      expect(progress.length).toBeGreaterThan(1);
      for (let i = 1; i < progress.length; i++) {
        expect(progress[i].chars).toBeGreaterThanOrEqual(progress[i - 1].chars);
        expect(progress[i].tokens).toBeGreaterThanOrEqual(progress[i - 1].tokens);
      }
      expect(progress.every((item) => item.round === 1)).toBe(true);
      // 收尾分片（text 空、只载用量）也回调 → 终态拿到的就是完整 story 的预览。
      const last = progress[progress.length - 1];
      expect(last.preview.title).toBe("进度演示");
      expect(last.preview.scenes).toEqual([
        { id: "begin", text: "起点文案" },
        { id: "detour", text: "岔路文案" },
      ]);
      expect(last.preview.endings).toEqual([{ id: "a", text: "【结局】终。" }]);
      // 最终 story 文本的完整字符数 == 累计 chars（分片无丢失）。
      expect(last.chars).toBe(JSON.stringify(GOOD_STORY, null, 2).length);
    } finally {
      cleanup();
    }
  });

  it("未给 onProgress → 与旧行为一致（只走 complete；completeStream 不调用）", async () => {
    const { provider, calls } = streamingProvider([GOOD_STORY]);
    const { options, cleanup } = makeOptions(provider, gateRunner({ playtestOk: true }));
    try {
      const result = await generate(options);
      expect(result.ok).toBe(true);
      expect(calls.complete).toBe(1);
      expect(calls.completeStream).toBe(0);
    } finally {
      cleanup();
    }
  });

  it("重试轮次：round 递增，chars 跨轮累加（累计口径 = 本次 generate 全程）", async () => {
    const { provider } = streamingProvider([BAD_STORY, GOOD_STORY]);
    const { options, cleanup } = makeOptions(provider, gateRunner({ playtestOk: true }));
    const progress: GenerateProgress[] = [];
    try {
      const result = await generate({ ...options, onProgress: (item) => progress.push(item) });
      expect(result.ok).toBe(true);
      expect(result.attempts).toBe(2);

      const rounds = new Set(progress.map((item) => item.round));
      expect([...rounds]).toEqual([1, 2]);
      const firstRoundLast = progress.filter((item) => item.round === 1).at(-1);
      const secondRoundFirst = progress.filter((item) => item.round === 2)[0];
      // 第二轮起点承接第一轮累计（不是重新计数）。
      expect(secondRoundFirst.chars).toBeGreaterThan(firstRoundLast?.chars ?? 0);
      // 第二轮预览是重试产物的（slug 合法那版），不是失败那版。
      const last = progress[progress.length - 1];
      expect(last.preview.scenes.map((scene) => scene.id)).toEqual(["begin", "detour"]);
    } finally {
      cleanup();
    }
  });

  it("门失败型耗尽：进度仍全部到位（不影响 gate 语义与返回值）", async () => {
    const { provider } = streamingProvider([GOOD_STORY]);
    const { options, cleanup } = makeOptions(provider, gateRunner({ playtestOk: false }));
    const progress: GenerateProgress[] = [];
    try {
      const result = await generate({ ...options, onProgress: (item) => progress.push(item) });
      expect(result.ok).toBe(false);
      expect(result.issues[0]).toMatchObject({ gate: "playtest", code: "PLAYTEST" });
      // maxRetries 1 → 两轮都流过，进度覆盖两轮。
      expect(new Set(progress.map((item) => item.round)).size).toBe(2);
    } finally {
      cleanup();
    }
  });
});
