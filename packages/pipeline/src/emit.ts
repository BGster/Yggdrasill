// emit.ts — 五件套落盘（FR-9）：packages/mods/generated/<slug>/，五件与
// test-adventure 同构（mod.lua / pack.json / manifest.json / data/ / schemas/）。
// 调用方保证终态才落盘（D-15）；playtest 门复用本函数物化临时目录。

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { ModCandidate } from "./types.js";

export interface EmitPaths {
  dir: string;
  files: string[];
}

const BUNDLE_FILES: Array<[relative: string, pick: (candidate: ModCandidate) => string]> = [
  ["mod.lua", (candidate) => candidate.modLua],
  ["pack.json", (candidate) => candidate.packJson],
  ["manifest.json", (candidate) => candidate.manifestJson],
  ["data/scenes.json", (candidate) => candidate.scenesJson],
  ["schemas/scene-texts.json", (candidate) => candidate.sceneSchemaJson],
];

/** 五件套写入 <root>/<slug>/（root = generated 根或临时目录）。 */
export function emitModBundle(root: string, slug: string, candidate: ModCandidate): EmitPaths {
  const dir = join(root, slug);
  mkdirSync(join(dir, "data"), { recursive: true });
  mkdirSync(join(dir, "schemas"), { recursive: true });
  const files: string[] = [];
  for (const [relative, pick] of BUNDLE_FILES) {
    writeFileSync(join(dir, relative), pick(candidate), "utf8");
    files.push(relative);
  }
  return { dir, files };
}
