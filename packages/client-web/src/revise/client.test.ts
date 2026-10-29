// client.test.ts — /__revise 的浏览器侧调用：请求体形状（slug / 意见 / allowLua）、
// ok:false 当正常返回值、HTTP 与形状层错误抛错。
import { describe, expect, it } from "vitest";
import { runRevise } from "./client";

interface Captured {
  url: string;
  body: unknown;
}

function jsonFetch(payload: unknown, status = 200, calls: Captured[] = []): typeof fetch {
  return (async (url: string, init?: RequestInit) => {
    calls.push({ url, body: JSON.parse(String(init?.body ?? "null")) as unknown });
    return new Response(JSON.stringify(payload), { status });
  }) as unknown as typeof fetch;
}

async function captureError(run: () => Promise<unknown>): Promise<Error> {
  try {
    await run();
  } catch (error) {
    return error as Error;
  }
  throw new Error("预期抛错但没有抛");
}

describe("runRevise", () => {
  it("POST 到 /__revise，请求体三键原样；成功包原样返回", async () => {
    const calls: Captured[] = [];
    const response = await runRevise(
      { slug: "lighthouse-keeper", feedback: "风雨写急一点", allowLua: true },
      {
        fetchImpl: jsonFetch(
          { ok: true, slug: "lighthouse-keeper", changes: ["scenes.scenes.storm"], noop: false },
          200,
          calls,
        ),
      },
    );
    expect(calls).toEqual([
      {
        url: "/__revise",
        body: { slug: "lighthouse-keeper", feedback: "风雨写急一点", allowLua: true },
      },
    ]);
    expect(response).toEqual({
      ok: true,
      slug: "lighthouse-keeper",
      changes: ["scenes.scenes.storm"],
      noop: false,
    });
  });

  it("ok:false（白名单/门拒绝）是正常返回值，不抛错", async () => {
    const response = await runRevise(
      { slug: "x1", feedback: "改 Lua", allowLua: false },
      {
        fetchImpl: jsonFetch({
          ok: false,
          issues: [{ gate: "revise", code: "REVISE_LUA_FROZEN", file: "mod.lua", message: "冻结" }],
        }),
      },
    );
    expect(response.ok).toBe(false);
    expect(response.ok === false && response.issues[0]?.code).toBe("REVISE_LUA_FROZEN");
  });

  it("非 2xx + JSON error → 抛服务端文案（缺 bundle / 缺环境变量）", async () => {
    const error = await captureError(() =>
      runRevise(
        { slug: "missing", feedback: "改", allowLua: false },
        { fetchImpl: jsonFetch({ error: "找不到要修改的 Mod bundle：/x/missing" }, 400) },
      ),
    );
    expect(error.message).toContain("找不到要修改的 Mod bundle");
  });

  it("非 2xx 且响应体不是 JSON → 退化为状态码文案", async () => {
    const fetchImpl = (async () =>
      new Response("<html>502</html>", { status: 502 })) as unknown as typeof fetch;
    const error = await captureError(() =>
      runRevise({ slug: "x1", feedback: "改", allowLua: false }, { fetchImpl }),
    );
    expect(error.message).toContain("502");
  });

  it("2xx 但不是 JSON / 形状不符 → 明确抛错（不把半个对象当成功）", async () => {
    const htmlFetch = (async () =>
      new Response("<html>ok</html>", { status: 200 })) as unknown as typeof fetch;
    expect(
      (await captureError(() => runRevise({ slug: "x1", feedback: "改", allowLua: false }, { fetchImpl: htmlFetch })))
        .message,
    ).toContain("不是合法 JSON");

    const wrongShape = await captureError(() =>
      runRevise(
        { slug: "x1", feedback: "改", allowLua: false },
        { fetchImpl: jsonFetch({ ok: true, slug: "x1" }) }, // 缺 changes / noop
      ),
    );
    expect(wrongShape.message).toContain("响应形状不符");
  });
});
