#!/usr/bin/env node
// WASM 双运行时金值验证车道（M3.4b）：
// 零 npm 依赖，只用 node:fs/promises + node:path + node:url。
//
// 两种模式：
//   - 默认模式（`node scripts/wasm-lane.mjs [--image <路径>]`）：
//     1. 读 `crates/ygg-wasm/pkg/ygg_wasm.js` + `ygg_wasm_bg.wasm`，`initSync`；
//     2. 装载镜像（默认 `hosts/h5/pack.image.js`，也接受 `--image <json或js路径>`）；
//     3. `hello` 错指纹 → 校验 `E_PACK` 并从中提取真实 `pack_hash`；
//     4. `hello` 对指纹 → 校验 `ready`；
//     5. 跑 `packs/example/golden.json` 指定的 seed / ticks：
//        a) 握手时把 `golden.seed` 作为 `hello.seed` 交给内核（创世重播），
//           再逐 tick 发 `step{ticks:1,dt:1}`，逐条核对 `events.hash` == `golden.hashes[i]`，
//           同时核对每步回执第二帧是 `delta`；
//        b) 再发一条完整 `replay` 帧（带 `golden.seed` 与全部 50 帧哈希）校验 `replay_result.ok === true`；
//     6. 全等打印 50 条逐 tick 哈希与 `ok`；任一不符打印 tick / 期望值 / 实际值并 exit 1。
//   - 冒烟模式（`node scripts/wasm-lane.mjs --smoke`，供 `scripts/build-wasm.sh` 末尾调用）：
//     用内联最小镜像跑 `hello(错指纹) → error E_PACK` + `hello(对指纹) → ready`，
//     不依赖外部镜像文件，确保 wasm 产物与 wasi-shim 内存绑定在构建期立刻验收。
//
// 当 `crates/ygg-wasm/pkg/` 不存在时打印 skipped 并以 0 退出（不阻断未建 wasm 的环境）。

import { access, readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const PKG_DIR = path.join(REPO_ROOT, "crates", "ygg-wasm", "pkg");
const GLUE_PATH = path.join(PKG_DIR, "ygg_wasm.js");
const WASM_PATH = path.join(PKG_DIR, "ygg_wasm_bg.wasm");
const GOLDEN_PATH = path.join(REPO_ROOT, "packs", "example", "golden.json");
const DEFAULT_IMAGE_PATH = path.join(REPO_ROOT, "hosts", "h5", "pack.image.js");

const SMOKE_IMAGE = {
  modSource: "world.component{ name = 'hp', kind = 'u8' }\nworld.seal()\n",
  manifest: "",
};

const ZERO_HASH = "0".repeat(64);

function parseArgs(argv) {
  let smoke = false;
  let imagePath = DEFAULT_IMAGE_PATH;
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--smoke") {
      smoke = true;
    } else if (arg === "--image") {
      const next = argv[i + 1];
      if (!next) {
        throw new Error("--image 需要指定文件路径");
      }
      imagePath = path.resolve(REPO_ROOT, next);
      i += 1;
    } else if (arg === "-h" || arg === "--help") {
      console.log("用法: node scripts/wasm-lane.mjs [--smoke] [--image <pack.image.js|json>]");
      process.exit(0);
    } else {
      throw new Error(`未知参数：${arg}`);
    }
  }
  return { smoke, imagePath };
}

async function exists(filePath) {
  try {
    await access(filePath);
    return true;
  } catch {
    return false;
  }
}

/**
 * 解析会话回执 JSON 数组，同时把任意层的 `hash` 字段按原始十进制文本保留为字符串，
 * 避免 JavaScript IEEE-754 Number 对超过 2^53 - 1 的 u64 state hash 舍入丢精度。
 */
function parseFramesPreservingHash(raw) {
  return JSON.parse(raw, (key, value, context) => {
    if (key === "hash" && typeof value === "number") {
      if (context && typeof context.source === "string") {
        return context.source;
      }
      throw new Error("当前 Node 版本不支持 JSON.parse reviver context.source，无法精确读取 u64 hash");
    }
    return value;
  });
}

/** 把任意合法镜像文件（`.js`/`.mjs` ES module 或 `.json`）转为 UTF-8 字节送进 YggSession。 */
async function loadImageBytes(imagePath) {
  if (!(await exists(imagePath))) {
    throw new Error(
      `镜像文件缺失：${path.relative(REPO_ROOT, imagePath)}（先跑 cargo run -q --example image -p ygg-host -- packs/example --out hosts/h5/pack.image.js）`,
    );
  }
  if (imagePath.endsWith(".js") || imagePath.endsWith(".mjs")) {
    const mod = await import(pathToFileURL(imagePath).href);
    if (!mod || typeof mod.IMAGE !== "object" || mod.IMAGE === null) {
      throw new Error(`镜像模块 ${imagePath} 未导出对象 IMAGE`);
    }
    return new TextEncoder().encode(JSON.stringify(mod.IMAGE));
  }
  return new Uint8Array(await readFile(imagePath));
}

/**
 * 握手两步：错指纹拿真实 `pack_hash`，再用真实 `pack_hash` 换回 `ready`。
 * `seed` 是创世重播的种子：缺省时内核沿用默认种子；给了就在进入 ready 之前
 * 按该种子重建干净世界谱系（`hello.seed`，见 `crates/ygg-host/src/proto.rs`）。
 */
