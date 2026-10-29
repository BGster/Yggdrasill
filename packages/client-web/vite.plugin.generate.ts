// vite.plugin.generate.ts — 「AI 生成」dev 端点（POST /__generate，A2/A3）。
//
// 背景：client-web 原本只有「装载已生成 Mod」的入口（?mod=generated/<slug>），
// 没有生成入口。本插件在 dev 侧补一条生成通道：请求体 { story } → 服务端装配
// env/provider（复用 pipeline 的 loadDotEnv / readLlmEnv / createProvider /
// generate），把流式进度与文本预览以 SSE 推给面板。
//
// 边界（硬约束）：
// - apply: "serve"：仅 dev server 挂载；生产构建不加载（生产无此面）。
// - 凭据只在 vite 进程侧：事件里只有 token 计数、标题/场景文案、slug、门错误；
//   绝不出现 baseUrl / apiKey / 模型名，也不回显 LLM 响应体。错误文案统一过
//   redact()（万一上游把密钥拼进 message），日志同理。
// - 产物落盘走 pipeline 既有路径（packages/mods/generated/<slug>/），随后由
//   /__generated-mod/<slug>（vite.plugin.mods.ts）供 mod-loader 装载。
//
// 事件序列：progress* preview* → done | error。wire 契约（src/generate/protocol.ts）
// 由服务端 type-only 引用、客户端运行时解析——同一联合类型，双侧共用一份形状。
//
// 模块解析：pipeline 源码按**相对路径** import：vite 配置打包时相对路径内联
// （esbuild 会把 ./x.js 解析到 x.ts），bare import（ajv）由 vite 按 importer 位置
// 改写成绝对 file URL——所以 ajv 从 packages/pipeline 自己的 node_modules 解析，
// 不需要 client-web 声明该依赖。

import type { IncomingMessage, ServerResponse } from "node:http";
import { fileURLToPath } from "node:url";
import type { Plugin } from "vite";
import { loadDotEnv, readLlmEnv } from "../pipeline/src/env.js";
import { createProvider } from "../pipeline/src/provider.js";
import { generate } from "../pipeline/src/repair.js";
import { execRunner } from "../pipeline/src/runner.js";
import type { GateIssue, GenerateProgress } from "../pipeline/src/types.js";
import type { GenerateEvent, GenerateIssueView } from "./src/generate/protocol";

export const GENERATE_ENDPOINT = "/__generate";

/** 剧本描述上限：prompt 预算有限，超长描述没有收益（也挡住误粘大文件）。 */
export const MAX_STORY_CHARS = 4000;

/** 进度/预览推送节流（毫秒）：每分片一发会淹没面板渲染。 */
const EMIT_INTERVAL_MS = 120;

/** 仓库根（本文件在 packages/client-web/ 下）。 */
const REPO_ROOT = fileURLToPath(new URL("../..", import.meta.url));

