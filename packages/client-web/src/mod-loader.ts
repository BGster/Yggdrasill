// mod-loader.ts — 生成 Mod 动态载入（FR-10 / 代决 3）：?mod=generated/<slug>
// 的五件套收集与组装。import.meta.glob 在模块转换期批量收集 packages/mods/
// generated/（research Q5 方案 A：零插件、vite 原生；目录 gitignored →
// build 产物为空 map → 部署站安全降级，「即载即玩」边界 = 本地 dev，D-6）。
// 解析核心抽为纯函数（fixture map 单测；不依赖 generated/ 真实内容——
// B3 lesson：单测门禁不得依赖 wasm 构建序与 gitignored 目录）。

import { assembleModBundle, type ModBundle } from "./kernel-wasm";

/** pack.json 声明面（与 kernel-wasm.ts 的 ModPack 同构；只消费这两个键）。 */
export interface GeneratedModPack {
  entries: Array<{ path: string }>;
  schemaFiles: string[];
}

/**
 * middleware wire 契约（C1 plan B）：GET /__generated-mod/<slug> 的响应体，
 * 字段面 = assembleFromMaps 的文件面（pack + 固定两件 + 声明面文本 map），
 * 客户端直接喂 assembleModBundle。单一事实源：vite.plugin.mods.ts type-only
 * import 本接口；改动必须双侧同步（wire 契约）。
 */
export interface GeneratedModFiles {
  pack: GeneratedModPack;
  modLua: string;
  manifestJson: string;
  schemaTexts: Record<string, string>;
  resourceTexts: Record<string, string>;
}

/** 生成 Mod 不可用（App 落 notice 横幅 + 回退默认冒险）。 */
export class GeneratedModError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "GeneratedModError";
  }
}

/** dev middleware 端点前缀（与 vite.plugin.mods.ts 的 GENERATED_MOD_PREFIX 一致）。 */
const MIDDLEWARE_PREFIX = "/__generated-mod";

// vite 静态收集：pack.json 解析为对象，其余五件以 ?raw 文本进包。
const PACK_MODULES = import.meta.glob("../../mods/generated/*/pack.json", {
  eager: true,
  import: "default",
}) as Record<string, GeneratedModPack>;
const RAW_MODULES = {
  ...(import.meta.glob("../../mods/generated/*/mod.lua", {
    eager: true,
    query: "?raw",
    import: "default",
  }) as Record<string, string>),
  ...(import.meta.glob("../../mods/generated/*/manifest.json", {
    eager: true,
    query: "?raw",
    import: "default",
  }) as Record<string, string>),
  ...(import.meta.glob("../../mods/generated/*/data/*.json", {
    eager: true,
    query: "?raw",
    import: "default",
  }) as Record<string, string>),
  ...(import.meta.glob("../../mods/generated/*/schemas/*.json", {
    eager: true,
    query: "?raw",
    import: "default",
  }) as Record<string, string>),
};

/** glob 键 → slug（"../../mods/generated/foo/data/x.json" → "foo"）。 */
export function slugFromGlobKey(key: string): string | null {
  const match = /mods\/generated\/([^/]+)\//.exec(key);
  return match?.[1] ?? null;
}

/** 当前目录内可用的生成 Mod slug 列表（dev 诊断面）。 */
export function listGeneratedSlugs(): string[] {
  return Object.keys(PACK_MODULES)
    .map(slugFromGlobKey)
    .filter((slug): slug is string => slug !== null)
    .sort();
}

/**
 * 纯解析核：从 glob 形态的 map 组装 ModBundle（单测注入 fixture map）。
 * mod.lua / manifest.json 为固定 bundle 成员，schema/资源按 pack.json 声明面
 * 收集；slug 不存在 / 任何成员缺失 → GeneratedModError（缺文件 = 生成目录
 * 不完整，提示重新运行 generate，不做部分装载）。
 */
