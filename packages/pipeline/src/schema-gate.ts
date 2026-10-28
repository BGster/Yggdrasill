// schema-gate.ts — ajv 2020-12 校验（FR-4 / D15 单源）：story 契约（LLM 输出）
// + manifest.json + data/scenes.json。schema 文件全部读自 packages/schema/
// （管线不内嵌任何副本——铁律 2；scene-texts 通用模板由 D-10 落
// packages/schema/schemas/，与 manifest.json 同目录单源）。

import Ajv2020 from "ajv/dist/2020";
import type { ErrorObject, ValidateFunction } from "ajv";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { locatePointer, type SourcePosition } from "./locate.js";
import { STORY_SCHEMA, type GateIssue, type ModCandidate } from "./types.js";

const ajv = new Ajv2020({ allErrors: true });

/** 同一 $id 只注册一次（ajv 对重复 $id 的 compile 抛 "already exists"；
 * schema 运行期静态，按 $id 复用编译产物语义等价）。 */
function compile(schema: Record<string, unknown>): ValidateFunction {
  const id = schema.$id;
  const cached = typeof id === "string" ? ajv.getSchema(id) : undefined;
  return cached ?? ajv.compile(schema);
}

export interface SchemaSchemas {
  /** packages/schema/schemas/manifest.json（D15 契约单源）。 */
  manifest: Record<string, unknown>;
  /** packages/schema/schemas/scene-texts.json（D-10 通用模板）。 */
  sceneTexts: Record<string, unknown>;
}

/** 加载 schema 单源文件。try/catch 包装后仍 fail-fast：这些是仓库控制的契约
 * 文件（非外部输入），损坏即开发者错误——抛带路径的可读错误优于裸 SyntaxError。 */
function parseSchemaFile(path: string): Record<string, unknown> {
  try {
    return JSON.parse(readFileSync(path, "utf8")) as Record<string, unknown>;
  } catch (cause) {
    throw new Error(
      `schema 契约文件损坏或非 JSON：${path}（修复 packages/schema 侧单源文件）`,
      { cause },
    );
  }
}

export function loadSchemaSchemas(schemaDir: string): SchemaSchemas {
  return {
    manifest: parseSchemaFile(join(schemaDir, "manifest.json")),
    sceneTexts: parseSchemaFile(join(schemaDir, "scene-texts.json")),
  };
}

/** story 契约校验（pack 之前的第一道失败面；file = "<response>"）。 */
export function validateStory(story: unknown): GateIssue[] {
  const validate = compile(STORY_SCHEMA as Record<string, unknown>);
  if (validate(story)) {
    return [];
  }
  return (validate.errors ?? []).map((error) => issueFromAjv("<response>", error));
}

/** 产物校验：manifest + data/scenes.json（对 pack.ts 现算结果）。
 * 两处 parse 输入是同进程 JSON.stringify 产物，不可能非法；try/catch 只把
 * 「不可能路径」转成带上下文的可读错误（仍是 fail-fast，不产生门 issue）。 */
function parseInProcessJson(text: string, what: string): Record<string, unknown> {
  try {
    const parsed: unknown = JSON.parse(text);
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
      throw new Error("顶层不是 JSON 对象");
    }
    return parsed as Record<string, unknown>;
  } catch (cause) {
    throw new Error(`内部错误：${what} 不是合法 JSON（pack.ts 产物不应出现）`, { cause });
  }
}

export function validateBundle(schemas: SchemaSchemas, candidate: ModCandidate): GateIssue[] {
  const issues: GateIssue[] = [];
  const manifest = parseInProcessJson(candidate.manifestJson, "manifestJson");
  const scenes = parseInProcessJson(candidate.scenesJson, "scenesJson");

  const validateManifest = compile(schemas.manifest);
  if (!validateManifest(manifest)) {
    for (const error of validateManifest.errors ?? []) {
      issues.push(issueFromAjv("manifest.json", error, candidate.manifestJson));
    }
  }
  const validateScenes = compile(schemas.sceneTexts);
  if (!validateScenes(scenes)) {
    for (const error of validateScenes.errors ?? []) {
      issues.push(issueFromAjv("data/scenes.json", error, candidate.scenesJson));
    }
  }
  return issues;
}

function issueFromAjv(file: string, error: ErrorObject, jsonText?: string): GateIssue {
  const position: SourcePosition | null = jsonText
    ? locatePointer(jsonText, error.instancePath)
    : null;
  return {
    gate: "schema",
    file,
    line: position?.line,
    column: position?.column,
    code: error.keyword,
    severity: "error",
    message: `${error.instancePath || "/"} ${error.message ?? "校验失败"}`,
    hint: `JSON Pointer ${error.instancePath || "/"}${position ? `（${file}:${position.line}:${position.column}）` : ""}`,
  };
}
