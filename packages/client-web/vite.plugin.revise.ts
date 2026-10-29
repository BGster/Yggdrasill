// vite.plugin.revise.ts — 「AI 修改」dev 端点（POST /__revise，A5）。
//
// 背景：/__generate 能从零生成一个作品，但没有「对已有作品提一条意见再改」的
// 浏览器入口（A4 只交付了 CLI）。本插件把 pipeline 的 revise 编排接到 dev server：
// 请求体 { slug, feedback, allowLua } → 复用 A4 的白名单 + 三道门 → 覆盖同 slug
// 落盘 → 回一包 JSON。
//
// 为什么不是 SSE：revise 单轮（改的是几十 KB 既有文本，没有 token 级进度可言），
// 唯一的长耗时是 lint/playtest 两道子进程门。为此铺一套流式协议换不来信息量，
// 面板只显示「修改中…（三道门校验需数秒）」。若将来 revise 变成多轮，再升级。
//
// 边界（与 /__generate 同款）：
// - apply: "serve"：仅 dev server 挂载；生产构建不加载（生产无此面）。
// - 凭据只在 vite 进程：响应里只有 slug / 变更路径 / 门诊断；错误文案过 redact()。
// - slug 白名单（SLUG_PATTERN）：它直接拼进 packages/mods/generated/<slug>，
//   必须杜绝路径穿越；与 /__generated-mod 的白名单同源。
// - 落盘语义由 pipeline revise 保证：任何一段失败 → 磁盘一字不动。

import type { IncomingMessage, ServerResponse } from "node:http";
import { fileURLToPath } from "node:url";
import type { Plugin } from "vite";
import { revise } from "../pipeline/src/revise.js";
import { execRunner } from "../pipeline/src/runner.js";
import { describeError, loadLlmProvider, readJsonBody, sendJson } from "./vite.dev-endpoint";
import {
  parseReviseRequest,
  REVISE_ENDPOINT,
  toReviseResponse,
} from "./src/revise/protocol";

/** 仓库根（本文件在 packages/client-web/ 下）。 */
const REPO_ROOT = fileURLToPath(new URL("../..", import.meta.url));

/** dev-only 修改端点插件（vite.config.ts plugins 挂载）。 */
export function reviseMiddleware(): Plugin {
  return {
    name: "ygg-revise-middleware",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use(REVISE_ENDPOINT, (request, response) => {
        void handleRevise(request, response, (message) =>
          server.config.logger.info(`[revise] ${message}`),
        );
      });
    },
  };
}

async function handleRevise(
  request: IncomingMessage,
  response: ServerResponse,
  log: (message: string) => void,
): Promise<void> {
  if (request.method !== "POST") {
    sendJson(response, 405, { error: "只接受 POST" });
    return;
  }

  let parsed: ReturnType<typeof parseReviseRequest>;
  try {
    parsed = parseReviseRequest(await readJsonBody(request));
  } catch (error) {
    sendJson(response, 400, {
      error: `请求体不可用：${error instanceof Error ? error.message : String(error)}`,
    });
    return;
  }
  if (!parsed.ok) {
    sendJson(response, parsed.status, { error: parsed.error });
    return;
  }

  const llm = loadLlmProvider();
  if ("error" in llm) {
    // 缺配置给干净的 HTTP 错误（面板显示可操作提示），而不是一包空诊断。
    sendJson(response, 503, { error: llm.error });
    return;
  }
  const { provider, redact } = llm;

  try {
    const result = await revise({
      slug: parsed.request.slug,
      feedback: parsed.request.feedback,
      allowLua: parsed.request.allowLua,
      repoRoot: REPO_ROOT,
      provider,
      runner: execRunner(),
      log: (message) => log(redact(message)),
    });
    // ok:false（白名单/门拒绝）也是**正常业务结果**：200 + 结构化诊断，
    // 让面板能按诊断码给「勾选允许修改 Lua 后重试」这类可操作提示。
    sendJson(response, 200, toReviseResponse(result));
  } catch (error) {
    // 缺 bundle / 五件套缺件 / provider 上游错误：配置面错误，非门失败。
    sendJson(response, 400, { error: describeError(redact, error) });
  }
}
