// cli.ts — generate 命令入口（FR-1）：参数解析 + env 装配 + 退出码映射。
// 退出码：0 = 生成成功；1 = 重试额度耗尽（最后产物已保留）；2 = 配置/环境
// 错误。运行：pnpm -C packages/pipeline run generate -- "<一句话剧本>"

import { execFile } from "node:child_process";
import { readFileSync } from "node:fs";
import { promisify } from "node:util";
import { pathToFileURL } from "node:url";
import { loadDotEnv, readLlmEnv, repoRoot } from "./env.js";
import { createProvider } from "./provider.js";
import { DEFAULT_MAX_RETRIES, generate } from "./repair.js";
import type { CommandRunner } from "./types.js";

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

/** 真实子进程 runner（playtest/lint 门共用；非零退出 = 门结果，ENOENT = 环境错误）。 */
export function execRunner(): CommandRunner {
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
  const result = await generate({
    story,
    maxRetries: parsed.maxRetries,
    repoRoot: repoRoot(),
    outRoot: parsed.outRoot,
    provider,
    runner: execRunner(),
    log: (message) => console.log(message),
  });
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
