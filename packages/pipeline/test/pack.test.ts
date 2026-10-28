import { createHash } from "node:crypto";
import { describe, expect, it } from "vitest";
import {
  buildManifestJson,
  buildPackJson,
  buildScenesJson,
  safeSlug,
  sha256Hex,
} from "../src/pack.js";

const SCENES = {
  scenes: { begin: "起点" },
  endings: { a: "【结局】终。" },
  prompts: {},
  options: {},
  beats: {},
  tuning: {},
};

describe("pack（D-7：对齐 pack.rs 线格式）", () => {
  it("manifest 键序 = 字典序（pack.rs 实跑形态，research Q4）", () => {
    const manifest = JSON.parse(buildManifestJson(buildScenesJson(SCENES))) as Record<string, unknown>;
    expect(Object.keys(manifest)).toEqual(["entries", "schemaVersion"]);
    const entry = (manifest.entries as Array<Record<string, unknown>>)[0]!;
    expect(Object.keys(entry)).toEqual(["byteLength", "id", "kind", "path", "schema", "sha256"]);
  });

  it("sha256/byteLength 对条目原始字节现算（含尾换行）", () => {
    const scenesJson = buildScenesJson(SCENES);
    const manifest = JSON.parse(buildManifestJson(scenesJson)) as {
      entries: Array<{ sha256: string; byteLength: number }>;
    };
    expect(manifest.entries[0]!.sha256).toBe(
      createHash("sha256").update(scenesJson, "utf8").digest("hex"),
    );
    expect(manifest.entries[0]!.byteLength).toBe(Buffer.byteLength(scenesJson, "utf8"));
  });

  it("sha256Hex 命中固定向量（hello）", () => {
    expect(sha256Hex("hello")).toBe(
      "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
    );
  });

  it("pretty 2 空格 + 尾部换行（pack.rs:137 形态）", () => {
    const scenesJson = buildScenesJson({ a: 1 });
    expect(scenesJson.startsWith('{\n  "a": 1\n}')).toBe(true);
    expect(scenesJson.endsWith("}\n")).toBe(true);
  });

  it("pack.json 固定模板（D-9：单条目 + schemaFiles；LLM 不产此文件）", () => {
    const pack = JSON.parse(buildPackJson()) as {
      schemaVersion: number;
      entries: Array<{ id: string; kind: string; path: string }>;
      schemaFiles: string[];
    };
    expect(pack.schemaVersion).toBe(1);
    expect(pack.entries).toHaveLength(1);
    expect(pack.entries[0]).toMatchObject({
      id: "ygg:scene/texts",
      kind: "scene-texts",
      path: "data/scenes.json",
    });
    expect(pack.schemaFiles).toEqual(["schemas/scene-texts.json"]);
  });

  it("safeSlug 净化非法字符并兜底", () => {
    expect(safeSlug("My Story! 学院" )).toBe("my-story");
    expect(safeSlug("!!!")).toMatch(/^story-[0-9a-z]+$/);
  });
});