function handshake(session, packId, seed) {
  const badReply = parseFramesPreservingHash(
    session.handle(
      JSON.stringify({
        type: "hello",
        protocol: "ygg/1",
        codec: "json",
        pack_id: packId,
        pack_hash: ZERO_HASH,
      }),
    ),
  );
  if (badReply.length !== 1 || badReply[0].type !== "error" || badReply[0].code !== "E_PACK") {
    throw new Error(`错指纹 hello 期望单帧 error/E_PACK，实际收到：${JSON.stringify(badReply)}`);
  }
  const match = String(badReply[0].msg ?? "").match(/[0-9a-f]{64}/);
  if (!match) {
    throw new Error(`无法从 E_PACK 报文提取真实 pack_hash：${badReply[0].msg}`);
  }
  const packHash = match[0];

  const readyReply = parseFramesPreservingHash(
    session.handle(
      JSON.stringify({
        type: "hello",
        protocol: "ygg/1",
        codec: "json",
        pack_id: packId,
        pack_hash: packHash,
        ...(seed !== undefined ? { seed } : {}),
      }),
    ),
  );
  if (readyReply.length !== 1 || readyReply[0].type !== "ready") {
    throw new Error(`对指纹 hello 期望单帧 ready，实际收到：${JSON.stringify(readyReply)}`);
  }
  return { packHash, ready: readyReply[0] };
}

async function main() {
  const { smoke, imagePath } = parseArgs(process.argv.slice(2));

  if (!(await exists(GLUE_PATH)) || !(await exists(WASM_PATH))) {
    console.log("skipped: crates/ygg-wasm/pkg 缺失（先跑 scripts/build-wasm.sh）");
    process.exit(0);
  }

  const [{ initSync, YggSession }, wasmBytes] = await Promise.all([
    import(pathToFileURL(GLUE_PATH).href),
    readFile(WASM_PATH),
  ]);
  initSync({ module: wasmBytes });

  if (smoke) {
    const smokeBytes = new TextEncoder().encode(JSON.stringify(SMOKE_IMAGE));
    const session = new YggSession(smokeBytes);
    const { packHash, ready } = handshake(session, "ygg:smoke");
    console.log(
      `ok (smoke): pack_hash=${packHash.slice(0, 12)}... tick=${ready.tick} sealed=${ready.sealed} components=${ready.components.length}`,
    );
    return;
  }

  const [imageBytes, goldenRaw] = await Promise.all([
    loadImageBytes(imagePath),
    readFile(GOLDEN_PATH, "utf8"),
  ]);
  const golden = JSON.parse(goldenRaw);
  const { seed, ticks, hashes } = golden;
  if (!Array.isArray(hashes) || hashes.length !== ticks) {
    throw new Error(`golden.json 形状异常：ticks=${ticks}, hashes.length=${hashes?.length}`);
  }

  const session = new YggSession(imageBytes);
  const { packHash, ready } = handshake(session, "ygg:example", seed);
  if (ready.tick !== 0) {
    throw new Error(`创世重播后世界应停在 tick 0，实际 tick=${ready.tick}`);
  }

  console.log(
    `wasm-lane: pack_hash=${packHash.slice(0, 12)}... seed=${seed} ticks=${ticks} components=${ready.components.length}`,
  );

  for (let i = 0; i < ticks; i += 1) {
    const tick = i + 1;
    const expected = String(hashes[i]);
    const frames = parseFramesPreservingHash(
      session.handle(JSON.stringify({ type: "step", ticks: 1, dt: 1 })),
    );
    if (frames.length !== 2 || frames[0].type !== "events" || frames[1].type !== "delta") {
      console.error(`tick ${tick} 回执帧形状异常：${JSON.stringify(frames)}`);
      process.exit(1);
    }
    const actual = String(frames[0].hash);
    if (frames[0].tick !== tick || actual !== expected) {
      console.error(
        `FAIL at tick ${tick}: expected=${expected} actual=${actual} (reported_tick=${frames[0].tick})`,
      );
      process.exit(1);
    }
    console.log(`  tick ${String(tick).padStart(2, "0")}: ${actual} == golden`);
  }

  // 再用 `replay` 帧独立复算同一份 seed + 50 帧金值，交叉验证内核 fork_with_seed 路径。
  const replayFramesJson = hashes
    .map((h, i) => `{"tick":${i},"dt":1,"ops":[],"hash":${h}}`)
    .join(",");
  const replayRaw = `{"type":"replay","log":{"header":{"seed":${seed},"start_tick":0},"frames":[${replayFramesJson}]}}`;
  const replayReply = parseFramesPreservingHash(session.handle(replayRaw));
  if (replayReply.length !== 1 || replayReply[0].type !== "replay_result" || !replayReply[0].ok) {
    console.error(`FAIL replay: ${JSON.stringify(replayReply)}`);
    process.exit(1);
  }

  console.log(`ok: wasm 逐 tick (${ticks}/${ticks}) + replay 全等 packs/example/golden.json`);
}

main().catch((err) => {
  console.error(`wasm-lane 失败：${err instanceof Error ? err.message : String(err)}`);
  process.exit(1);
});
