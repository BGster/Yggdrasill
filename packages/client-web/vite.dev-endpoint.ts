// vite.dev-endpoint.ts — dev 端点的共享底座（A5 抽出，/__generate 与 /__revise 共用）。
//
// 抽出的理由不是省几行，而是把两条**红线**收敛成单点实现：
// - 请求体读取（大小上限 + 非法 JSON 归一成可读错误），
// - 凭据边界（LLM 四键只在 vite 进程侧装配；redact 是唯一的外发文案出口）。
// 两份拷贝各写一次 redact，迟早会漂移出一个「某条路径忘了脱敏」的缺口。
//
// 本文件只被 vite 配置侧（apply: "serve" 的插件）引用：不进浏览器产物，
// 也不参与 vitest（mode === "test" 下 jsdom 环境里没有 node:http 请求对象）。

import type { IncomingMessage, ServerResponse } from "node:http";
import { loadDotEnv, readLlmEnv } from "../pipeline/src/env.js";
import type { LlmProvider } from "../pipeline/src/provider.js";
import { createProvider } from "../pipeline/src/provider.js";
import type { GateIssue } from "../pipeline/src/types.js";
import type { GenerateIssueView } from "./src/generate/protocol";

/** 请求体上限：剧本/意见都是短文本，1MB 纯粹是「误粘大文件」的拦截网。 */
export const MAX_BODY_BYTES = 1024 * 1024;

/** 读 JSON 请求体（空体 → null；超限/非法 JSON → 抛带可展示文案的错误）。 */
export async function readJsonBody(request: IncomingMessage): Promise<unknown> {
  const chunks: Buffer[] = [];
  let size = 0;
  for await (const chunk of request) {
    const buffer = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk as string);
    size += buffer.length;
    if (size > MAX_BODY_BYTES) {
      throw new Error("请求体过大");
    }
    chunks.push(buffer);
  }
  const text = Buffer.concat(chunks).toString("utf8");
  if (text.length === 0) {
    return null;
  }
  try {
    return JSON.parse(text) as unknown;
  } catch {
    throw new Error("请求体不是合法 JSON");
  }
}

/** 回一包 JSON（端点错误面统一走这里，保证 content-type 与 UTF-8 一致）。 */
export function sendJson(response: ServerResponse, status: number, body: unknown): void {
  response.statusCode = status;
  response.setHeader("content-type", "application/json; charset=utf-8");
  response.end(JSON.stringify(body));
}

export interface LlmAssembly {
  provider: LlmProvider;
  /** 任何要外发/落日志的文本都必须过这里（apiKey 原值绝不出现）。 */
  redact: (text: string) => string;
}

/**
 * 装配 dev 侧的 LLM provider：.env → 四键校验 → provider → redact。
 * 失败只回「缺哪些键名」的可操作文案（readLlmEnv 契约），绝不回显值。
 */
export function loadLlmProvider(): LlmAssembly | { error: string } {
  loadDotEnv();
  const env = readLlmEnv();
  if ("error" in env) {
    return { error: env.error };
  }
  const redact = (text: string): string =>
    env.apiKey.length > 0 ? text.split(env.apiKey).join("[redacted]") : text;
  return {
    redact,
    provider: createProvider(env.protocol, {
      baseUrl: env.baseUrl,
      apiKey: env.apiKey,
      model: env.model,
    }),
  };
}

/** 门错误 → wire 投影（只留可展示字段；行号折进 file，丢掉 hint/severity）。 */
export function toIssueView(issue: GateIssue): GenerateIssueView {
  const location = issue.line !== undefined ? `${issue.file}:${issue.line}` : issue.file;
  return { gate: issue.gate, code: issue.code, file: location, message: issue.message };
}

/** 异常 → 可展示文案（未自证的上游文案也要过 redact）。 */
export function describeError(redact: (text: string) => string, error: unknown): string {
  return redact(error instanceof Error ? error.message : String(error));
}
