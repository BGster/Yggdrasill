// protocol.test.ts — /__revise 的 wire 契约：请求校验（slug 白名单 / 意见非空与
// 长度 / allowLua 只认字面 true）、结果投影（ok/noop/诊断过滤/行号折叠）、
// 客户端形状解析（漂移即 null）。
import { describe, expect, it } from "vitest";
import {
  isValidSlug,
  MAX_FEEDBACK_CHARS,
  parseReviseRequest,
  parseReviseResponse,
  toReviseResponse,
} from "./protocol";

describe("isValidSlug（路径安全边界）", () => {
  it("接受产物 slug 形态；拒绝路径穿越与大写", () => {
    expect(isValidSlug("lighthouse-keeper")).toBe(true);
    expect(isValidSlug("a1")).toBe(true);
    expect(isValidSlug("../etc/passwd")).toBe(false);
    expect(isValidSlug("generated/lighthouse")).toBe(false);
    expect(isValidSlug("Lighthouse")).toBe(false);
    expect(isValidSlug("")).toBe(false);
    expect(isValidSlug("a".repeat(49))).toBe(false);
    expect(isValidSlug(7)).toBe(false);
  });
});

describe("parseReviseRequest（服务端权威面）", () => {
  it("合法请求：feedback trim 后返回，allowLua 归一为布尔", () => {
    expect(
      parseReviseRequest({ slug: "lighthouse-keeper", feedback: "  改急一点  ", allowLua: true }),
    ).toEqual({
      ok: true,
      request: { slug: "lighthouse-keeper", feedback: "改急一点", allowLua: true },
    });
    expect(parseReviseRequest({ slug: "x1", feedback: "改" })).toEqual({
      ok: true,
      request: { slug: "x1", feedback: "改", allowLua: false },
    });
  });

  it("allowLua 只认字面 true（字符串 \"true\" 视为未授权——冻结是安全默认）", () => {
    const parsed = parseReviseRequest({ slug: "x1", feedback: "改", allowLua: "true" });
    expect(parsed).toEqual({ ok: true, request: { slug: "x1", feedback: "改", allowLua: false } });
  });

  it("请求体非对象 / 缺 slug / 非法 slug → 400", () => {
    for (const body of [null, [], "x", 7]) {
      const parsed = parseReviseRequest(body);
      expect(parsed.ok).toBe(false);
      if (!parsed.ok) expect(parsed.status).toBe(400);
    }
    const noSlug = parseReviseRequest({ feedback: "改" });
    expect(noSlug).toEqual({ ok: false, status: 400, error: expect.stringContaining("slug") });
    const traversal = parseReviseRequest({ slug: "../x", feedback: "改" });
    expect(traversal.ok).toBe(false);
  });

  it("缺意见 / 全空白 → 400；超长 → 413", () => {
    expect(parseReviseRequest({ slug: "x1" })).toEqual({
      ok: false,
      status: 400,
      error: expect.stringContaining("feedback"),
    });
    expect(parseReviseRequest({ slug: "x1", feedback: "   " }).ok).toBe(false);

    const long = parseReviseRequest({ slug: "x1", feedback: "改".repeat(MAX_FEEDBACK_CHARS + 1) });
    expect(long.ok).toBe(false);
    if (!long.ok) {
      expect(long.status).toBe(413);
      expect(long.error).toContain(String(MAX_FEEDBACK_CHARS));
    }
    // 恰好上限：放行（边界不误伤）
    expect(parseReviseRequest({ slug: "x1", feedback: "改".repeat(MAX_FEEDBACK_CHARS) }).ok).toBe(
      true,
    );
  });
});

describe("toReviseResponse（服务端结果投影）", () => {
  it("ok → changes/noop；空 diff 时 noop=true", () => {
    expect(
      toReviseResponse({
        ok: true,
        slug: "lighthouse-keeper",
        changed: ["scenes.scenes.storm"],
        issues: [],
      }),
    ).toEqual({ ok: true, slug: "lighthouse-keeper", changes: ["scenes.scenes.storm"], noop: false });

    expect(
      toReviseResponse({ ok: true, slug: "x1", changed: [], issues: [] }),
    ).toEqual({ ok: true, slug: "x1", changes: [], noop: true });
  });

  it("ok → changes 是副本（不把管线数组的引用漏给上游）", () => {
    const changed = ["modLua"];
    const response = toReviseResponse({ ok: true, slug: "x1", changed, issues: [] });
    changed.push("scenes.scenes.storm");
    expect(response.ok && response.changes).toEqual(["modLua"]);
  });

  it("失败面：只留 error 级诊断，行号折进 file", () => {
    const response = toReviseResponse({
      ok: false,
      slug: "x1",
      changed: ["modLua"],
      issues: [
        {
          gate: "revise",
          code: "REVISE_NOOP",
          file: "<response>",
          message: "无变更",
          severity: "warning",
        },
        {
          gate: "revise",
          code: "REVISE_LUA_FROZEN",
          file: "mod.lua",
          message: "Lua 冻结",
          severity: "error",
        },
        {
          gate: "playtest",
          code: "PLAYTEST",
          file: "mod.lua",
          line: 12,
          message: "走查未达锚",
          severity: "error",
        },
      ],
    });
    expect(response).toEqual({
      ok: false,
      issues: [
        { gate: "revise", code: "REVISE_LUA_FROZEN", file: "mod.lua", message: "Lua 冻结" },
        { gate: "playtest", code: "PLAYTEST", file: "mod.lua:12", message: "走查未达锚" },
      ],
    });
  });

  it("失败面全为 warning（诊断面异常）→ 退回全量，不给空错误框", () => {
    const response = toReviseResponse({
      ok: false,
      slug: "x1",
      changed: [],
      issues: [
        { gate: "revise", code: "REVISE_NOOP", file: "<response>", message: "无变更", severity: "warning" },
      ],
    });
    expect(response.ok).toBe(false);
    expect(response.ok === false && response.issues).toHaveLength(1);
  });
});

describe("parseReviseResponse（客户端形状解析）", () => {
  it("合法成功/失败包原样还原", () => {
    expect(
      parseReviseResponse({ ok: true, slug: "x1", changes: ["modLua"], noop: false }),
    ).toEqual({ ok: true, slug: "x1", changes: ["modLua"], noop: false });
    expect(
      parseReviseResponse({
        ok: false,
        issues: [{ gate: "revise", code: "REVISE_STRUCTURE", file: "data/scenes.json", message: "键集" }],
      }),
    ).toEqual({
      ok: false,
      issues: [{ gate: "revise", code: "REVISE_STRUCTURE", file: "data/scenes.json", message: "键集" }],
    });
  });

  it("形状不符一律 null（缺 noop / changes 非字符串数组 / issues 元素缺字段 / ok 非布尔）", () => {
    expect(parseReviseResponse(null)).toBeNull();
    expect(parseReviseResponse([])).toBeNull();
    expect(parseReviseResponse({ ok: "yes" })).toBeNull();
    expect(parseReviseResponse({ ok: true, slug: "x1", changes: [] })).toBeNull(); // 缺 noop
    expect(parseReviseResponse({ ok: true, slug: "", changes: [], noop: false })).toBeNull();
    expect(parseReviseResponse({ ok: true, slug: "x1", changes: [1], noop: false })).toBeNull();
    expect(parseReviseResponse({ ok: false, issues: [{ gate: "revise" }] })).toBeNull();
    expect(parseReviseResponse({ ok: false })).toBeNull();
  });
});
