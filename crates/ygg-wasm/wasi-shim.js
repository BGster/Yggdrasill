// WASI preview-1 import shim for the ygg-wasm module（B1）。
//
// 为什么需要：mlua wasm32-unknown-unknown 链路按 wasi C runtime 链接
// （wasi-sdk sysroot，mlua-rs/mlua docs/wasm32 recipe）——模块 import
// wasi_snapshot_preview1，宿主（浏览器 / node / 小游戏 JS）必须逐个兑现。
// 上游无现成成品 shim（mlua#742 实证可行性但未发布），本文件为内核
// 实测 import 面的最小实现。
//
// 实测 import 面（pkg/ygg_wasm_bg.wasm，2026-10-28）：environ_get/_sizes_get、
// clock_time_get、fd_close/_fdstat_get/_fdstat_set_flags/_prestat_get/
// _prestat_dir_name/_read/_renumber/_seek/_write、path_open/
// _remove_directory/_rename/_unlink_file、proc_exit、random_get —— 全部来自
// wasi-libc 初始化 / stdio / preopen 枚举路径；内核语义面（Lua 沙箱裁剪
// stdlib、无 IO）不触达其中任何文件类调用。
//
// 语义：无文件系统（path_* 一律 ENOSYS）、无 preopen（fd_prestat_get 返回
// EBADF 即终止 wasi-libc 的 preopen 枚举）、stdin 读 EBADF、stdout/stderr
// 转发 console（调试面）、时钟/随机兑现 import 但**绝不进入内核状态**
// （确定性锚：luai_makeseed 构建期常量 + os 库剔除 + math.random 内核种子
// 注入）。
//
// 消费方式（二选一，smoke 与默认构建走 ①）：
// ① 静态导入直连（scripts/build-wasm.sh 把 pkg 胶水的 bare specifier
//   `"wasi_snapshot_preview1"` 重写为 `../wasi-shim.js`——本文件以命名导出
//   承载全部函数，namespace import 即可被 WebAssembly 按属性解析）；
// ② 保留 bare specifier 的胶水：宿主用 import map 把
//   `"wasi_snapshot_preview1"` 映射到本文件，或用 installWasiShim() 打补丁
//   （合并进 instantiate 的 importObject——适配 importObject 路径的胶水）。
//
// memory 捕获：preview-1 写内存调用（clock/environ/fd_write 等）通过
// runtime.memory 惰性取用；`__wbindgen_start` 由胶水在 instantiate 完成
// 后调用，先于任何 wasi 调用——捕获时序安全。

const ERRNO_SUCCESS = 0;
const ERRNO_BADF = 8;
const ERRNO_NOSYS = 52;

/** 实例化完成后捕获的模块导出 memory（需写内存的 preview-1 调用消费）。 */
const runtime = { memory: null };

const dataView = () => new DataView(runtime.memory.buffer);

const decoder = new TextDecoder("utf-8", { fatal: false });

/** C iovec 数组（{ptr u32, len u32} × n）→ 总长 + 各分段视图。 */
function readIovecs(iovs, iovsLen) {
  const view = dataView();
  let total = 0;
  const parts = [];
  for (let i = 0; i < iovsLen; i++) {
    const base = iovs + i * 8;
    const ptr = view.getUint32(base, true);
    const len = view.getUint32(base + 4, true);
    total += len;
    parts.push(new Uint8Array(runtime.memory.buffer, ptr, len));
  }
  return { total, parts };
}

export function environ_sizes_get(retptr, retptrLen) {
  const view = dataView();
  view.setUint32(retptr, 0, true);
  view.setUint32(retptrLen, 0, true);
  return ERRNO_SUCCESS;
}

export function environ_get() {
  return ERRNO_SUCCESS;
}

export function clock_time_get(_clockId, _precision, retptr) {
  // 仅兑现 import（确定性内核不消费宿主时钟）；毫秒 → 纳秒。
  dataView().setBigInt64(retptr, BigInt(Date.now()) * 1_000_000n, true);
  return ERRNO_SUCCESS;
}

export function random_get(buf, len) {
  // 仅兑现 import（math.random 由内核种子化，不依赖此路径）。
  const out = new Uint8Array(runtime.memory.buffer, buf, len);
  const crypto = globalThis.crypto;
  if (crypto && typeof crypto.getRandomValues === "function") {
    for (let filled = 0; filled < len; filled += 65536) {
      crypto.getRandomValues(out.subarray(filled, Math.min(filled + 65536, len)));
    }
  } else {
    for (let i = 0; i < len; i++) out[i] = (Math.random() * 256) | 0;
  }
  return ERRNO_SUCCESS;
}

