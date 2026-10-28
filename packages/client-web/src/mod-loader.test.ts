import { afterEach, describe, expect, it, vi } from "vitest";
import {
  assembleFromMaps,
  GeneratedModError,
  loadGeneratedBundle,
  slugFromGlobKey,
} from "./mod-loader";

const PACK_KEY = "../../mods/generated/demo/pack.json";
const PACK = {
  entries: [{ path: "data/scenes.json" }],
  schemaFiles: ["schemas/scene-texts.json"],
};
const RAWS = {
  "../../mods/generated/demo/mod.lua": "-- demo",
  "../../mods/generated/demo/manifest.json": "{}",
  "../../mods/generated/demo/data/scenes.json": "{}",
  "../../mods/generated/demo/schemas/scene-texts.json": "{}",
};

describe("mod-loader（FR-10：生成 Mod 动态载入）", () => {
  it("slugFromGlobKey 提取 slug；非 generated/ 键返回 null", () => {
    expect(slugFromGlobKey(PACK_KEY)).toBe("demo");
    expect(slugFromGlobKey("../../mods/test-adventure/pack.json")).toBeNull();
  });

  it("assembleFromMaps 组装五件套（resources base64 化由 assembleModBundle 承担）", () => {
    const bundle = assembleFromMaps({ [PACK_KEY]: PACK }, RAWS, "demo");
    expect(bundle.modSource).toBe("-- demo");
    expect(bundle.manifestJson).toBe("{}");
    expect(JSON.parse(bundle.schemasJson)).toEqual(["{}"]);
    expect(Object.keys(JSON.parse(bundle.resourcesJson))).toEqual(["data/scenes.json"]);
  });

  it("未知 slug → GeneratedModError（文案携可用列表与 dev 边界提示）", () => {
    expect(() => assembleFromMaps({}, RAWS, "nope")).toThrow(GeneratedModError);
    expect(() => assembleFromMaps({}, RAWS, "nope")).toThrow(/仅本地 dev/);
  });

  it("声明文件缺失 → GeneratedModError（提示重新 generate）", () => {
    expect(() => assembleFromMaps({ [PACK_KEY]: PACK }, {}, "demo")).toThrow(/重新运行 generate/);
  });
});

// ── C1 plan B：DEV middleware 回退分支（fetch 桩注入） ────────────────────
// loadGeneratedBundle 直读模块级 glob 图；generated/ 不入 git，vitest 环境
// 下静态图恒空 → 任何 slug 都走回退分支（若本地恰好存在同名生成目录，
// 「断言 fetch 被调用」会当场失败暴露，不致假绿）。生产路径（DEV=false）
// 无法在 vitest 内翻转（import.meta.env 编译期替换），由 vite plugin
// apply:"serve" + DEV 守卫 + build 门禁承担。

// middleware wire 契约样例（与 vite.plugin.mods.ts 的响应体同构）。
const WIRE_FILES = {
  pack: { entries: [{ path: "data/scenes.json" }], schemaFiles: ["schemas/scene-texts.json"] },
  modLua: "-- middleware fresh",
  manifestJson: "{}",
  schemaTexts: { "schemas/scene-texts.json": "{}" },
  resourceTexts: { "data/scenes.json": "{\"scene\":1}" },
};

describe("loadGeneratedBundle（DEV middleware 回退，C1 plan B）", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("glob miss → fetch /__generated-mod/<slug> → 2xx 组装 ModBundle", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => WIRE_FILES,
    });
    vi.stubGlobal("fetch", fetchMock);
    const bundle = await loadGeneratedBundle("middleware-fixture");
    expect(fetchMock).toHaveBeenCalledWith("/__generated-mod/middleware-fixture");
    expect(bundle.modSource).toBe("-- middleware fresh");
    expect(bundle.manifestJson).toBe("{}");
    expect(JSON.parse(bundle.schemasJson)).toEqual(["{}"]);
    expect(Object.keys(JSON.parse(bundle.resourcesJson))).toEqual(["data/scenes.json"]);
  });

  it("middleware 404 → GeneratedModError（保底横幅文案：可用列表 + dev 边界提示）", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({ ok: false, status: 404, json: async () => ({}) }),
    );
    await expect(loadGeneratedBundle("no-such-mod")).rejects.toThrow(GeneratedModError);
    await expect(loadGeneratedBundle("no-such-mod")).rejects.toThrow(/no-such-mod/);
    await expect(loadGeneratedBundle("no-such-mod")).rejects.toThrow(/仅本地 dev/);
  });

  it("middleware 网络异常 → 同样收敛为 GeneratedModError（不逃逸成加载失败面）", async () => {
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("network down")));
    await expect(loadGeneratedBundle("middleware-fixture")).rejects.toThrow(GeneratedModError);
  });

  it("middleware 2xx 但响应体损坏 → GeneratedModError（不抛裸 SyntaxError）", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: true,
        json: async () => {
          throw new SyntaxError("Unexpected token in JSON");
        },
      }),
    );
    await expect(loadGeneratedBundle("middleware-fixture")).rejects.toThrow(GeneratedModError);
  });
});
