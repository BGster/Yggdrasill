import { describe, expect, it } from "vitest";
import {
  buildManifestJson,
  buildPackJson,
  buildScenesJson,
} from "../src/pack.js";
import {
  loadSchemaSchemas,
  validateBundle,
  validateStory,
} from "../src/schema-gate.js";
import type { ModCandidate } from "../src/types.js";
import { repoRoot } from "../src/env.js";

const GOOD_SCENES = {
  scenes: { begin: "起点" },
  endings: { a: "【结局】终。" },
  prompts: { door: "推门吗" },
  options: { enter: "推门" },
  beats: {},
  tuning: {},
};

const GOOD_STORY = {
  slug: "demo-tale",
  title: "演示",
  modLua: "-- demo",
  scenes: GOOD_SCENES,
};

describe("schema-gate（FR-4：ajv 2020-12，packages/schema 单源）", () => {
  it("story 契约：合法 story 零 issue", () => {
    expect(validateStory(GOOD_STORY)).toEqual([]);
  });

  it("story 契约：slug 非法 → <response> 定位 issue", () => {
    const issues = validateStory({ ...GOOD_STORY, slug: "Bad Slug!" });
    expect(issues.length).toBeGreaterThan(0);
    expect(issues[0]).toMatchObject({ gate: "schema", file: "<response>", code: "pattern" });
  });

  it("story 契约：endings 值缺【结局前缀 → pattern 拦截（D-10 咬合锚）", () => {
    const broken = {
      ...GOOD_STORY,
      scenes: { ...GOOD_SCENES, endings: { a: "终局" } },
    };
    const issues = validateStory(broken);
    expect(issues.some((issue) => issue.code === "pattern")).toBe(true);
  });

  it("bundle：合法候选零 issue（manifest 过 D15 单源 + scenes 过通用模板）", () => {
    const schemas = loadSchemaSchemas(joinSchemaDir());
    const candidate = makeCandidate(GOOD_SCENES);
    expect(validateBundle(schemas, candidate)).toEqual([]);
  });

  it("bundle：坏 manifest（sha256 非法形态）→ manifest.json 定位 issue", () => {
    const schemas = loadSchemaSchemas(joinSchemaDir());
    const candidate = makeCandidate(GOOD_SCENES);
    candidate.manifestJson = candidate.manifestJson.replace(
      /"sha256": "[0-9a-f]{64}"/,
      '"sha256": "nothex"',
    );
    const issues = validateBundle(schemas, candidate);
    // review round 1：只断言 file/code，不断言 line——坏 sha256 的 ajv 指针是
    // /entries/0/sha256（含数组下标 token），locate.ts 只解析对象键不做数组
    // 定位，position 为 null（取舍：M1 保持最小面，数组定位需求出现再扩展）。
    expect(issues[0]).toMatchObject({ gate: "schema", file: "manifest.json", code: "pattern" });
  });

  it("bundle：endings 空对象（minProperties 违规）→ data/scenes.json 定位 issue", () => {
    const schemas = loadSchemaSchemas(joinSchemaDir());
    const candidate = makeCandidate({ ...GOOD_SCENES, endings: {} });
    const issues = validateBundle(schemas, candidate);
    expect(issues[0]).toMatchObject({ gate: "schema", file: "data/scenes.json" });
  });
});

function joinSchemaDir(): string {
  return `${repoRoot()}/packages/schema/schemas`;
}

function makeCandidate(scenes: Record<string, unknown>): ModCandidate {
  const scenesJson = buildScenesJson(scenes);
  return {
    story: GOOD_STORY,
    modLua: "-- demo",
    scenesJson,
    packJson: buildPackJson(),
    manifestJson: buildManifestJson(scenesJson),
    sceneSchemaJson: "{}",
  };
}
