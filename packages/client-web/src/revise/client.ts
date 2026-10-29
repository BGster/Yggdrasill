// client.ts — /__revise 的浏览器侧调用（A5）：一条意见 → 一包 JSON 结果。
//
// 与 /__generate 的 client 的差别：这里没有流要读（端点单轮、无 SSE），
// 所以只需 fetch → 校验形状 → 返回联合类型。形状不符一律抛错：面板宁可显示
// 一条「响应形状不符」，也不把半个对象当成修改成功画出来。
//
// 凭据红线：请求体只有 slug / 意见 / allowLua，浏览器侧全程不接触任何密钥。
import {
  parseReviseResponse,
  REVISE_ENDPOINT,
  type ReviseRequest,
  type ReviseResponse,
} from "./protocol";

/** 面板注入面（测试替换 fetchImpl，不触网）。 */
export interface RunReviseDeps {
  fetchImpl?: typeof fetch;
}

/**
 * 跑一轮定向修改。返回 ok:false（白名单/门拒绝）是**正常返回值**，
 * 只有 HTTP 层/形状层问题才抛错（面板据此区分「被拒」与「端点坏了」）。
 */
export async function runRevise(
  request: ReviseRequest,
  deps: RunReviseDeps = {},
): Promise<ReviseResponse> {
  const doFetch = deps.fetchImpl ?? fetch;
  const response = await doFetch(REVISE_ENDPOINT, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(request),
  });
  if (!response.ok) {
    throw new Error(await describeHttpError(response));
  }
  let payload: unknown;
  try {
    payload = (await response.json()) as unknown;
  } catch {
    throw new Error("修改端点返回的不是合法 JSON");
  }
  const parsed = parseReviseResponse(payload);
  if (parsed === null) {
    throw new Error("修改端点响应形状不符（前后端版本不一致？）");
  }
  return parsed;
}

/** 非 2xx 的错误文案：优先用服务端 error 字段（只含键名/诊断，不含凭据）。 */
async function describeHttpError(response: Response): Promise<string> {
  try {
    const payload = (await response.json()) as { error?: unknown };
    if (typeof payload.error === "string" && payload.error.length > 0) {
      return payload.error;
    }
  } catch {
    // 响应体不是 JSON / 已消费：退化为状态码文案。
  }
  return `修改端点返回 HTTP ${response.status}`;
}
