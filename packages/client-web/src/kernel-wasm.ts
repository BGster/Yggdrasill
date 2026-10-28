// ============================================================================
// kernel-wasm.ts — 真实 wasm 内核绑定（B3）。KernelBridge 的 wasm 实现：
// 装载 packages/mods/test-adventure 资源包 → 实例化 crates/ygg-wasm 的
// YggKernel → dispatch 面逐字透传（A1 信封）。
//
// 装载序（硬约束，见 crates/ygg-wasm/wasi-shim.js 头注）：
//   1. installWasiShim() —— 补丁 WebAssembly.instantiate 以并入 preview-1
//      imports 并**捕获模块 memory**（写内存的 wasi 调用依赖）；
//   2. init()（胶水默认导出）—— 拉取并实例化 ygg_wasm_bg.wasm；
//   3. new YggKernel(mod_source, manifest_json, schemas_json, resources_json)
//      —— 构造期域失败 **throw**（值 = A1 错误信封 JSON 文本或 marshal 文案），
//      在此 catch 一次映射为人读中文错误（describeInitError）。
// dispatch 永不 throw（内核契约），回包经 parseEnvelope 结构化。
//
// wasm 产物 pkg/ 不入 git（.gitignore）：胶水经 `import("@ygg/wasm")` **动态**
// 导入（vite alias → crates/ygg-wasm/pkg/ygg_wasm.js）——产物缺失时动态导入
// 失败，由 App 落「wasm 不可用，回放模式」降级面，不阻塞模块加载。
// ============================================================================

import { installWasiShim } from "@ygg/wasm-shim";
import scenesJsonText from "../../mods/test-adventure/data/scenes.json?raw";
import manifestJsonText from "../../mods/test-adventure/manifest.json?raw";
import modSourceText from "../../mods/test-adventure/mod.lua?raw";
import packJson from "../../mods/test-adventure/pack.json";
import sceneSchemaText from "../../mods/test-adventure/schemas/scene-texts.json?raw";
import { parseEnvelope, type Envelope, type KernelBridge } from "./kernel";

/** pack.json 声明面（本文件只消费这两个键）。 */
interface ModPack {
  entries: Array<{ path: string }>;
  schemaFiles: string[];
}

// SAFETY: pack.json 是 test-adventure 的静态清单（schemaVersion + entries +
// schemaFiles）；键集漂移时 assembleModBundle 的缺文件校验兜底。
const MOD_PACK = packJson as unknown as ModPack;

// ── 纯编解码（单测主靶；零环境依赖，jsdom / 浏览器 / node 同构） ──────────

/** UTF-8 编码（自实现，零依赖）：codePoint 遍历 + 代理对合并；无 BOM。 */
export function encodeUtf8(text: string): Uint8Array {
  const out: number[] = [];
  for (let i = 0; i < text.length; i++) {
    const cp = text.codePointAt(i) ?? 0;
    if (cp > 0xffff) {
      i++; // 代理对占两个 code unit（codePointAt 已合并），跳过低位。
    }
    if (cp < 0x80) {
      out.push(cp);
    } else if (cp < 0x800) {
      out.push(0xc0 | (cp >> 6), 0x80 | (cp & 0x3f));
    } else if (cp < 0x10000) {
      out.push(0xe0 | (cp >> 12), 0x80 | ((cp >> 6) & 0x3f), 0x80 | (cp & 0x3f));
    } else {
      out.push(
        0xf0 | (cp >> 18),
        0x80 | ((cp >> 12) & 0x3f),
        0x80 | ((cp >> 6) & 0x3f),
        0x80 | (cp & 0x3f),
      );
    }
  }
  return new Uint8Array(out);
}