export function fd_write(fd, iovs, iovsLen, nwrittenPtr) {
  // stdout/stderr → console（wasi-libc stdio 初始化与调试输出消费点）；
  // 其余 fd 一律 BADF（无文件系统语义面）。
  if (fd !== 1 && fd !== 2) {
    dataView().setUint32(nwrittenPtr, 0, true);
    return ERRNO_BADF;
  }
  const { total, parts } = readIovecs(iovs, iovsLen);
  const text = parts.map((part) => decoder.decode(part)).join("");
  (fd === 1 ? console.log : console.error)(text.replace(/\n$/, ""));
  dataView().setUint32(nwrittenPtr, total, true);
  return ERRNO_SUCCESS;
}

export function fd_read() {
  return ERRNO_BADF;
}

export function fd_close() {
  return ERRNO_SUCCESS;
}

export function fd_fdstat_get(_fd, retptr) {
  // 零填充 fdstat（24B：filetype=UNKNOWN + rights 全零）。
  new Uint8Array(runtime.memory.buffer, retptr, 24).fill(0);
  return ERRNO_SUCCESS;
}

export function fd_fdstat_set_flags() {
  return ERRNO_SUCCESS;
}

export function fd_prestat_get() {
  // 返回 BADF 即终止 wasi-libc 的 preopen 枚举（无预打开目录）。
  return ERRNO_BADF;
}

export function fd_prestat_dir_name() {
  return ERRNO_BADF;
}

export function fd_renumber() {
  return ERRNO_BADF;
}

export function fd_seek() {
  return ERRNO_BADF;
}

export function path_open() {
  return ERRNO_NOSYS;
}

export function path_remove_directory() {
  return ERRNO_NOSYS;
}

export function path_rename() {
  return ERRNO_NOSYS;
}

export function path_unlink_file() {
  return ERRNO_NOSYS;
}

export function proc_exit(code) {
  throw new Error(`ygg-wasm wasi proc_exit(${code})`);
}

/** 全量 preview-1 面（importObject 合并路径 / 未知 import 兜底用）。 */
export const wasiSnapshotPreview1 = new Proxy(
  {
    environ_sizes_get,
    environ_get,
    clock_time_get,
    random_get,
    fd_write,
    fd_read,
    fd_close,
    fd_fdstat_get,
    fd_fdstat_set_flags,
    fd_prestat_get,
    fd_prestat_dir_name,
    fd_renumber,
    fd_seek,
    path_open,
    path_remove_directory,
    path_rename,
    path_unlink_file,
    proc_exit,
  },
  {
    get(target, prop) {
      if (typeof prop !== "string" || !(prop in target)) {
        // 未知 preview-1 import：ENOSYS 兜底（签名多半为返回 errno 的函数）。
        return () => ERRNO_NOSYS;
      }
      return target[prop];
    },
  },
);

/** 幂等安装：补丁 WebAssembly.instantiateStreaming / instantiate，把 shim
 * 并入任意 importObject（宿主已带 wasi_snapshot_preview1 时不覆盖）。仅
 * importObject 路径的胶水需要；静态导入直连（方式①）无需调用。 */
export function installWasiShim() {
  if (installWasiShim.installed) return;
  installWasiShim.installed = true;

  const withWasi = (importObject) => {
    const next = importObject ?? {};
    if (!next.wasi_snapshot_preview1) {
      next.wasi_snapshot_preview1 = wasiSnapshotPreview1;
    }
    return next;
  };

  // 实例化完成即捕获 memory：preview-1 调用发生在 start/之后，
  // 胶水 await 的就是本补丁返回的 promise——捕获先于任何调用。
  const captureMemory = (result) => {
    const memory = result?.instance?.exports?.memory;
    if (memory) runtime.memory = memory;
    return result;
  };

  const origInstantiate = WebAssembly.instantiate.bind(WebAssembly);
  WebAssembly.instantiate = (source, importObject, ...rest) =>
    origInstantiate(source, withWasi(importObject), ...rest).then(captureMemory);

  if (typeof WebAssembly.instantiateStreaming === "function") {
    const origStreaming = WebAssembly.instantiateStreaming.bind(WebAssembly);
    WebAssembly.instantiateStreaming = (source, importObject, ...rest) =>
      origStreaming(source, withWasi(importObject), ...rest).then(captureMemory);
  }
}
