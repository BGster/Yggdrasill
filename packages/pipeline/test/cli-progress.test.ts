// cli-progress.test.ts — CLI 进度行（A2）纯渲染面：TTY 单行覆写 / 非 TTY 千 token
// 步进追加 / 清行不留残影。渲染抽成类是为了让「终端干净」这条可测（不进真 TTY）。
import { describe, expect, it } from "vitest";
import { formatProgress, ProgressLine } from "../src/cli.js";
import type { GenerateProgress } from "../src/types.js";

function progress(tokens: number, scenes: number, endings = 0, round = 1): GenerateProgress {
  return {
    round,
    chars: tokens * 4,
    tokens,
    preview: {
      title: null,
      scenes: Array.from({ length: scenes }, (_unused, i) => ({ id: `s${i}`, text: `文案 ${i}` })),
      endings: Array.from({ length: endings }, (_unused, i) => ({ id: `e${i}`, text: `结局 ${i}` })),
    },
  };
}

function collector(): { write: (text: string) => void; output: () => string } {
  let buffer = "";
  return {
    write: (text) => {
      buffer += text;
    },
    output: () => buffer,
  };
}

describe("formatProgress（进度行文案）", () => {
  it("千分位 token + 场景数；无结局时不显示结局段", () => {
    expect(formatProgress(progress(8432, 3))).toBe("  已收到 ≈8,432 tokens · 场景 ×3");
  });

  it("有结局时追加结局段", () => {
    expect(formatProgress(progress(1200, 2, 4))).toBe(
      "  已收到 ≈1,200 tokens · 场景 ×2 · 结局 ×4",
    );
  });
});

describe("ProgressLine — TTY 单行覆写", () => {
  it("同一行 \\r 覆写；变短时用空格补齐（不留残影）", () => {
    const sink = collector();
    const line = new ProgressLine(sink.write, true);
    const wide = formatProgress(progress(12_345, 3));
    const narrow = formatProgress(progress(999, 1));
    line.update(progress(12_345, 3));
    line.update(progress(999, 1));
    expect(sink.output()).toBe(`\r${wide}\r${narrow.padEnd(wide.length, " ")}`);
    expect(narrow.length).toBeLessThan(wide.length); // 第二条确实更短，补白才有意义
  });

  it("clear 擦掉瞬态行（后续日志从行首开始）", () => {
    const sink = collector();
    const line = new ProgressLine(sink.write, true);
    line.update(progress(1000, 1));
    const before = sink.output().length;
    line.clear();
    const cleared = sink.output().slice(before);
    expect(cleared.startsWith("\r")).toBe(true);
    expect(cleared.endsWith("\r")).toBe(true);
    expect(cleared.replace(/\s/g, "")).toBe(""); // 全是空格，无内容残留
    // 幂等：没有活动行时 clear 不写任何东西
    const afterFirstClear = sink.output();
    line.clear();
    expect(sink.output()).toBe(afterFirstClear);
  });
});

describe("ProgressLine — 非 TTY 千 token 步进", () => {
  it("每跨 1000 token 追加一行（重定向日志里不出现 \\r 粘行）", () => {
    const sink = collector();
    const line = new ProgressLine(sink.write, false);
    line.update(progress(120, 0));
    line.update(progress(900, 1));
    line.update(progress(1000, 1));
    line.update(progress(1001, 2));
    line.update(progress(2500, 3));
    const lines = sink.output().trimEnd().split("\n");
    expect(lines).toEqual([
      "[progress] 已收到 ≈120 tokens · 场景 ×0",
      "[progress] 已收到 ≈1,000 tokens · 场景 ×1",
      "[progress] 已收到 ≈2,500 tokens · 场景 ×3",
    ]);
    expect(sink.output()).not.toContain("\r");
  });

  it("非 TTY 下 clear 是空操作（追加行无需擦除）", () => {
    const sink = collector();
    const line = new ProgressLine(sink.write, false);
    line.update(progress(1000, 0));
    const before = sink.output();
    line.clear();
    expect(sink.output()).toBe(before);
  });
});
