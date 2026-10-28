import { describe, expect, it } from "vitest";
import { parseArgs } from "../src/cli.js";
import { parseDotEnv, readLlmEnv } from "../src/env.js";

describe("cli 参数面（FR-1）", () => {
  it("位置参数 + 选项", () => {
    const parsed = parseArgs(["一个故事", "--max-retries", "1", "--out", "/tmp/x"]);
    expect(parsed).toMatchObject({ story: "一个故事", maxRetries: 1, outRoot: "/tmp/x" });
  });

  it("裸 -- 旗末分隔符被跳过（pnpm run generate -- \"剧本\" 透传形态）", () => {
    const parsed = parseArgs(["--", "一个故事"]);
    expect(parsed).toMatchObject({ story: "一个故事" });
    expect(parseArgs(["--", "--max-retries", "0", "另一个故事"])).toMatchObject({
      story: "另一个故事",
      maxRetries: 0,
    });
  });

  it("--story-file 与位置参数互斥", () => {
    expect(parseArgs(["故事", "--story-file", "a.txt"])).toHaveProperty("error");
  });

  it("max-retries 越界拒绝", () => {
    expect(parseArgs(["故事", "--max-retries", "99"])).toHaveProperty("error");
  });

  it("无参数 → 帮助文本错误", () => {
    expect(parseArgs([])).toHaveProperty("error");
  });
});

describe("env（FR-11 凭据红线）", () => {
  it("parseDotEnv 解析键值与注释", () => {
    const parsed = parseDotEnv('# 注释\nA=1\nB="two words"\n\nC=3');
    expect(parsed).toEqual({ A: "1", B: "two words", C: "3" });
  });

  it("readLlmEnv 缺键只报键名，绝不回显值", () => {
    const result = readLlmEnv({ YGG_LLM_API_KEY: "secret-value" });
    if (!("error" in result)) {
      throw new Error("应当报缺失");
    }
    expect(result.error).toContain("YGG_LLM_BASE_URL");
    expect(result.error).toContain("YGG_LLM_MODEL");
    expect(result.error).not.toContain("secret-value");
  });

  it("readLlmEnv 非法 protocol 拒绝", () => {
    expect(
      readLlmEnv({
        YGG_LLM_BASE_URL: "x",
        YGG_LLM_API_KEY: "k",
        YGG_LLM_MODEL: "m",
        YGG_LLM_PROTOCOL: "grpc",
      }),
    ).toHaveProperty("error");
  });
});