export function assembleFromMaps(
  packs: Record<string, GeneratedModPack>,
  raws: Record<string, string>,
  slug: string,
): ModBundle {
  const packKey = Object.keys(packs).find((key) => slugFromGlobKey(key) === slug);
  if (packKey === undefined) {
    const available = Object.keys(packs)
      .map(slugFromGlobKey)
      .filter((value): value is string => value !== null)
      .join(", ");
    throw new GeneratedModError(
      `未找到生成 Mod「${slug}」（可用：${available || "无"}；生成 Mod 仅本地 dev 可用）`,
    );
  }
  const pack = packs[packKey]!;
  // mod.lua 与 manifest.json 是固定 bundle 成员（pack.json 声明面之外），按 slug
  // 直取（review round 1：assembleModBundle files 面扩展后由此传入，不再硬编码）。
  const modLua = rawText(raws, slug, "mod.lua");
  const manifestJson = rawText(raws, slug, "manifest.json");
  const schemaTexts: Record<string, string> = {};
  const resourceTexts: Record<string, string> = {};
  for (const relative of [...pack.schemaFiles, ...pack.entries.map((entry) => entry.path)]) {
    const text = rawText(raws, slug, relative);
    if (pack.schemaFiles.includes(relative)) {
      schemaTexts[relative] = text;
    } else {
      resourceTexts[relative] = text;
    }
  }
  return assembleModBundle(pack, { modLua, manifestJson, schemaTexts, resourceTexts });
}

/** RAW_MODULES 内按 slug + 包内相对路径取文本；缺失 → GeneratedModError。 */
function rawText(raws: Record<string, string>, slug: string, relative: string): string {
  const text = Object.entries(raws).find(
    ([key]) => slugFromGlobKey(key) === slug && key.endsWith(`/${relative}`),
  )?.[1];
  if (text === undefined) {
    throw new GeneratedModError(`生成 Mod「${slug}」缺少声明文件 ${relative}（重新运行 generate）`);
  }
  return text;
}

/** App 消费面：?mod=generated/<slug> → ModBundle。
 *
 * 解析序：先走 import.meta.glob 静态图（构建期收集，含既有 slug）；命中即组装。
 * 未命中且处于 vite dev（import.meta.env.DEV）→ 转投 dev middleware
 * （/__generated-mod/<slug>，vite.plugin.mods.ts 按需读盘）——vite 6 glob 不重扫
 * **新增**文件（M1 spike 实证），dev server 运行期新生成的 Mod 只有这条通道
 * 可见。生产构建不回退（generated/ 不入包，DEV 恒 false），路径不变。
 * middleware 404（真缺失/不完整）→ GeneratedModError，文案保持「仅本地 dev
 * 可用」提示——App 的降级横幅行为不变。
 */
export async function loadGeneratedBundle(slug: string): Promise<ModBundle> {
  try {
    return assembleFromMaps(PACK_MODULES, RAW_MODULES, slug);
  } catch (error) {
    if (!(error instanceof GeneratedModError) || !import.meta.env.DEV) {
      throw error;
    }
    return loadFromMiddleware(slug);
  }
}

/** dev middleware 回退：响应 2xx → 组装；一切失败（404/网络/坏 JSON）→
 * GeneratedModError（保底同一降级横幅，文案携静态图可用列表）。 */
async function loadFromMiddleware(slug: string): Promise<ModBundle> {
  const notFound = () =>
    new GeneratedModError(
      `未找到生成 Mod「${slug}」（可用：${listGeneratedSlugs().join(", ") || "无"}；生成 Mod 仅本地 dev 可用）`,
    );
  try {
    const response = await fetch(`${MIDDLEWARE_PREFIX}/${encodeURIComponent(slug)}`);
    if (!response.ok) {
      throw notFound();
    }
    const files = (await response.json()) as GeneratedModFiles;
    return assembleModBundle(files.pack, files);
  } catch (error) {
    if (error instanceof GeneratedModError) {
      throw error;
    }
    throw notFound();
  }
}
