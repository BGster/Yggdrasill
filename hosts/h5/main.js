// H5 零构建宿主主循环：协议握手 → 读取视图列 → 把点击/按键翻译成 command 帧 → 应用 delta 重绘。
// 这里不写任何规则逻辑：可达格、伤害、胜负全在 Lua 内核里，拒绝由内核答 E_RULE。

import initWasm, { YggSession } from "../../crates/ygg-wasm/pkg/ygg_wasm.js";
import * as _wasiShim from "../../crates/ygg-wasm/wasi-shim.js";
import { IMAGE } from "./pack.image.js";
import { cellFromPointer, decodePos, drawBoard, drawComponentTable } from "./render.js";

// 保持对 wasi-shim 的显式模块引用（ygg_wasm.js 内部也绑定到同一份模块）。
void _wasiShim;

const PROTOCOL = "ygg/1";
const CODEC = "json";
const STEP_DT = 16;
// 包标识只是宿主自己的标签（内核只用 pack_hash 判包，pack_id 仅回显在错误文案里）。
const PACK_ID = "ygg:example";

// 命令名常量（仅协议消息名，不承载规则逻辑）。
const CMD_MOVE = "move";
const CMD_ATTACK = "attack";
const CMD_END_TURN = "end_turn";

const TRACE = new URLSearchParams(globalThis.location?.search ?? "").get("trace") === "1";

/**
 * base64 → Uint8Array（标准浏览器 API，零依赖）。
 * @param {string} b64
 * @returns {Uint8Array}
 */
function base64ToBytes(b64) {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

/**
 * Uint8Array → base64（分片避免大数组参数栈溢出）。
 * @param {Uint8Array} bytes
 * @returns {string}
 */
function bytesToBase64(bytes) {
  let bin = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    bin += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(bin);
}

/**
 * 从帧 JSON 中保真提取 64 位无符号整数 `hash` 字符串（避开 JS Number 2^53 精度限制）。
 * @param {string} raw
 * @returns {string | null}
 */
function extractHashLiteral(raw) {
  const m = raw.match(/"hash"\s*:\s*([0-9]+)/);
  return m ? m[1] : null;
}

/**
 * 解包镜像里的棋盘网格资源（仅用于画格底色，不做任何寻路/阻挡判定）。
 * @returns {{ width: number, height: number, cells: number[] }}
 */
function readBoardResource() {
  const b64 = IMAGE.resources?.["data/board.json"];
  if (!b64) return { width: 10, height: 5, cells: new Array(50).fill(0) };
  const text = new TextDecoder().decode(base64ToBytes(b64));
  return JSON.parse(text);
}

/**
 * 读取二进制快照帧里的实体与列初值（用于首帧起屏与 open 读档，按 `ready.components` 对齐）。
 * 快照二进制布局由 `crates/ygg-sim/src/snapshot.rs` 定义。
 * @param {Uint8Array} bytes
 * @param {Array<{ id: number, name: string, kind: string }>} components
 * @returns {Map<number, Map<number, unknown>>}
 */
export function decodeSnapshotEntities(bytes, components) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const dec = new TextDecoder();
  let off = 8 + 2 + 2; // magic(8) + fmt(2) + flags(2)
  off += 8; // tick
  off += 8; // next_entity

  const compN = view.getUint32(off, true);
  off += 4;
  const snapComps = [];
  for (let i = 0; i < compN; i++) {
    const nlen = view.getUint16(off, true);
    off += 2;
    const name = dec.decode(bytes.subarray(off, off + nlen));
    off += nlen;
    const ty = view.getUint8(off);
    off += 1;
    off += 1; // frac_bits
    snapComps.push({ name, ty });
  }

  const resN = view.getUint32(off, true);
  off += 4 + resN * 16;
  off += 8; // rng_seed
  const rngN = view.getUint32(off, true);
  off += 4;
  for (let i = 0; i < rngN; i++) {
    const nlen = view.getUint16(off, true);
    off += 2 + nlen + 16;
  }

  const cidByName = new Map(components.map((c) => [c.name, c.id]));
  const archN = view.getUint32(off, true);
  off += 4;
  /** @type {Map<number, Map<number, unknown>>} */
  const entities = new Map();

  for (let a = 0; a < archN; a++) {
    const compCnt = view.getUint16(off, true);
    off += 2;
    const idxs = [];
    for (let k = 0; k < compCnt; k++) {
      idxs.push(view.getUint16(off, true));
      off += 2;
    }
    const rowN = view.getUint32(off, true);
    off += 4;
    const ids = [];
    for (let r = 0; r < rowN; r++) {
      const eid = Number(view.getBigUint64(off, true));
      ids.push(eid);
      if (!entities.has(eid)) entities.set(eid, new Map());
      off += 8;
    }
    for (const idx of idxs) {
      const { name, ty } = snapComps[idx];
      const cid = cidByName.get(name);
      if (ty === 5) continue; // Tag 无列
      for (let r = 0; r < rowN; r++) {
        let val;
        if (ty === 1) {
          val = Number(view.getBigInt64(off, true));
          off += 8;
        } else if (ty === 4 || ty === 6) {
          val = Number(view.getBigUint64(off, true));
          off += 8;
        } else if (ty === 3) {
          val = view.getUint8(off) !== 0;
          off += 1;
        } else if (ty === 2) {
          const len = view.getUint32(off, true);
          off += 4;
          val = dec.decode(bytes.subarray(off, off + len));
          off += len;
        }
        if (cid !== undefined) entities.get(ids[r]).set(cid, val);
      }
    }
  }
  return entities;
}

