// cli.ts — generate 命令入口（FR-1）：参数解析 + env 装配 + 退出码映射。
// 退出码：0 = 生成成功；1 = 重试额度耗尽（最后产物已保留）；2 = 配置/环境
// 错误。运行：pnpm -C packages/pipeline run generate -- "<一句话剧本>"
// A2：透传 onProgress 显示流式进度行（TTY 单行 \r 刷新，非 TTY 千 token 步进）。

import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import { loadDotEnv, readLlmEnv, repoRoot } from "./env.js";
import { createProvider } from "./provider.js";
import { DEFAULT_MAX_RETRIES, generate } from "./repair.js";
import { execRunner } from "./runner.js";
import type { GenerateProgress, GenerateResult } from "./types.js";

// 实现移入 runner.ts（dev 生成端点复用同一实现）；此处保留原有导出面。
export { execRunner };

const HELP = [
  "用法: pnpm -C packages/pipeline run generate -- \"<剧本描述>\" [选项]",
  "  --max-retries <N>   重试上限（0..10，默认 3；总轮数 = N + 1）",
  "  --out <目录>        生成根目录（默认 packages/mods/generated）",
  "  --story-file <路径> 从文件读剧本描述（长文本用）",
  "  -h | --help         本帮助",
  "凭据：仓库根 .env 的 YGG_LLM_BASE_URL / YGG_LLM_API_KEY / YGG_LLM_MODEL / YGG_LLM_PROTOCOL",
].join("\n");

interface ParsedArgs {
  story?: string;
  storyFile?: string;
  maxRetries: number;
  outRoot?: string;
}

export function parseArgs(argv: string[]): ParsedArgs | { error: string } {
  let story: string | undefined;
  let storyFile: string | undefined;
  let maxRetries = DEFAULT_MAX_RETRIES;
  let outRoot: string | undefined;
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i]!;
    if (arg === "--") {
      // 裸 `--` = 旗末分隔符（getopt 惯例）：pnpm run generate -- "剧本" 会把
      // `--` 原样透传给脚本，跳过之（Slice 6 Desired End State 命令形态）。
      continue;
    } else if (arg === "--max-retries") {
      const value = Number(argv[++i]);
      if (!Number.isInteger(value) || value < 0 || value > 10) {
        return { error: "--max-retries 需要 0..10 的整数" };
      }
      maxRetries = value;
    } else if (arg === "--out") {
      outRoot = argv[++i];
      if (outRoot === undefined) return { error: "--out 需要目录参数" };
    } else if (arg === "--story-file") {
      storyFile = argv[++i];
      if (storyFile === undefined) return { error: "--story-file 需要文件参数" };
    } else if (arg === "-h" || arg === "--help") {
      return { error: HELP };
    } else if (arg.startsWith("-")) {
      return { error: `未知参数 ${arg}` };
    } else if (story === undefined) {
      story = arg;
    } else {
      return { error: "剧本描述只接受一个位置参数（长文本用 --story-file）" };
    }
  }
  if (story !== undefined && storyFile !== undefined) {
    return { error: "剧本描述与 --story-file 二选一" };
  }
  if (story === undefined && storyFile === undefined) {
    return { error: HELP }; // 无故事输入 = 用法错误（cli.test.ts「无参数」用例锚）。
  }
  return { story, storyFile, maxRetries, outRoot };
}

/** 进度行文案：累计粗估 token + 已解析出的场景/结局数（A2 形态）。 */
export function formatProgress(progress: GenerateProgress): string {
  const scenes = progress.preview.scenes.length;
  const endings = progress.preview.endings.length;
  const suffix = endings > 0 ? ` · 结局 ×${endings}` : "";
  return `  已收到 ≈${progress.tokens.toLocaleString("en-US")} tokens · 场景 ×${scenes}${suffix}`;
}

/**
 * 生成进度行（A2）：TTY 用 `\r` 覆写同一行（终端干净）；非 TTY（重定向为日志）
 * 每跳 1000 token 追加一行——重定向场景下 `\r` 只会变成不可读的粘行。
 */
export class ProgressLine {
  private width = 0;
  private lastStep = -1;
  private active = false;

  constructor(
    private readonly write: (text: string) => void,
    private readonly isTty: boolean,
  ) {}

  update(progress: GenerateProgress): void {
    const line = formatProgress(progress);
    if (this.isTty) {
      this.write(`\r${line.padEnd(this.width, " ")}`);
      this.width = line.length;
      this.active = true;
      return;
    }
    const step = Math.floor(progress.tokens / 1000);
    if (step > this.lastStep) {
      this.lastStep = step;
      this.write(`[progress] ${line.trim()}\n`);
    }
  }

  /** 清除瞬态行（打日志 / 收尾前调用），保证后续输出不粘连。 */
  clear(): void {
    if (!this.active) {
      return;
    }
    this.write(`\r${" ".repeat(this.width)}\r`);
    this.width = 0;
    this.active = false;
  }
}

export async function main(argv: string[]): Promise<number> {
  const parsed = parseArgs(argv);
  if ("error" in parsed) {
    console.error(parsed.error);
    return 2;
  }
  let story = parsed.story;
  if (parsed.storyFile !== undefined) {
    try {
      story = readFileSync(parsed.storyFile, "utf8").trim();
    } catch (error) {
      console.error(`无法读取剧本文件 ${parsed.storyFile}：${error instanceof Error ? error.message : String(error)}`);
      return 2;
    }
  }
  if (story === undefined || story.length === 0) {
    console.error(HELP);
    return 2;
  }

  loadDotEnv();
  const env = readLlmEnv();
  if ("error" in env) {
    console.error(env.error);
    return 2;
  }
  const provider = createProvider(env.protocol, {
    baseUrl: env.baseUrl,
    apiKey: env.apiKey,
    model: env.model,
  });
  const progressLine = new ProgressLine(
    (text) => process.stdout.write(text),
    process.stdout.isTTY === true,
  );
  let result: GenerateResult;
  try {
    result = await generate({
      story,
      maxRetries: parsed.maxRetries,
      repoRoot: repoRoot(),
      outRoot: parsed.outRoot,
      provider,
      runner: execRunner(),
      log: (message) => {
        progressLine.clear(); // 日志行前先擦掉瞬态进度行（不清会粘连成乱码）
        console.log(message);
      },
      onProgress: (progress) => progressLine.update(progress),
    });
  } finally {
    progressLine.clear();
  }
  if (result.ok) {
    console.log(`生成成功：${result.outDir}（round ${result.attempts}）`);
    return 0;
  }
  console.error(
    result.outDir !== ""
      ? `生成失败（重试额度耗尽，最后产物保留于 ${result.outDir}）：`
      : "生成失败（重试额度耗尽：最后一轮未过 story 契约校验，无产物落盘）：",
  );
  for (const issue of result.issues) {
    console.error(
      `- [${issue.gate}/${issue.code}] ${issue.file}` +
        `${issue.line !== undefined ? `:${issue.line}:${issue.column ?? 1}` : ""} ${issue.message}`,
    );
  }
  return 1;
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href) {
  void main(process.argv.slice(2)).then((code) => {
    process.exitCode = code;
  });
}