const B64_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/** 字节 → base64（标准字母表 + `=` padding；与壳内 base64_decode 互逆）。 */
export function bytesToBase64(bytes: Uint8Array): string {
  let out = "";
  for (let i = 0; i < bytes.length; i += 3) {
    const b0 = bytes.at(i) ?? 0;
    const b1 = bytes.at(i + 1);
    const b2 = bytes.at(i + 2);
    out += B64_ALPHABET[b0 >> 2];
    out += B64_ALPHABET[((b0 & 0x03) << 4) | ((b1 ?? 0) >> 4)];
    out += b1 === undefined ? "=" : B64_ALPHABET[((b1 & 0x0f) << 2) | ((b2 ?? 0) >> 6)];
    out += b2 === undefined ? "=" : B64_ALPHABET[b2 & 0x3f];
  }
  return out;
}

/** UTF-8 文本 → base64（资源包装载通道：resources_json 值域）。 */
export function utf8ToBase64(text: string): string {
  return bytesToBase64(encodeUtf8(text));
}

// ── Mod 资源包组装（纯函数；resources_json / schemas_json 单测面） ────────

/** YggKernel 构造参数四件套（全部 JSON 文本 / Lua 源码原文）。 */
export interface ModBundle {
  modSource: string;
  manifestJson: string;
  /** schema 文本 JSON 数组（序 = pack.schemaFiles；注册 id 取自各 `$id`）。 */
  schemasJson: string;
  /** `path → base64(bytes)` JSON 对象（path = manifest entry.path 查找键）。 */
  resourcesJson: string;
}

/**
 * 组装 Mod 构造参数：mod.lua / manifest.json 文本经 files 注入（不再硬编码
 * test-adventure 静态文本——静态包与生成 Mod 共用本函数，review round 1）；
 * schema 按 pack.schemaFiles 序取文本、资源按 pack.entries 逐项 base64 编码
 * （UTF-8）。任一声明文件缺供 → Error（装载前快败，不进 wasm 构造期）。
 */
export function assembleModBundle(
  pack: ModPack,
  files: {
    modLua: string;
    manifestJson: string;
    schemaTexts: Record<string, string>;
    resourceTexts: Record<string, string>;
  },
): ModBundle {
  const schemas = pack.schemaFiles.map((rel) => {
    const text = files.schemaTexts[rel];
    if (text === undefined) {
      throw new Error(`mod 资源包：pack.schemaFiles 声明的 \`${rel}\` 未提供文本`);
    }
    return text;
  });
  const resources: Record<string, string> = {};
  for (const entry of pack.entries) {
    const text = files.resourceTexts[entry.path];
    if (text === undefined) {
      throw new Error(`mod 资源包：pack.entries 声明的 \`${entry.path}\` 未提供文本`);
    }
    resources[entry.path] = utf8ToBase64(text);
  }
  return {
    modSource: files.modLua,
    manifestJson: files.manifestJson,
    schemasJson: JSON.stringify(schemas),
    resourcesJson: JSON.stringify(resources),
  };
}

/** test-adventure 静态资源包（模块级一次组装；文件集固定，失败即编程错误）。 */
const MOD_BUNDLE: ModBundle = assembleModBundle(MOD_PACK, {
  modLua: modSourceText,
  manifestJson: manifestJsonText,
  schemaTexts: { "schemas/scene-texts.json": sceneSchemaText },
  resourceTexts: { "data/scenes.json": scenesJsonText },
});

// ── 构造期错误映射（纯函数，单测主靶） ────────────────────────────────────

/** 人读错误文案截断上限（信封 message 可能很长，UI 呈现面截断）。 */
const INIT_ERROR_MAX_CHARS = 240;

/**
 * wasm 初始化失败 → 人读中文文案。构造期 throw 的值有两种形态（B1 契约）：
 * - A1 错误信封 JSON 文本（域失败：ModLoad / 资源包拒绝）→ 提取 code/message；
 * - marshal 定位文案 / 其它 Error（胶水缺失、wasm 实例化失败）→ 原文透出。
 */
export function describeInitError(thrown: unknown): string {
  const text =
    typeof thrown === "string"
      ? thrown
      : thrown instanceof Error
        ? thrown.message
        : String(thrown);
  const envelope = parseErrorEnvelope(text);
  if (envelope) {
    return `内核装载失败（${envelope.code}）：${truncate(envelope.message)}`;
  }
  return `wasm 初始化失败：${truncate(text)}`;
}

