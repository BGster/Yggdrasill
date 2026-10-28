#!/usr/bin/env node
// B1 活链验证脚本（非测试套件）：加载 pkg 产物 + WASI shim，装载
// packages/mods/test-adventure（真实资源包三件套 + Lua Mod），dispatch
// "event.start"，打印叙事信封并断言关键面（叙事非空 + 酒馆选择挂起 +
// 存读档往返）。
//
// 用法（仓库根）：
//   scripts/build-wasm.sh            # 先出产物（或手工两步构建）
//   node crates/ygg-wasm/smoke.mjs
// 退出码 0 = 活链通过；非 0 = 任一断言/异常失败（含信封 ok=false）。
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { installWasiShim } from "./wasi-shim.js";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, "..", "..");

/** 带 label 的受控 JSON 解析：非法输入 → 可读 Error（拒绝裸 throw 路径）。 */
function parseJson(text, label) {
  try {
    return JSON.parse(text);
  } catch (err) {
    throw new Error(`${label}: invalid JSON — ${err.message}`);
  }
}

let failed = 0;
const expect = (cond, label) => {
  console.log(`${cond ? "PASS" : "FAIL"}  ${label}`);
  if (!cond) failed++;
};

async function main() {
  // 双职责说明：WASI 导入本体经构建期重写静态直连 wasi-shim.js（见
  // build-wasm.sh）；installWasiShim() 仍需调用——它补丁 instantiate 以
  // **捕获模块 memory**（写内存的 preview-1 调用——clock/environ/fdstat——
  // 需要），静态导入路径无法自获得 memory。
  installWasiShim();
  const ygg = await import(join(here, "pkg", "ygg_wasm.js"));
  // 显式传 bytes：绕开胶水的 fetch 路径（node 无 fetch(file://)）。
  await ygg.default({ module_or_path: await readFile(join(here, "pkg", "ygg_wasm_bg.wasm")) });

  const modDir = join(repoRoot, "packages", "mods", "test-adventure");
  const pack = parseJson(
    await readFile(join(modDir, "pack.json"), "utf8"),
    "packages/mods/test-adventure/pack.json",
  );
  const modSource = await readFile(join(modDir, "mod.lua"), "utf8");
  const manifestJson = await readFile(join(modDir, "manifest.json"), "utf8");
  const schemas = [];
  for (const rel of pack.schemaFiles) {
    schemas.push(await readFile(join(modDir, rel), "utf8"));
  }
  const resources = {};
  for (const entry of pack.entries) {
    resources[entry.path] = (await readFile(join(modDir, entry.path))).toString("base64");
  }

  const kernel = new ygg.YggKernel(
    modSource,
    manifestJson,
    JSON.stringify(schemas),
    JSON.stringify(resources),
  );

  // ① event.start：叙事 + 酒馆选择1 挂起（与 crates/ygg-sandbox/tests/
  //    test_adventure.rs 的 start() 断言同面）。
  const parsed = parseJson(kernel.dispatch("event.start", "0"), "event.start envelope");
  console.log("== envelope ==");
  console.log(JSON.stringify(parsed, null, 2));

  expect(parsed.ok === true, "event.start ok=true");
  expect(Array.isArray(parsed.narrative) && parsed.narrative.length > 0, "narrative non-empty");
  expect(
    parsed.pendingChoice?.prompt === "掌柜把铜提灯推向你：『地窖黑得很，带上它吧？』",
    "pendingChoice = 选择1（酒馆提灯）",
  );
  console.log("== narrative ==");
  for (const line of parsed.narrative) console.log(line);

  // ② 保留命令面：state + save/load 往返。
  const state = parseJson(kernel.state(), "state envelope");
  expect(state.ok === true && typeof state.snapshotBytes === "number", "state ok");
  const save = parseJson(kernel.save(), "save envelope");
  expect(save.ok === true && save.save != null, "save ok");
  const restored = parseJson(kernel.load(JSON.stringify(save.save)), "load envelope");
  expect(restored.ok === true && restored.restored === true, "load roundtrip ok");
}

main()
  .then(() => {
    console.log(failed === 0 ? "== smoke PASSED ==" : `== smoke FAILED (${failed}) ==`);
    process.exit(failed === 0 ? 0 : 1);
  })
  .catch((err) => {
    console.error("== smoke ABORTED ==", err);
    process.exit(1);
  });
