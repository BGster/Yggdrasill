// client.ts — /__generate 的浏览器侧流式消费（A2）：POST 剧本描述 → 边到边解析
// SSE 事件（progress / preview / done / error）。
//
// 为什么不用 EventSource：它只支持 GET、不能带 JSON 请求体。所以走 fetch +
// response.body 的逐块读取，分帧复用 pipeline 的 sse.ts（带跨块缓冲与多行 data
// 处理，已被 provider 双协议实测覆盖）——不自己手搓换行切分。
// 凭据红线：provider 凭据只在 vite 进程侧，这里只发剧本文本、只收结构化事件。
import { readSseEvents } from "@ygg/pipeline/sse";
import { parseGenerateEvent, type GenerateEvent } from "./protocol";

/** dev 生成端点（与 vite.plugin.generate.ts 的 GENERATE_ENDPOINT 一致）。 */
export const GENERATE_ENDPOINT = "/__generate";

/** 面板注入面：测试替换 fetchImpl，不触网。 */
export interface RunGenerateDeps {
  fetchImpl?: typeof fetch;
}

/**
 * 跑一次流式生成：每条合法事件回调一次；非法/未知事件静默忽略（不打断流）。
 * 非 2xx（含服务端未配置 LLM 的 503）→ 抛错，文案取响应体 error 字段。
 */
export async function runGenerate(
  story: string,
  onEvent: (event: GenerateEvent) => void,
  deps: RunGenerateDeps = {},
): Promise<void> {
  const doFetch = deps.fetchImpl ?? fetch;
  const response = await doFetch(GENERATE_ENDPOINT, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ story }),
  });
  if (!response.ok || response.body === null) {
    throw new Error(await describeHttpError(response));
  }
  for await (const event of readSseEvents(response.body)) {
    const parsed = parseGenerateEvent(event.data);
    if (parsed !== null) {
      onEvent(parsed);
    }
  }
}

/** 非 2xx 的错误文案：优先用服务端 error 字段（只含键名/状态，不含凭据）。 */
async function describeHttpError(response: Response): Promise<string> {
  try {
    const payload = (await response.json()) as { error?: unknown };
    if (typeof payload.error === "string" && payload.error.length > 0) {
      return payload.error;
    }
  } catch {
    // 响应体不是 JSON / 已消费：退化为状态码文案。
  }
  return `生成端点返回 HTTP ${response.status}`;
}