/** 请求体读取（上限 1MB；超限即拒）。 */
async function readJsonBody(request: IncomingMessage): Promise<unknown> {
  const chunks: Buffer[] = [];
  let size = 0;
  for await (const chunk of request) {
    const buffer = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk as string);
    size += buffer.length;
    if (size > 1024 * 1024) {
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

/** SSE 输出面：客户端断开后静默丢弃（不让写入异常冒到 connect）。 */
function openSse(response: ServerResponse, isClientGone: () => boolean): {
  send: (event: GenerateEvent) => void;
  close: () => void;
} {
  response.writeHead(200, {
    "content-type": "text/event-stream; charset=utf-8",
    "cache-control": "no-cache, no-transform",
    connection: "keep-alive",
    "x-accel-buffering": "no", // 让 nginx 类中间层直通（本地 dev 也无害）
  });
  let ended = false;
  return {
    send(event) {
      if (ended || isClientGone() || response.writableEnded) {
        return;
      }
      response.write(`data: ${JSON.stringify(event)}\n\n`);
    },
    close() {
      if (ended) {
        return;
      }
      ended = true;
      if (!response.writableEnded) {
        response.end();
      }
    },
  };
}

/** 门错误 → wire 投影（只留可展示字段；行号折进 file，丢掉 hint/severity）。 */
function toIssueView(issue: GateIssue): GenerateIssueView {
  const location = issue.line !== undefined ? `${issue.file}:${issue.line}` : issue.file;
  return { gate: issue.gate, code: issue.code, file: location, message: issue.message };
}

/** dev-only 生成端点插件（vite.config.ts plugins 挂载）。 */
export function generateMiddleware(): Plugin {
  return {
    name: "ygg-generate-middleware",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use(GENERATE_ENDPOINT, (request, response) => {
        void handleGenerate(request, response, (message) =>
          server.config.logger.info(`[generate] ${message}`),
        );
      });
    },
  };
}

async function handleGenerate(
  request: IncomingMessage,
  response: ServerResponse,
  log: (message: string) => void,
): Promise<void> {
  const sendJson = (status: number, body: unknown): void => {
    response.statusCode = status;
    response.setHeader("content-type", "application/json; charset=utf-8");
    response.end(JSON.stringify(body));
  };

  if (request.method !== "POST") {
    sendJson(405, { error: "只接受 POST" });
    return;
  }

  let story: string;
  try {
    const body = await readJsonBody(request);
    const candidate = (body as { story?: unknown } | null)?.story;
    if (typeof candidate !== "string" || candidate.trim().length === 0) {
      sendJson(400, { error: "缺少 story（剧本描述）" });
      return;
    }
    if (candidate.length > MAX_STORY_CHARS) {
      sendJson(413, { error: `剧本描述过长（上限 ${MAX_STORY_CHARS} 字）` });
      return;
    }
    story = candidate.trim();
  } catch (error) {
    sendJson(400, {
      error: `请求体不可用：${error instanceof Error ? error.message : String(error)}`,
    });
    return;
  }

  // env 装配：只报键名（readLlmEnv 的契约），缺配置时给干净的 HTTP 错误而不是
  // 一条空 SSE 流——面板据此显示可操作的提示。
  loadDotEnv();
  const env = readLlmEnv();
  if ("error" in env) {
    sendJson(503, { error: env.error });
    return;
  }

  // 凭据红线兜底：任何要外发的文本都不许带 apiKey 原值。
  const redact = (text: string): string =>
    env.apiKey.length > 0 ? text.split(env.apiKey).join("[redacted]") : text;

  const provider = createProvider(env.protocol, {
    baseUrl: env.baseUrl,
    apiKey: env.apiKey,
    model: env.model,
  });

  let clientGone = false;
  response.on("close", () => {
    if (!response.writableEnded) {
      clientGone = true; // 客户端 abort：后续 send 直接丢弃
    }
  });
  const sink = openSse(response, () => clientGone);

  let latest: GenerateProgress | null = null;
  let lastEmitAt = 0;
  let lastPreviewKey = "";

  /** 节流推送：progress 按时间窗；preview 在标题/条目数变化时立刻推。 */
  const emit = (progress: GenerateProgress, force: boolean): void => {
    const now = Date.now();
    const previewKey = [
      progress.preview.title ?? "",
      progress.preview.scenes.length,
      progress.preview.endings.length,
    ].join("|");
    const progressDue = force || now - lastEmitAt >= EMIT_INTERVAL_MS;
    if (progressDue) {
      lastEmitAt = now;
      sink.send({
        type: "progress",
        round: progress.round,
        tokens: progress.tokens,
        scenes: progress.preview.scenes.length,
        endings: progress.preview.endings.length,
      });
    }
    if (force || previewKey !== lastPreviewKey) {
      lastPreviewKey = previewKey;
      sink.send({
        type: "preview",
        title: progress.preview.title,
        scenes: progress.preview.scenes,
        endings: progress.preview.endings,
      });
    }
  };

  try {
    const result = await generate({
      story,
      repoRoot: REPO_ROOT,
      provider,
      runner: execRunner(),
      log: (message) => log(redact(message)),
      onProgress: (progress) => {
        latest = progress;
        emit(progress, false);
      },
    });
    if (latest !== null) {
      emit(latest, true); // 尾量补齐：节流可能吞掉最后一次进度/预览
    }
    if (result.ok) {
      sink.send({ type: "done", slug: result.slug });
    } else {
      sink.send({
        type: "error",
        message:
          result.outDir !== ""
            ? `重试额度耗尽（最后产物保留在 packages/mods/generated/${result.slug}）`
            : "重试额度耗尽：最后一轮未过 story 契约校验，无产物落盘",
        issues: result.issues.map(toIssueView),
      });
    }
  } catch (error) {
    // provider 错误自带「只含状态码」的契约；redact 只兜底不自证的上游文案。
    sink.send({
      type: "error",
      message: redact(error instanceof Error ? error.message : String(error)),
      issues: [],
    });
  } finally {
    sink.close();
  }
}