/**
 * 解包协议 `Value` 外部标签（{"int":n} / {"uint":n} / {"str":s} / {"bool":b}）。
 * @param {Record<string, unknown>} tagged
 * @returns {unknown}
 */
function unwrapValue(tagged) {
  if (!tagged || typeof tagged !== "object") return tagged;
  if ("int" in tagged) return tagged.int;
  if ("uint" in tagged) return tagged.uint;
  if ("str" in tagged) return tagged.str;
  if ("bool" in tagged) return tagged.bool;
  return tagged;
}

/**
 * 把 `delta.changed` 增量合并进现有实体表（不做整表重读）。
 * @param {Map<number, Map<number, unknown>>} entities
 * @param {Array<{ eid: number, comps: Array<{ cid: number, values: unknown[] }> }>} changed
 */
export function applyDelta(entities, changed) {
  for (const row of changed) {
    let cols = entities.get(row.eid);
    if (!cols) {
      cols = new Map();
      entities.set(row.eid, cols);
    }
    for (const cell of row.comps) {
      cols.set(cell.cid, unwrapValue(cell.values[0]));
    }
  }
}

/**
 * 从快照同步移除已离场的实体 ID（`delta` 本身只携带仍存活的被写单元格）。
 * @param {Map<number, Map<number, unknown>>} entities
 * @param {Uint8Array} snapshotBytes
 */
function pruneMissingEntities(entities, snapshotBytes) {
  const view = new DataView(snapshotBytes.buffer, snapshotBytes.byteOffset, snapshotBytes.byteLength);
  let off = 12 + 16;
  const compN = view.getUint32(off, true);
  off += 4;
  const snapTypes = [];
  for (let i = 0; i < compN; i++) {
    const nlen = view.getUint16(off, true);
    off += 2 + nlen;
    snapTypes.push(view.getUint8(off));
    off += 2;
  }
  const resN = view.getUint32(off, true);
  off += 4 + resN * 16;
  off += 8;
  const rngN = view.getUint32(off, true);
  off += 4;
  for (let i = 0; i < rngN; i++) {
    const nlen = view.getUint16(off, true);
    off += 2 + nlen + 16;
  }
  const archN = view.getUint32(off, true);
  off += 4;
  const alive = new Set();
  for (let a = 0; a < archN; a++) {
    const compCnt = view.getUint16(off, true);
    off += 2;
    const idxs = [];
    for (let k = 0; k < compCnt; k++) {
      idxs.push(view.getUint16(off, true));
      off += 2;
    }
    const rowN = view.getUint32(off, true);
    off += 4;
    for (let r = 0; r < rowN; r++) {
      alive.add(Number(view.getBigUint64(off, true)));
      off += 8;
    }
    for (const idx of idxs) {
      const ty = snapTypes[idx];
      if (ty === 5) continue;
      if (ty === 1 || ty === 4 || ty === 6) off += rowN * 8;
      else if (ty === 3) off += rowN;
      else if (ty === 2) {
        for (let r = 0; r < rowN; r++) {
          const len = view.getUint32(off, true);
          off += 4 + len;
        }
      }
    }
  }
  for (const eid of [...entities.keys()]) {
    if (!alive.has(eid)) entities.delete(eid);
  }
}

