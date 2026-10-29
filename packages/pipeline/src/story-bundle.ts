// story-bundle.ts — revise 的磁盘回读面 + title 锚点（A4）。
//
// 回读为什么要锚点：emit 落盘的五件套（mod.lua / pack.json / manifest.json /
// data/scenes.json / schemas/scene-texts.json）里 **没有 slug 与 title**——
// pack.json 是管线固定模板（D-9：LLM 不产此文件），manifest 只串内容寻址。
// slug 可由目录名恢复，title 不能。而 revise 的 title 冻结校验需要一个可信
// 原标题，故在 bundle **目录外**记一份锚点：
//
//   <outRoot>/.meta/<slug>.json  = {"slug": …, "title": …}
//
// 放在目录外的理由：五件套是分发产物（内核按 pack.json 声明面读、client-web
// 的 dev middleware 同口径），不掺入管线私有的工作状态；锚点跟着 outRoot 走
// （outRoot 可变——测试用临时目录），不进 git（generated/ 已 gitignore）。
//
// 降级：A1-A3 期生成的 bundle 没有锚点（titleKnown=false）——此时 title 无法
// 判定是否被改，校验跳过，revise 成功后补写锚点（从下一轮起恢复冻结）。

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { StoryMod } from "./types.js";

/** 锚点目录名（outRoot 下的隐藏子目录，与 mod 目录并列但不在其内）。 */
export const STORY_META_DIR = ".meta";

/** 五件套内的固定文件名（读回面）。 */
const MOD_LUA = "mod.lua";
const PACK_JSON = "pack.json";
const SCENES_JSON = join("data", "scenes.json");

export interface StoryMeta {
  slug: string;
  title: string;
}

export interface LoadedStory {
  story: StoryMod;
  /** bundle 目录（绝对或相对，随 outRoot 传入形态）。 */
  dir: string;
  /** 原标题是否可信（有锚点）；false 时 title 冻结校验降级跳过。 */
  titleKnown: boolean;
}

export function storyMetaPath(outRoot: string, slug: string): string {
  return join(outRoot, STORY_META_DIR, `${slug}.json`);
}

/** 写 title 锚点（成功后调用；2 空格缩进 + 尾换行，与管线其他 JSON 同形）。 */
export function writeStoryMeta(outRoot: string, slug: string, story: StoryMod): string {
  const path = storyMetaPath(outRoot, slug);
  mkdirSync(join(outRoot, STORY_META_DIR), { recursive: true });
  const meta: StoryMeta = { slug, title: story.title };
  writeFileSync(path, `${JSON.stringify(meta, null, 2)}\n`, "utf8");
  return path;
}

/** 读 title 锚点；缺失 = null（A1-A3 期产物，降级路径）。损坏则抛错（仓库态错）。 */
export function readStoryMeta(outRoot: string, slug: string): StoryMeta | null {
  const path = storyMetaPath(outRoot, slug);
  if (!existsSync(path)) {
    return null;
  }
  return parseJson<StoryMeta>(path, "锚点");
}

/**
 * 从磁盘 bundle 重组 StoryMod（revise 的输入面）。
 *
 * 权威来源：**bundle 内文件**——scenes 取 data/scenes.json、modLua 取 mod.lua
 * （手改过的 bundle 以磁盘为准），slug 取目录名（= 作品 id 的物理落点）。
 * 锚点只补 title 这一个不可恢复字段，且只在校验 title 冻结时用。
 *
 * 抛错（配置/仓库态错误，不是门失败）：目录不存在、pack.json 不是管线模板、
 * 五件套缺件、JSON 损坏——这些都意味着「没有可修改的既有作品」。
 */
export function loadBundleStory(outRoot: string, slug: string): LoadedStory {
  const dir = join(outRoot, slug);
  if (!existsSync(dir)) {
    throw new Error(
      `找不到要修改的 Mod bundle：${dir}（先 pnpm -C packages/pipeline run generate 生成，或检查 --out 目录）`,
    );
  }
  assertPipelinePack(dir);
  const modLua = readText(join(dir, MOD_LUA), dir);
  const scenes = readJson<Record<string, unknown>>(join(dir, SCENES_JSON), dir);
  const meta = readStoryMeta(outRoot, slug);
  return {
    dir,
    titleKnown: meta !== null,
    story: {
      slug,
      // 无锚点时原标题不可恢复（五件套本就不存 title）：用 slug 占位，保证
      // story 契约（title minLength 1）能过——否则基线 JSON 带空 title，模型
      // 会忠实回填空串，整轮卡在契约门。占位不参与冻结校验（titleKnown=false）。
      title: meta?.title ?? slug,
      modLua,
      scenes,
    },
  };
}

/** pack.json 契约自检：只接受管线模板声明面（避免把手工/外部 mod 目录当产物改）。 */
function assertPipelinePack(dir: string): void {
  const pack = readJson<{ entries?: Array<{ id?: unknown; path?: unknown }>; schemaFiles?: unknown }>(
    join(dir, PACK_JSON),
    dir,
  );
  const entries = Array.isArray(pack.entries) ? pack.entries : [];
  const hasSceneEntry = entries.some(
    (entry) => entry.id === "ygg:scene/texts" && entry.path === "data/scenes.json",
  );
  const schemaFiles = Array.isArray(pack.schemaFiles) ? pack.schemaFiles : [];
  if (!hasSceneEntry || !schemaFiles.includes("schemas/scene-texts.json")) {
    throw new Error(
      `不是管线生成的 Mod bundle（pack.json 声明面不符，D-9 固定模板）：${dir}`,
    );
  }
}

function readText(path: string, dir: string): string {
  if (!existsSync(path)) {
    throw new Error(`bundle 缺件：${path}（${dir} 不是完整五件套，先用 generate 重建）`);
  }
  return readFileSync(path, "utf8");
}

function readJson<T>(path: string, dir: string): T {
  readText(path, dir); // 缺件时给出与 readText 一致的 bundle 语境错误
  return parseJson<T>(path, "bundle 文件");
}

function parseJson<T>(path: string, what: string): T {
  try {
    return JSON.parse(readFileSync(path, "utf8")) as T;
  } catch (cause) {
    throw new Error(`${what}不是合法 JSON：${path}`, { cause });
  }
}
