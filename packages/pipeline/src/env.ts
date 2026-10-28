// env.ts — .env 读取（仓库根）+ LLM 四键装配。凭据红线（FR-11）：全管线只
// 允许出现键名；值只经内存流向 fetch 头，禁止 console/log/产物落值。

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import type { LlmProtocol } from "./provider.js";

/** 仓库根（src/ 上三级；本包落位 packages/pipeline）。 */
export function repoRoot(): string {
  return fileURLToPath(new URL("../../..", import.meta.url));
}

/** 解析 .env 文本（KEY=VALUE；# 注释；支持成对引号）。 */
export function parseDotEnv(text: string): Record<string, string> {
  const result: Record<string, string> = {};
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (line === "" || line.startsWith("#")) {
      continue;
    }
    const eq = line.indexOf("=");
    if (eq <= 0) {
      continue;
    }
    const key = line.slice(0, eq).trim();
    let value = line.slice(eq + 1).trim();
    if (
      (value.startsWith('"') && value.endsWith('"')) ||
      (value.startsWith("'") && value.endsWith("'"))
    ) {
      value = value.slice(1, -1);
    }
    if (key.length > 0 && !(key in process.env)) {
      result[key] = value;
    }
  }
  return result;
}

/** 装载仓库根 .env（缺文件静默——CI 用真环境变量；不覆盖已有 env）。 */
export function loadDotEnv(): void {
  try {
    const text = readFileSync(`${repoRoot()}/.env`, "utf8");
    for (const [key, value] of Object.entries(parseDotEnv(text))) {
      process.env[key] ??= value;
    }
  } catch {
    // 无 .env = 只依赖真环境变量（键名清单见 packages/pipeline/.env.example）。
  }
}

export interface LlmEnv {
  baseUrl: string;
  apiKey: string;
  model: string;
  protocol: LlmProtocol;
}

function isProtocol(value: string): value is LlmProtocol {
  return value === "openai-chat" || value === "anthropic-messages";
}

/** 读 LLM 四键；缺失/非法只报键名，绝不回显值（FR-11 红线）。 */
export function readLlmEnv(
  env: NodeJS.ProcessEnv = process.env,
): LlmEnv | { error: string } {
  const missing: string[] = [];
  const baseUrl = env.YGG_LLM_BASE_URL ?? "";
  const apiKey = env.YGG_LLM_API_KEY ?? "";
  const model = env.YGG_LLM_MODEL ?? "";
  const rawProtocol = env.YGG_LLM_PROTOCOL ?? "openai-chat";
  if (!baseUrl) missing.push("YGG_LLM_BASE_URL");
  if (!apiKey) missing.push("YGG_LLM_API_KEY");
  if (!model) missing.push("YGG_LLM_MODEL");
  if (!isProtocol(rawProtocol)) {
    return { error: "YGG_LLM_PROTOCOL 必须是 openai-chat 或 anthropic-messages" };
  }
  if (missing.length > 0) {
    return {
      error: `缺少 LLM 环境变量：${missing.join(" / ")}（复制 packages/pipeline/.env.example 到仓库根 .env 填写）`,
    };
  }
  return { baseUrl, apiKey, model, protocol: rawProtocol };
}
