// vite.plugin.mods.ts — 生成 Mod dev 热载 middleware（C1 plan B）。
//
// 背景：vite 6 的 import.meta.glob 收集发生在模块转换期，**不重扫新增文件**
//（M1 spike 实证：modification 事件触发失效重收集，new file 事件不入图）——
// dev server 运行期新生成的 Mod 首访会落「不可用」横幅。本插件在 dev 侧
// 补一条按需读盘通道：GET /__generated-mod/<slug> 现场读
// packages/mods/generated/<slug>/（pack.json / mod.lua / manifest.json +
// pack.json 声明面的 schemas/ 与 data/），返回 wire 契约 JSON（类型单源 =
// src/mod-loader.ts 的 GeneratedModFiles；type-only import，编译期擦除）。
//
// 边界：
// - apply: "serve"——仅 dev server 挂载；build / vitest（mode test）不加载，
//   生产路径仍走 glob（generated/ 目录 gitignored，构建产物本就为空）；
// - 读盘发生在请求时（fs.readFile），零缓存零启动期扫描——新增 slug 天然可见；
// - 任一文件缺失 / pack.json 损坏 → 404 + {error}：客户端把一切非 2xx 映射
//   回既有 GeneratedModError 降级横幅（缺 slug 与缺文件的 UI 语义一致）；
// - slug 白名单正则（字母数字开头，限 [A-Za-z0-9._-]，禁 / 与 ..）——杜绝
//   路径穿越；decodeURIComponent 失败同 404。

import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import type { Plugin } from "vite";
import type { GeneratedModFiles, GeneratedModPack } from "./src/mod-loader";

export const GENERATED_MOD_PREFIX = "/__generated-mod";

/** slug 白名单：目录名安全子集（拒绝空串、路径分隔符、前导点）。 */
const SLUG_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._-]*$/;

const GENERATED_DIR = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../mods/generated",
);

/** 现场读盘收集一个 slug 的五件套（声明面驱动，与 assembleFromMaps 同构）。 */
async function collectGeneratedMod(slug: string): Promise<GeneratedModFiles> {
  const dir = path.join(GENERATED_DIR, slug);
  const read = (relative: string) => readFile(path.join(dir, relative), "utf8");
  let pack: GeneratedModPack;
  try {
    pack = JSON.parse(await read("pack.json")) as GeneratedModPack;
  } catch (error) {
    // 缺文件 / 损坏 JSON 统一转带 slug 上下文的错误（调用方 404 兑现）。
    throw new Error(
      `pack.json 不可用：${error instanceof Error ? error.message : String(error)}`,
    );
  }
  const schemaTexts: Record<string, string> = {};
  const resourceTexts: Record<string, string> = {};
  for (const relative of [...pack.schemaFiles, ...pack.entries.map((e) => e.path)]) {
    const text = await read(relative);
    if (pack.schemaFiles.includes(relative)) {
      schemaTexts[relative] = text;
    } else {
      resourceTexts[relative] = text;
    }
  }
  return {
    pack,
    modLua: await read("mod.lua"),
    manifestJson: await read("manifest.json"),
    schemaTexts,
    resourceTexts,
  };
}

/** dev-only middleware 插件（vite.config.ts plugins 挂载）。 */
export function generatedModsMiddleware(): Plugin {
  return {
    name: "ygg-generated-mods-middleware",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use(GENERATED_MOD_PREFIX, (req, res) => {
        void (async () => {
          const send = (status: number, body: unknown) => {
            res.statusCode = status;
            res.setHeader("Content-Type", "application/json; charset=utf-8");
            res.end(JSON.stringify(body));
          };
          try {
            // connect 剥掉前缀后 req.url = "/<slug>"（可能带查询串）。
            const slug = decodeURIComponent((req.url ?? "").split("?")[0] ?? "").replace(
              /^\/+/,
              "",
            );
            if (!SLUG_PATTERN.test(slug)) {
              send(404, { error: `非法 slug「${slug}」` });
              return;
            }
            send(200, await collectGeneratedMod(slug));
          } catch (error) {
            // slug 解码异常 / 目录不存在 / 成员缺失 / pack.json 损坏一律 404
            //（= 重新 generate）；不让任何 rejection 逃出 connect。
            send(404, {
              error: `生成 Mod 读盘失败：${error instanceof Error ? error.message : String(error)}（重新运行 generate）`,
            });
          }
        })();
      });
    },
  };
}
