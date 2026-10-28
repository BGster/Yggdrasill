#!/usr/bin/env node
// FR-9 wasm 侧确定性验证（M1 C2，非测试套件）：把 native 探针
// crates/ygg-sandbox/tests/probe_determinism.rs::probe_golden_vector 钉死的
// golden-vector 四元组（Mod + 事件序列 + 期望叙事 + 期望状态哈希，fixtures/
// golden_vector.json 编译期嵌入 Rust 测试的**同一份文件**）在 node + wasm
// 内核上原样重放，比较壳导出 state_hash()（= native ygg_sandbox::hash::
// state_hash 同一函数）与 fixture 的 expected_state_hash。
//
// 判定（全部通过才算 FR-9 wasm 等价）：
//   ① wasm 内双跑（run-a/run-b）叙事与哈希一致（同进程稳定性，镜像 native
//      探针的双跑口径）；
//   ② 叙事逐事件等于 fixture expected_narrative；
//   ③ 状态哈希等于 fixture expected_state_hash（native 钉死基线，哈希相等
//      即 canonical bytes 逐字节相等 → native == wasm）。
//
// 失败处置：打印两个哈希 + 首个分叉信封（事件序号 + 原文），退出码 1——
// 平台间分叉是**真实确定性破口**，不得静默改基线（fixture 口径见探针头注）。
//
// 用法（仓库根）：
//   WASI_SDK_PATH=<path> ./scripts/build-wasm.sh   # 先出产物
//   node crates/ygg-wasm/verify-determinism.mjs
// 退出码 0 = FR-9 wasm 侧哈希等价成立；非 0 = 任一判定失败/异常。
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { installWasiShim } from "./wasi-shim.js";

const here = dirname(fileURLToPath(import.meta.url));

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

/** 单轮完整回放：构造 → 逐事件 dispatch → state_hash。返回叙事/哈希/信封留痕。 */
async function runOnce(label, modSource, sequence) {
  const ygg = await import(join(here, "pkg", "ygg_wasm.js"));
  const kernel = new ygg.YggKernel(modSource, "", "[]", "{}");
  const narratives = [];
  const envelopes = [];
  for (const step of sequence) {
    const raw = kernel.dispatch(step.cmd, step.payload);
    const parsed = parseJson(raw, `${label} envelope (${step.cmd})`);
    expect(parsed.ok === true, `${label}: ${step.cmd}(${step.payload}) ok=true`);
    narratives.push(parsed.narrative ?? []);
    envelopes.push(raw);
  }
  const hashReply = kernel.state_hash();
  const hashParsed = parseJson(hashReply, `${label} state_hash envelope`);
  expect(
    hashParsed.ok === true && typeof hashParsed.stateHash === "string",
    `${label}: state_hash envelope ok=true`,
  );
  return { narratives, hash: hashParsed.stateHash ?? null, envelopes, hashReply };
}

/** 首个分叉点定位：返回首个 i 使两轮叙事不同；全等返回 -1。 */
function firstNarrativeDivergence(a, b) {
  for (let i = 0; i < Math.min(a.length, b.length); i++) {
    if (JSON.stringify(a[i]) !== JSON.stringify(b[i])) return i;
  }
  return a.length === b.length ? -1 : Math.min(a.length, b.length);
}

async function main() {
  // 双职责说明同 smoke.mjs：WASI 导入已构建期静态直连，installWasiShim 负责
  // 补丁 instantiate 以捕获模块 memory（写内存的 preview-1 调用需要）。
  installWasiShim();
  const ygg = await import(join(here, "pkg", "ygg_wasm.js"));
  await ygg.default({
    module_or_path: await readFile(join(here, "pkg", "ygg_wasm_bg.wasm")),
  });

  const fixture = parseJson(
    await readFile(join(here, "..", "ygg-sandbox", "tests", "fixtures", "golden_vector.json"), "utf8"),
    "golden_vector.json",
  );
  const expectedHash = fixture.expected_state_hash;
  console.log(`native fixture hash : ${expectedHash}`);

  // ① wasm 内双跑（同 Mod + 同事件序列，镜像 native 探针 run-a/run-b）。
  const a = await runOnce("run-a", fixture.mod_source, fixture.event_sequence);
  const b = await runOnce("run-b", fixture.mod_source, fixture.event_sequence);

  console.log(`wasm run-a hash     : ${a.hash}`);
  console.log(`wasm run-b hash     : ${b.hash}`);

  const divA = firstNarrativeDivergence(a.narratives, b.narratives);
  expect(divA === -1, "run-a/run-b narrative stability (wasm double-run)");
  expect(a.hash !== null && a.hash === b.hash, "run-a/run-b hash stability (wasm double-run)");

  // ② 叙事逐事件等于 fixture。
  expect(
    JSON.stringify(a.narratives) === JSON.stringify(fixture.expected_narrative),
    "narrative equals fixture expected_narrative",
  );

  // ③ 哈希等于 native 钉死基线（FR-9 本体判定）。
  expect(a.hash === expectedHash, "state hash equals fixture expected_state_hash (native==wasm)");

  // 失败时定位首个分叉信封（事件序号 + 原文），供主会话 triage。
  const divX = firstNarrativeDivergence(a.narratives, fixture.expected_narrative);
  if (divX !== -1) {
    console.error(`-- first narrative divergence at event #${divX + 1} --`);
    console.error(`wasm envelope : ${a.envelopes[divX]}`);
    console.error(`fixture narrative #${divX + 1}: ${JSON.stringify(fixture.expected_narrative[divX])}`);
  }
  if (a.hash !== expectedHash) {
    console.error("-- hash divergence (FR-9 breach candidate; do NOT re-pin silently) --");
    console.error(`native fixture : ${expectedHash}`);
    console.error(`wasm computed  : ${a.hash}`);
  }

  console.log(
    failed === 0
      ? "== verify-determinism PASSED: FR-9 native/wasm hash equivalence =="
      : `== verify-determinism FAILED (${failed}) ==`,
  );
  process.exit(failed === 0 ? 0 : 1);
}

main().catch((err) => {
  console.error("== verify-determinism ABORTED ==", err);
  process.exit(1);
});