async function boot() {
  const els = {
    canvas: /** @type {HTMLCanvasElement} */ (document.getElementById("board")),
    meta: document.getElementById("session-meta"),
    status: document.getElementById("status"),
    selected: document.getElementById("selected-info"),
    compHead: /** @type {HTMLTableRowElement} */ (document.getElementById("comp-head")),
    compBody: /** @type {HTMLTableSectionElement} */ (document.getElementById("comp-body")),
    btnStep: /** @type {HTMLButtonElement} */ (document.getElementById("btn-step")),
    btnEndTurn: /** @type {HTMLButtonElement} */ (document.getElementById("btn-end-turn")),
    btnSnapshot: /** @type {HTMLButtonElement} */ (document.getElementById("btn-snapshot")),
    fileOpen: /** @type {HTMLInputElement} */ (document.getElementById("file-open")),
    traceBox: document.getElementById("trace-box"),
    traceLog: document.getElementById("trace-log"),
  };

  if (TRACE) els.traceBox.classList.add("active");

  /** @type {{
   *   session: YggSession | null,
   *   board: { width: number, height: number, cells: number[] },
   *   components: Array<{ id: number, name: string, kind: string }>,
   *   entities: Map<number, Map<number, unknown>>,
   *   selectedId: number | null,
   *   tick: number,
   *   hash: string,
   * }} */
  const state = {
    session: null,
    board: readBoardResource(),
    components: [],
    entities: new Map(),
    selectedId: null,
    tick: 0,
    hash: "0",
  };

  function setStatus(text, kind = "ok") {
    els.status.textContent = text;
    els.status.className = `status ${kind}`;
  }

  function recordTrace(label, deltaFrame) {
    if (!TRACE) return;
    const compName = new Map(state.components.map((c) => [c.id, c.name]));
    const summary = deltaFrame.changed.map((row) => ({
      eid: row.eid,
      comps: row.comps.map((c) => ({
        name: compName.get(c.cid) ?? `#${c.cid}`,
        val: unwrapValue(c.values[0]),
      })),
    }));
    console.log(`[trace] ${label} tick=${deltaFrame.tick} changed(${summary.length}):`, deltaFrame.changed);
    const line = `[${label}] tick=${deltaFrame.tick} changed=${summary.length}\n${JSON.stringify(summary)}\n`;
    els.traceLog.textContent = line + els.traceLog.textContent;
  }

  function render() {
    els.meta.textContent = `tick ${state.tick} | hash ${state.hash} | 实体 ${state.entities.size}`;
    els.selected.textContent = state.selectedId === null ? "选中：无" : `选中：#${state.selectedId}`;
    drawBoard(els.canvas, {
      board: state.board,
      components: state.components,
      entities: state.entities,
      selectedId: state.selectedId,
    });
    drawComponentTable(
      els.compHead,
      els.compBody,
      {
        components: state.components,
        entities: state.entities,
        selectedId: state.selectedId,
      },
      (eid) => {
        state.selectedId = eid;
        render();
      },
    );
  }

  /**
   * 发送一条协议 JSON 并解析返回帧流，同时保真取出 `hash` 字面量。
   * @param {Record<string, unknown>} msg
   * @returns {{ frames: any[], hashLiteral: string | null }}
   */
  function send(msg) {
    const raw = state.session.handle(JSON.stringify(msg));
    return { frames: JSON.parse(raw), hashLiteral: extractHashLiteral(raw) };
  }

  /** @returns {number | undefined} */
  function posCid() {
    return state.components.find((c) => c.name === "pos")?.id;
  }

  /**
   * 查找当前落在格子 (x, y) 上的实体 ID（仅查表，不做规则推理）。
   * @param {number} x
   * @param {number} y
   * @returns {number | null}
   */
  function entityAt(x, y) {
    const cid = posCid();
    if (cid === undefined) return null;
    for (const [eid, cols] of state.entities) {
      const pos = cols.get(cid);
      if (typeof pos !== "number") continue;
      const p = decodePos(pos);
      if (p.x === x && p.y === y) return eid;
    }
    return null;
  }

  function applyFrames(label, frames, hashLiteral, allowPrune = false) {
    for (const frame of frames) {
      if (frame.type === "error") {
        setStatus(`[${frame.code}] ${frame.msg}`, "error");
        return false;
      }
      if (frame.type === "ready") {
        state.tick = frame.tick;
        state.hash = hashLiteral ?? String(frame.hash);
        state.components = frame.components;
        state.entities = decodeSnapshotEntities(state.session.snapshot(), state.components);
        if (state.selectedId !== null && !state.entities.has(state.selectedId)) {
          state.selectedId = null;
        }
      } else if (frame.type === "events") {
        state.tick = frame.tick;
        state.hash = hashLiteral ?? String(frame.hash);
      } else if (frame.type === "delta") {
        state.tick = frame.tick;
        applyDelta(state.entities, frame.changed);
        if (allowPrune) pruneMissingEntities(state.entities, state.session.snapshot());
        if (state.selectedId !== null && !state.entities.has(state.selectedId)) {
          state.selectedId = null;
        }
        recordTrace(label, frame);
      }
    }
    return true;
  }

  function doStep(label = "step") {
    const { frames, hashLiteral } = send({ type: "step", ticks: 1, dt: STEP_DT });
    const ok = applyFrames(label, frames, hashLiteral, true);
    if (ok) {
      const delta = frames.find((f) => f.type === "delta");
      setStatus(`tick ${state.tick}（delta 更新 ${delta ? delta.changed.length : 0} 个实体）`, "ok");
    }
    render();
    return ok;
  }

  function doMove(id, dx, dy) {
    const { frames, hashLiteral } = send({
      type: "command",
      name: CMD_MOVE,
      args: { id, dx, dy },
    });
    if (!applyFrames(CMD_MOVE, frames, hashLiteral, false)) {
      render();
      return;
    }
    const { frames: stepFrames, hashLiteral: stepHash } = send({
      type: "step",
      ticks: 1,
      dt: STEP_DT,
    });
    if (applyFrames("step", stepFrames, stepHash, true)) {
      setStatus(`命令 ${CMD_MOVE}(#${id}, dx=${dx}, dy=${dy}) 已落地并推进至 tick ${state.tick}`, "ok");
    }
    render();
  }

  function doAttack(attacker, target) {
    const { frames, hashLiteral } = send({
      type: "command",
      name: CMD_ATTACK,
      args: { attacker, target },
    });
    if (applyFrames(CMD_ATTACK, frames, hashLiteral, false)) {
      setStatus(`命令 ${CMD_ATTACK}(#${attacker} -> #${target}) 已生效（由 delta 更新）`, "ok");
    }
    render();
  }

  // 1. 初始化 WASM 并用 pack.image.js 启动会话
  // 用字节（而非 URL/Response）喂 initSync 路径：不依赖服务端的 wasm MIME 类型，也就不会
  // 因为 `python3 -m http.server` 之类静态服务少了 `application/wasm` 而在控制台报 warn。
  const wasmBytes = await (await fetch(new URL("../../crates/ygg-wasm/pkg/ygg_wasm_bg.wasm", import.meta.url))).arrayBuffer();
  await initWasm({ module_or_path: wasmBytes });
  const packId = PACK_ID;
  const imageBytes = new TextEncoder().encode(JSON.stringify(IMAGE));
  state.session = new YggSession(imageBytes);

  // 2. 握手：先用零指纹取内核真实 pack_hash，再发对指纹拿 ready
  const probe = send({
    type: "hello",
    protocol: PROTOCOL,
    codec: CODEC,
    pack_id: packId,
    pack_hash: "0".repeat(64),
  });
  const errFrame = probe.frames[0];
  const hashMatch = errFrame?.type === "error" ? errFrame.msg.match(/[0-9a-f]{64}/) : null;
  if (!hashMatch) {
    setStatus(`握手探测失败：${JSON.stringify(probe.frames)}`, "error");
    return;
  }

  const readyResp = send({
    type: "hello",
    protocol: PROTOCOL,
    codec: CODEC,
    pack_id: packId,
    pack_hash: hashMatch[0],
  });
  if (!applyFrames("ready", readyResp.frames, readyResp.hashLiteral, false)) return;

  // 默认选中第一个实体，方便直接按键或点击邻格测试
  const firstId = [...state.entities.keys()].sort((a, b) => a - b)[0] ?? null;
  state.selectedId = firstId;
  setStatus(`就绪：tick ${state.tick}，pack=${packId}，点击棋盘格或按方向键发送命令`, "ok");
  render();

  // 3. 棋盘点击：选实体 / 发 move / 发 attack（不做任何规则判定）
  els.canvas.addEventListener("click", (ev) => {
    const cell = cellFromPointer(els.canvas, state.board, ev.clientX, ev.clientY);
    if (!cell) return;
    const hit = entityAt(cell.x, cell.y);

    if (state.selectedId === null) {
      if (hit !== null) {
        state.selectedId = hit;
        setStatus(`已选中 #${hit}`, "ok");
        render();
      }
      return;
    }

    if (hit === state.selectedId) {
      state.selectedId = null;
      setStatus("已取消选中", "ok");
      render();
      return;
    }

    if (hit !== null) {
      // 点击另一实体：若用户按住 Shift 则仅切换选中，否则直接发 attack 命令交给内核判定。
      if (ev.shiftKey) {
        state.selectedId = hit;
        setStatus(`已切换选中 #${hit}`, "ok");
        render();
        return;
      }
      doAttack(state.selectedId, hit);
      return;
    }

    // 点击空格或墙格：计算坐标差 dx/dy，原样发 move 命令交给内核判定（越界/撞墙/跨格由内核答 E_RULE）。
    const cid = posCid();
    const curPos = cid !== undefined ? state.entities.get(state.selectedId)?.get(cid) : undefined;
    if (typeof curPos !== "number") return;
    const origin = decodePos(curPos);
    doMove(state.selectedId, cell.x - origin.x, cell.y - origin.y);
  });

  // 4. 键盘方向键 / WASD：对选中单位发一格位移的 move 命令
  globalThis.addEventListener("keydown", (ev) => {
    if (state.selectedId === null) return;
    const deltaByKey = {
      ArrowUp: [0, -1],
      KeyW: [0, -1],
      ArrowDown: [0, 1],
      KeyS: [0, 1],
      ArrowLeft: [-1, 0],
      KeyA: [-1, 0],
      ArrowRight: [1, 0],
      KeyD: [1, 0],
    };
    const d = deltaByKey[ev.code];
    if (!d) return;
    ev.preventDefault();
    doMove(state.selectedId, d[0], d[1]);
  });

  // 5. 按钮：Step / End turn / Snapshot / Open snapshot
  els.btnStep.addEventListener("click", () => {
    doStep("step");
  });

  els.btnEndTurn.addEventListener("click", () => {
    const { frames, hashLiteral } = send({
      type: "command",
      name: CMD_END_TURN,
      args: {},
    });
    if (applyFrames(CMD_END_TURN, frames, hashLiteral, false)) {
      setStatus(`命令 ${CMD_END_TURN} 已生效（tick ${state.tick}）`, "ok");
    }
    render();
  });

  els.btnSnapshot.addEventListener("click", () => {
    const { frames } = send({ type: "snapshot", direction: "request" });
    const reply = frames[0];
    if (!reply || reply.type !== "snapshot" || reply.direction !== "reply") {
      setStatus(`快照失败：${JSON.stringify(frames)}`, "error");
      return;
    }
    const rawBytes = base64ToBytes(reply.bytes);
    const blob = new Blob([rawBytes], { type: "application/octet-stream" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = "snapshot.bin";
    a.click();
    URL.revokeObjectURL(url);
    setStatus(`已导出 snapshot.bin（tick ${reply.tick}，${rawBytes.length} bytes）`, "ok");
  });

  els.fileOpen.addEventListener("change", async () => {
    const file = els.fileOpen.files?.[0];
    els.fileOpen.value = "";
    if (!file) return;
    const buf = new Uint8Array(await file.arrayBuffer());
    const { frames, hashLiteral } = send({
      type: "open",
      bytes: bytesToBase64(buf),
    });
    if (applyFrames("open", frames, hashLiteral, false)) {
      setStatus(`已恢复快照：tick ${state.tick} | hash ${state.hash}`, "ok");
    }
    render();
  });
}

if (typeof document !== "undefined") {
  boot().catch((err) => {
    const el = document.getElementById("status");
    if (el) {
      el.textContent = `启动失败：${err instanceof Error ? err.message : String(err)}`;
      el.className = "status error";
    }
    console.error(err);
  });
}