/** 域失败信封提取：`{"ok":false,"error":{code,message}}` 形态才认，其余 null。 */
function parseErrorEnvelope(text: string): { code: string; message: string } | null {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return null;
  }
  if (typeof value !== "object" || value === null) {
    return null;
  }
  const obj = value as Record<string, unknown>;
  if (obj.ok !== false) {
    return null;
  }
  const error = obj.error;
  if (typeof error !== "object" || error === null) {
    return null;
  }
  const fields = error as Record<string, unknown>;
  return {
    code: typeof fields.code === "string" ? fields.code : "UNKNOWN",
    message: typeof fields.message === "string" ? fields.message : text,
  };
}

function truncate(text: string): string {
  return text.length <= INIT_ERROR_MAX_CHARS ? text : `${text.slice(0, INIT_ERROR_MAX_CHARS)}…`;
}

/** wasm 内核初始化失败（构造期域失败 / 实例化失败）。message = describeInitError 文案。 */
export class WasmKernelInitError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "WasmKernelInitError";
  }
}

// ── WasmKernel ───────────────────────────────────────────────────────────

/**
 * 真实 wasm 内核（KernelBridge 实现）：方法面全部一行透传到 wasm 导出，
 * 回包 parseEnvelope 结构化（驱动信封契约保证合法；非法回包 = 编程错误，
 * 在此显式抛出而非静默）。
 */
export class WasmKernel implements KernelBridge {
  private constructor(private readonly inner: import("@ygg/wasm").YggKernel) {}

  /**
   * 装载并构造：installWasiShim → init → new YggKernel(bundle…)。
   * bundle 缺省 = test-adventure 静态资源包（MOD_BUNDLE，FR-10 默认路径
   * 保持不变）；生成 Mod 由 mod-loader 组装后传入（FR-10 / ?mod= 入口）。
   * 构造期 throw 一律包装为 WasmKernelInitError（文案经 describeInitError）。
   */
  static async create(bundle: ModBundle = MOD_BUNDLE): Promise<WasmKernel> {
    // 双职责（与 node smoke 同款）：WASI 导入本体已由构建期重写静态直连
    // wasi-shim.js；installWasiShim() 负责补丁 instantiate **捕获模块 memory**
    // ——静态导入路径无法自获得 memory，此调用不可省。
    installWasiShim();
    const ygg = await import("@ygg/wasm");
    await ygg.default();
    let inner: import("@ygg/wasm").YggKernel;
    try {
      inner = new ygg.YggKernel(
        bundle.modSource,
        bundle.manifestJson,
        bundle.schemasJson,
        bundle.resourcesJson,
      );
    } catch (thrown) {
      throw new WasmKernelInitError(describeInitError(thrown));
    }
    return new WasmKernel(inner);
  }

  start(): Envelope {
    return parseEnvelope(this.inner.dispatch("event.start", "0"));
  }

  dispatch(cmd: string, payload: string): Envelope {
    return parseEnvelope(this.inner.dispatch(cmd, payload));
  }

  /** 存档镜像（= dispatch("save","")，壳便捷导出面）。 */
  save(): Envelope {
    return parseEnvelope(this.inner.save());
  }

  /** 读档镜像（= dispatch("load", save_json)，壳便捷导出面）。 */
  load(saveJson: string): Envelope {
    return parseEnvelope(this.inner.load(saveJson));
  }
}

/**
 * 装载真实 wasm 内核（App 唯一消费面）：installWasiShim → init → 构造。
 * bundle 缺省 = MOD_BUNDLE（test-adventure）；生成 Mod 传 mod-loader 的
 * 组装产物。失败 → reject（message = describeInitError 人读文案）。
 */
export async function createWasmKernel(bundle?: ModBundle): Promise<WasmKernel> {
  return WasmKernel.create(bundle ?? MOD_BUNDLE);
}
