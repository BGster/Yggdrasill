// revise-cli.test.ts — revise 参数面（A4）：位置参数 + 旗标 + 互斥/缺参拒绝。
// main() 需要真 provider/runner/磁盘 bundle，故只测可纯函数化的解析面与文案面。

import { describe, expect, it } from "vitest";
import { formatIssue, parseReviseArgs } from "../src/revise-cli.js";

describe("revise cli 参数面（A4）", () => {
  it("slug + 意见 + 选项", () => {
    const parsed = parseReviseArgs(["my-mod", "改冷一点", "--allow-lua", "--out", "/tmp/x"]);
    expect(parsed).toMatchObject({
      slug: "my-mod",
      feedback: "改冷一点",
      allowLua: true,
      outRoot: "/tmp/x",
    });
  });

  it("裸 -- 旗末分隔符被跳过（pnpm run revise -- <slug> \"<意见>\" 透传形态）", () => {
    const parsed = parseReviseArgs(["--", "my-mod", "改冷一点"]);
    expect(parsed).toMatchObject({ slug: "my-mod", feedback: "改冷一点" });
    expect(parsed).toMatchObject({ allowLua: false });
  });

  it("--feedback-file 与位置意见互斥", () => {
    expect(parseReviseArgs(["my-mod", "意见", "--feedback-file", "a.txt"])).toHaveProperty("error");
  });

  it("缺 slug / 缺意见 → 帮助文本错误", () => {
    expect(parseReviseArgs([])).toHaveProperty("error");
    expect(parseReviseArgs(["my-mod"])).toHaveProperty("error");
  });

  it("第三个位置参数拒绝", () => {
    expect(parseReviseArgs(["my-mod", "意见", "多余的"])).toHaveProperty("error");
  });

  it("未知旗标拒绝；-h 给帮助", () => {
    expect(parseReviseArgs(["my-mod", "意见", "--nope"])).toHaveProperty("error");
    const help = parseReviseArgs(["-h"]);
    expect(help).toHaveProperty("error");
    expect((help as { error: string }).error).toContain("--allow-lua");
  });

  it("formatIssue 带行号时折进 file 位置", () => {
    const text = formatIssue({
      gate: "revise",
      file: "mod.lua",
      line: 3,
      column: 2,
      code: "REVISE_LUA_FROZEN",
      severity: "error",
      message: "冻结",
    });
    expect(text).toBe("- [revise/REVISE_LUA_FROZEN] mod.lua:3:2 冻结");
  });
});
