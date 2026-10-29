// revise-cli.ts — revise 命令入口（A4）：参数解析 + env 装配 + 退出码映射。
// 退出码与 generate 对齐：0 = 修改成功；1 = 被白名单/门拒绝（磁盘保持原样）；
// 2 = 配置/环境错误（缺 bundle、LLM 环境变量缺失、意见为空）。
// 运行：pnpm -C packages/pipeline run revise -- <slug> "<修改意见>" [--allow-lua]

import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import { loadDotEnv, readLlmEnv, repoRoot } from "./env.js";
import { createProvider } from "./provider.js";
import { revise } from "./revise.js";
import { execRunner } from "./runner.js";
import type { GateIssue, ReviseResult } from "./types.js";

const HELP = [
  '用法: pnpm -C packages/pipeline run revise -- <slug> "<修改意见>" [选项]',
  "  --allow-lua         授权大改（放行 mod.lua 变更与 scenes 六节键集增删；默认都冻结）",
  "  --out <目录>        bundle 根目录（默认 packages/mods/generated）",
  "  --feedback-file <路径>  从文件读修改意见（长文本用）",
  "  -h | --help         本帮助",
  "凭据：仓库根 .env 的 YGG_LLM_BASE_URL / YGG_LLM_API_KEY / YGG_LLM_MODEL / YGG_LLM_PROTOCOL",
].join("\n");

export interface ParsedReviseArgs {
  slug?: string;
  feedback?: string;
  feedbackFile?: string;
  allowLua: boolean;
  outRoot?: string;
}

export function parseReviseArgs(argv: string[]): ParsedReviseArgs | { error: string } {
  let slug: string | undefined;
  let feedback: string | undefined;
  let feedbackFile: string | undefined;
  let allowLua = false;
  let outRoot: string | undefined;
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i]!;
    if (arg === "--") {
      // 裸 `--` = pnpm run 透传的旗末分隔符（与 generate 同处理）。
      continue;
    } else if (arg === "--allow-lua") {
      allowLua = true;
    } else if (arg === "--out") {
      outRoot = argv[++i];
      if (outRoot === undefined) return { error: "--out 需要目录参数" };
    } else if (arg === "--feedback-file") {
      feedbackFile = argv[++i];
      if (feedbackFile === undefined) return { error: "--feedback-file 需要文件参数" };
    } else if (arg === "-h" || arg === "--help") {
      return { error: HELP };
    } else if (arg.startsWith("-")) {
      return { error: `未知参数 ${arg}` };
    } else if (slug === undefined) {
      slug = arg;
    } else if (feedback === undefined) {
      feedback = arg;
    } else {
      return { error: "只接受两个位置参数：<slug> \"<修改意见>\"（长文本用 --feedback-file）" };
    }
  }
  if (slug === undefined) {
    return { error: HELP }; // 无 slug = 用法错误（cli 侧 exit 2）。
  }
  if (feedback !== undefined && feedbackFile !== undefined) {
    return { error: "修改意见与 --feedback-file 二选一" };
  }
  if (feedback === undefined && feedbackFile === undefined) {
    return { error: HELP };
  }
  return { slug, feedback, feedbackFile, allowLua, outRoot };
}

export function formatIssue(issue: GateIssue): string {
  return (
    `- [${issue.gate}/${issue.code}] ${issue.file}` +
    `${issue.line !== undefined ? `:${issue.line}:${issue.column ?? 1}` : ""} ${issue.message}`
  );
}

export async function main(argv: string[]): Promise<number> {
  const parsed = parseReviseArgs(argv);
  if ("error" in parsed) {
    console.error(parsed.error);
    return 2;
  }
  let feedback = parsed.feedback;
  if (parsed.feedbackFile !== undefined) {
    try {
      feedback = readFileSync(parsed.feedbackFile, "utf8").trim();
    } catch (error) {
      console.error(
        `无法读取修改意见文件 ${parsed.feedbackFile}：${error instanceof Error ? error.message : String(error)}`,
      );
      return 2;
    }
  }
  if (feedback === undefined || feedback.length === 0) {
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

  let result: ReviseResult;
  try {
    result = await revise({
      slug: parsed.slug!, // parse 已保证非空
      feedback,
      repoRoot: repoRoot(),
      outRoot: parsed.outRoot,
      allowLua: parsed.allowLua,
      provider,
      runner: execRunner(),
      log: (message) => console.log(message),
    });
  } catch (error) {
    // 缺 bundle / 声明面不符 / 五件套缺件 = 配置面错误（不是门失败）。
    console.error(error instanceof Error ? error.message : String(error));
    return 2;
  }

  const errors = result.issues.filter((issue) => issue.severity === "error");
  for (const warning of result.issues.filter((issue) => issue.severity === "warning")) {
    console.warn(`警告 ${formatIssue(warning)}`);
  }
  if (result.ok) {
    const summary =
      result.changed.length > 0 ? `变更 ${result.changed.length} 处` : "无变更（空 diff）";
    console.log(`revise 成功：${result.outDir}（${summary}）`);
    return 0;
  }
  console.error(`revise 失败（磁盘未改动，现场保持原样）：${errors.length} 条诊断`);
  for (const issue of errors) {
    console.error(formatIssue(issue));
  }
  return 1;
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href) {
  void main(process.argv.slice(2)).then((code) => {
    process.exitCode = code;
  });
}
