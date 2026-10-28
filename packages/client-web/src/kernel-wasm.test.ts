// kernel-wasm 纯逻辑单测（B3）：UTF-8/base64 编码向量 + Mod 资源包组装 +
// 构造期错误映射。wasm 依赖路径（installWasiShim → init → YggKernel →
// dispatch 面）不在 vitest 覆盖——活链验证由 node smoke（crates/ygg-wasm/
// smoke.mjs）与浏览器端到端承担（门禁未覆盖声明见 commit 汇报）。
import { describe, expect, it } from "vitest";
import scenesJsonText from "../../mods/test-adventure/data/scenes.json?raw";
import packJson from "../../mods/test-adventure/pack.json";
import sceneSchemaText from "../../mods/test-adventure/schemas/scene-texts.json?raw";
import {
  assembleModBundle,
  bytesToBase64,
  describeInitError,
  encodeUtf8,
  utf8ToBase64,
  WasmKernelInitError,
} from "./kernel-wasm";

describe("encodeUtf8 / bytesToBase64（与壳内 base64_decode 互逆的标准向量）", () => {
  it("base64 空/尾组长 1/2/3 字节（RFC 4648 'foobar' 向量族）", () => {
    expect(bytesToBase64(new Uint8Array(0))).toBe("");
    expect(bytesToBase64(new Uint8Array([102]))).toBe("Zg==");
    expect(bytesToBase64(new Uint8Array([102, 111]))).toBe("Zm8=");
    expect(bytesToBase64(new Uint8Array([102, 111, 111]))).toBe("Zm9v");
    expect(bytesToBase64(new Uint8Array([102, 111, 111, 98]))).toBe("Zm9vYg==");
    expect(bytesToBase64(new Uint8Array([102, 111, 111, 98, 97]))).toBe("Zm9vYmE=");
    expect(bytesToBase64(new Uint8Array([102, 111, 111, 98, 97, 114]))).toBe("Zm9vYmFy");
  });

  it("UTF-8 多字节（中文 3 字节 + 代理对 4 字节）", () => {
    expect([...encodeUtf8("雨夜")]).toEqual([0xe9, 0x9b, 0xa8, 0xe5, 0xa4, 0x9c]);
    expect([...encodeUtf8("𝄞")]).toEqual([0xf0, 0x9d, 0x84, 0x9e]);
    expect(encodeUtf8("ab")).toEqual(new Uint8Array([97, 98]));
  });

  it("utf8ToBase64 已知向量（node 独立实现交叉验证）", () => {
    expect(utf8ToBase64("雨")).toBe("6Zuo");
    expect(utf8ToBase64("雨夜")).toBe("6Zuo5aSc");
    expect(utf8ToBase64("foo")).toBe("Zm9v");
  });
});

describe("assembleModBundle（真实 test-adventure pack 驱动）", () => {
  const pack = packJson as unknown as {
    entries: Array<{ path: string }>;
    schemaFiles: string[];
  };

  it("resources 键集 = pack.entries 全覆盖；值经 atob 独立解码回到原文 UTF-8 字节", () => {
    const bundle = assembleModBundle(pack, {
      modLua: "-- test-adventure",
      manifestJson: "{}",
      schemaTexts: { "schemas/scene-texts.json": sceneSchemaText },
      resourceTexts: { "data/scenes.json": scenesJsonText },
    });
    const resources: Record<string, string> = JSON.parse(bundle.resourcesJson);
    expect(Object.keys(resources).sort()).toEqual(pack.entries.map((e) => e.path).sort());
    // atob = jsdom/浏览器独立 base64 解码器：解码回二进制串，与自实现
    // UTF-8 编码逐字节对上（编码器 ↔ 解码器互逆）。
    const binary = atob(resources["data/scenes.json"]);
    const expectedBinary = String.fromCharCode(...encodeUtf8(scenesJsonText));
    expect(binary).toBe(expectedBinary);
  });

  it("schemas 序 = pack.schemaFiles 序（schema 文本原样入数组）", () => {
    const bundle = assembleModBundle(pack, {
      modLua: "-- test-adventure",
      manifestJson: "{}",
      schemaTexts: { "schemas/scene-texts.json": sceneSchemaText },
      resourceTexts: { "data/scenes.json": scenesJsonText },
    });
    expect(JSON.parse(bundle.schemasJson)).toEqual([sceneSchemaText]);
  });

  it("mod.lua / manifest.json 经 files 注入（去 test-adventure 硬编码，review round 1）", () => {
    const bundle = assembleModBundle(pack, {
      modLua: "-- injected",
      manifestJson: '{"k":1}',
      schemaTexts: { "schemas/scene-texts.json": sceneSchemaText },
      resourceTexts: { "data/scenes.json": scenesJsonText },
    });
    expect(bundle.modSource).toBe("-- injected");
    expect(bundle.manifestJson).toBe('{"k":1}');
  });

  it("声明文件缺供 → 快败 Error（装载前拦截，不进 wasm 构造期）", () => {
    expect(() =>
      assembleModBundle(pack, {
        modLua: "-- x",
        manifestJson: "{}",
        schemaTexts: {},
        resourceTexts: { "data/scenes.json": scenesJsonText },
      }),
    ).toThrow(/schemas\/scene-texts\.json/);
    expect(() =>
      assembleModBundle(pack, {
        modLua: "-- x",
        manifestJson: "{}",
        schemaTexts: { "schemas/scene-texts.json": sceneSchemaText },
        resourceTexts: {},
      }),
    ).toThrow(/data\/scenes\.json/);
  });
});

describe("describeInitError（构造期 throw 的两形态映射）", () => {
  it("A1 错误信封 JSON 文本（域失败）→ 提取 code + message", () => {
    const thrown = JSON.stringify({
      ok: false,
      error: { code: "INTERNAL", message: "mod load failed: syntax error" },
    });
    expect(describeInitError(thrown)).toBe(
      "内核装载失败（INTERNAL）：mod load failed: syntax error",
    );
  });

  it("信封 message 超长 → 截断加省略号", () => {
    const thrown = JSON.stringify({
      ok: false,
      error: { code: "VALIDATION", message: "x".repeat(300) },
    });
    const text = describeInitError(thrown);
    expect(text.startsWith("内核装载失败（VALIDATION）：")).toBe(true);
    expect(text.endsWith("…")).toBe(true);
    expect(text.length).toBeLessThan(300);
  });

  it("marshal 定位文案 / Error 实例 → 透传（wasm 初始化失败 前缀）", () => {
    expect(describeInitError("resources_json[\"p\"]: invalid base64 byte 0x2a")).toBe(
      "wasm 初始化失败：resources_json[\"p\"]: invalid base64 byte 0x2a",
    );
    expect(describeInitError(new Error("Failed to fetch dynamically imported module"))).toBe(
      "wasm 初始化失败：Failed to fetch dynamically imported module",
    );
    expect(describeInitError(42)).toBe("wasm 初始化失败：42");
  });

  it("形似 JSON 但非错误信封（ok:true / 缺 error）→ 透传路径", () => {
    expect(describeInitError('{"ok":true}')).toBe('wasm 初始化失败：{"ok":true}');
    expect(describeInitError('{"ok":false}')).toBe('wasm 初始化失败：{"ok":false}');
  });

  it("WasmKernelInitError 名面（降级横幅归因用）", () => {
    const err = new WasmKernelInitError("内核装载失败（INTERNAL）：boom");
    expect(err.name).toBe("WasmKernelInitError");
    expect(err.message).toContain("boom");
  });
});
