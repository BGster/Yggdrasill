// pack.ts — manifest 现算（FR-3 / D-7；对齐 pack.rs 线格式，research Q4 对齐
// 清单 10 点）。键序 = pack.rs 实跑输出的字典序（serde_json 无 preserve_order）：
//   顶层 ["entries","schemaVersion"]，entry ["byteLength","id","kind","path","schema","sha256"]。
// 序列化 = 2 空格缩进 + 尾部换行（pack.rs:137）。内核装载期内容寻址校验兜底
// 一致性（不对齐即 ContentMismatch 暴露）——TS 侧不需要 Rust 工具链在环。
// **生成器侧文件**：本文件生成的是 `packages/mods/test-adventure` 同构的 Mod（结局前缀 / 入口事件写进生成物与提示词）；换 Mod 契约需同步修改。

import { createHash } from "node:crypto";

const SCENE_RESOURCE_ID = "ygg:scene/texts";
const SCENE_RESOURCE_KIND = "scene-texts";
export const SCENE_SCHEMA_URL = "https://yggdrasill.dev/schemas/v1/scene-texts.json";
export const SCENE_DATA_PATH = "data/scenes.json";
export const SCENE_SCHEMA_PATH = "schemas/scene-texts.json";

/** pack.json 文本（管线固定模板——LLM 不产此文件，D-9）。 */
export function buildPackJson(): string {
  const pack = {
    schemaVersion: 1,
    entries: [
      {
        id: SCENE_RESOURCE_ID,
        kind: SCENE_RESOURCE_KIND,
        schema: SCENE_SCHEMA_URL,
        path: SCENE_DATA_PATH,
      },
    ],
    schemaFiles: [SCENE_SCHEMA_PATH],
    // host 契约（必填）：事件名 / 结局前缀 / 叙事约定由 Mod 自己声明，通用工具
    // （playtest / agent）不含字面量。生成物没有异步 inbox → 只声明三个保留键。
    agent: {
      entryEvent: "event.start",
      choiceEvent: "ygg.choice_selected",
      summaryEvent: "agent.summary",
      endingPrefix: "【结局",
      metaKey: "summary",
      rejectionPrefixes: ["【拒绝】"],
      replayPrefix: "【重放】",
      defaultPayload: "0",
    },
    inboxRoutes: {
      externalIdField: "external_id",
      payloadField: "payload",
      extMarker: "@ext=",
    },
  };
  return `${JSON.stringify(pack, null, 2)}\n`;
}

/** data/scenes.json 文本（pretty 2 空格 + 尾换行——locate.ts 行映射的稳定前提）。 */
export function buildScenesJson(scenes: Record<string, unknown>): string {
  return `${JSON.stringify(scenes, null, 2)}\n`;
}

/** 小写 hex sha256（对条目原始字节；pack.rs:216 同口径）。 */
export function sha256Hex(text: string): string {
  return createHash("sha256").update(text, "utf8").digest("hex");
}

/**
 * manifest.json 文本（schemaVersion 1 线格式）。键序硬约束：按字典序构造
 * 对象（JS 字符串键保持插入序）；JSON.stringify(…, null, 2) 与 serde_json
 * pretty 的缩进/冒号后空格/空容器形态一致——字节级对齐 pack.rs 实跑输出
 * （注意：仓库 test-adventure/manifest.json 是手维护插入序，语义等价、字节
 * 不同——对拍基准是 pack.rs 输出形态，不是仓库样本，research Q4.8）。
 */
export function buildManifestJson(scenesJson: string): string {
  const manifest = {
    entries: [
      {
        byteLength: Buffer.byteLength(scenesJson, "utf8"),
        id: SCENE_RESOURCE_ID,
        kind: SCENE_RESOURCE_KIND,
        path: SCENE_DATA_PATH,
        schema: SCENE_SCHEMA_URL,
        sha256: sha256Hex(scenesJson),
      },
    ],
    schemaVersion: 1,
  };
  return `${JSON.stringify(manifest, null, 2)}\n`;
}

/** slug 目录名净化（story 契约已限 [a-z0-9-]；失败回退路径的二道防线）。 */
export function safeSlug(slug: string): string {
  const cleaned = slug
    .toLowerCase()
    .replace(/[^a-z0-9-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 48);
  return cleaned.length > 0 ? cleaned : `story-${Date.now().toString(36)}`;
}
