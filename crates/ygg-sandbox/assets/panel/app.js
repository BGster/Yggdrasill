// Yggdrasill · 面板列表页逻辑（无构建 ES module）。
// 路由：#/（mod 列表）→ #/mods/<key>（实例列表）→ #/runs/<id>（局内存档）；
// 返回导航由 setNav 按当前路由推导（渲染即有意义，结构上不可能 no-op）。
// 取数契约（DESIGN §3.7）：GET /api/runs、GET /api/runs/<id>/saves、
// EventSource /api/stream（event: change 帧后重取当前视图）；SSE 不可用降级 2s 轮询。
// 防御性访问纪律：字段缺失一律显示「—」，不抛错。

const $ = (sel) => document.querySelector(sel);
const enc = encodeURIComponent;

function fmt(value) {
  return value === null || value === undefined || value === "" ? "—" : String(value);
}

function fmtTime(value) {
  const n = Number(value);
  if (!Number.isFinite(n) || n <= 0) return "—";
  return new Date(n * 1000).toLocaleString();
}

function fmtBytes(value) {
  const n = Number(value);
  if (!Number.isFinite(n) || n < 0) return "—";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KiB`;
  return `${(n / 1024 / 1024).toFixed(2)} MiB`;
}

async function fetchJson(url) {
  const res = await fetch(url, { cache: "no-store" });
  let body = null;
  try {
    body = await res.json();
  } catch {
    throw new Error(`非 JSON 响应（HTTP ${res.status}）`);
  }
  if (!res.ok || body.ok === false) {
    throw new Error(body?.error?.message || `HTTP ${res.status}`);
  }
  return body;
}

function phaseOf(row) {
  // 诚实口径：coordination.json 只有常驻模式（交互/daemon/watch）才写，
  // 纯单发派发的活跃局没有心跳文件 —— 那是「已开局・无在跑会话」，不是「未开局」。
  // 心跳超过 10 分钟未更新则标注失联，不让死会话冒充 running。
  const coord = row?.coord;
  if (!coord || !coord.phase) return "未在跑";
  // heartbeat_at 是毫秒、generatedAt 是秒 —— 归一到秒再比 age
  const hbRaw = Number(coord.heartbeat_at || 0);
  const hb = hbRaw > 1e12 ? Math.floor(hbRaw / 1000) : hbRaw;
  const now = Number(row?.generatedAt || Math.floor(Date.now() / 1000));
  if (hb > 0 && now - hb > 600) return `${coord.phase}·失联`;
  return String(coord.phase);
}

function badge(text) {
  const span = document.createElement("span");
  span.className = `badge badge-${text}`;
  span.textContent = text;
  return span;
}

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (key === "class") node.className = value;
    else if (key.startsWith("on")) node.addEventListener(key.slice(2), value);
    else node.setAttribute(key, value);
  }
  for (const child of children) {
    node.append(child);
  }
  return node;
}

function cell(...children) {
  return el("td", {}, ...children);
}

// ---- 一级：mod 列表（#/；对 /api/runs 按 meta.mod 原始串分组，纯前端推导）----

async function renderMods() {
  const view = $("#view");
  let body;
  try {
    body = await fetchJson("/api/runs");
  } catch (err) {
    view.replaceChildren(el("p", { class: "error" }, `读取失败：${err.message}`));
    return;
  }
  $("#runs-root").textContent = `runs-root：${fmt(body.runsRoot)}`;
  $("#version").textContent = `version ${fmt(body.version)}`;
  // 键 = meta.mod 原始串；缺失或解析失败（metaError 行）归键 `-` 的「未知 mod」组，字段显「—」。
  // 客户端不做别名合并（别名归宿主 serve 侧，只服务面板托管，不服务分组）——数据说什么就显示什么。
  const groups = new Map();
  for (const row of body.runs || []) {
    const key = row?.meta?.mod || "-";
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(row);
  }
  const latestOf = (rows) => Math.max(0, ...rows.map((r) => Number(r?.meta?.updated_at) || 0));
  const tbody = el("tbody");
  // 组序：组内最新 updated_at 降序（活跃组自然在前）。
  for (const [key, rows] of [...groups.entries()].sort((a, b) => latestOf(b[1]) - latestOf(a[1]))) {
    tbody.append(
      el(
        "tr",
        { class: "run-row", onclick: () => { location.hash = `#/mods/${enc(key)}`; } },
        cell(el("b", {}, key === "-" ? "—" : basenameOf(key))),
        cell(el("span", { class: "num" }, String(rows.length))),
        cell(fmtTime(latestOf(rows))),
      ),
    );
  }
  const table = el(
    "table",
    { id: "mods-table" },
    el("thead", {}, el("tr", {},
      el("th", {}, "mod"), el("th", {}, "实例数"), el("th", {}, "最近 updatedAt"))),
    tbody,
  );
  const parts = [table];
  if (body.truncated) {
    parts.push(el("p", { class: "muted" }, "runs 数量超过 200，已截断（分组计数可能不全）。"));
  }
  if (!(body.runs || []).length) {
    parts.push(el("p", { class: "hint" }, "当前 runs-root 下还没有已开局的 run。"));
  }
  view.replaceChildren(...parts);
}

function basenameOf(mod) {
  if (!mod) return "—";
  const parts = String(mod).split("/");
  return parts[parts.length - 1];
}

// ---- 二级：mod 实例列表（#/mods/<key>；runs 表去掉 mod 列，按 key 过滤）----

async function renderModInstances(modKey) {
  const view = $("#view");
  let body;
  try {
    body = await fetchJson("/api/runs");
  } catch (err) {
    view.replaceChildren(el("p", { class: "error" }, `读取失败：${err.message}`));
    return;
  }
  $("#runs-root").textContent = `runs-root：${fmt(body.runsRoot)}`;
  $("#version").textContent = `version ${fmt(body.version)}`;
  const rows = (body.runs || []).filter((row) => (row?.meta?.mod || "-") === modKey);
  const table = el(
    "table",
    { id: "runs-table" },
    el(
      "thead",
      {},
      el(
        "tr",
        {},
        el("th", {}, "runId"),
        el("th", {}, "phase"),
        el("th", {}, "backend"),
        el("th", {}, "startedAt"),
        el("th", {}, "updatedAt"),
        el("th", {}, "存档"),
        el("th", {}, "openerActor"),
        el("th", {}, ""),
      ),
    ),
  );
  const tbody = el("tbody");
  for (const row of rows) {
    const opener = row?.meta?.openerActor;
    tbody.append(
      el(
        "tr",
        {
          class: "run-row",
          onclick: () => {
            location.hash = `#/runs/${enc(row.runId)}`;
          },
        },
        cell(el("b", {}, fmt(row.runId))),
        cell(badge(phaseOf(row))),
        cell(fmt(row.backend)),
        cell(fmtTime(row?.meta?.started_at)),
        cell(fmtTime(row?.meta?.updated_at)),
        cell(el("span", { class: "num" }, fmt(row?.probe?.count))),
        cell(fmt(opener)),
        cell(el("a", { href: `#/runs/${enc(row.runId)}` }, "存档 →")),
      ),
    );
  }
  table.append(tbody);
  const heading = modKey === "-" ? "未知 mod" : `mod ${basenameOf(modKey)}`;
  const parts = [el("h2", {}, `${heading} 的实例`), table];
  if (body.truncated) {
    parts.push(el("p", { class: "muted" }, "runs 数量超过 200，已截断。"));
  }
  if (!rows.length) {
    parts.push(el("p", { class: "hint" }, "该 mod 下暂无实例（可能已删除，或列表被截断）。"));
  }
  view.replaceChildren(...parts);
}

// ---- 二级：局内存档列表（#/runs/<id>）----

async function renderRun(runId) {
  const view = $("#view");
  // 返回链的父级解析与存档取数并行（同为 GET-only 面；截断/已删 → 降级返回链）。
  const runsPromise = fetchJson("/api/runs").catch(() => null);
  let body;
  try {
    body = await fetchJson(`/api/runs/${enc(runId)}/saves`);
  } catch (err) {
    setNav({ view: "run" }); // 父级未知：降级 ← mod 列表，返回链永不死
    view.replaceChildren(el("p", { class: "error" }, `读取失败：${err.message}`));
    return;
  }
  const runsBody = await runsPromise;
  // 双取数窗口内可能已切路由：过期渲染直接丢弃，防跨视图覆盖。
  const now = parseHash();
  if (now.view !== "run" || now.runId !== runId) return;
  const parentKey = (runsBody?.runs || []).find((row) => row?.runId === runId)?.meta?.mod;
  setNav(parentKey ? { view: "run", modKey: parentKey } : { view: "run" });
  $("#version").textContent = "";
  const summary = el(
    "p",
    { class: "summary" },
    el("span", {}, "run ", el("b", {}, fmt(body.runId))),
    el("span", {}, "backend ", el("b", {}, fmt(body.backend))),
    el("span", {}, "generation ", el("b", {}, fmt(body.generation))),
    el(
      "span",
      {},
      "面板 ",
      el("b", {}, body?.panel?.available ? "可用" : fmt(body?.panel?.reason)),
    ),
  );
  const table = el(
    "table",
    { id: "saves-table" },
    el(
      "thead",
      {},
      el(
        "tr",
        {},
        el("th", {}, "id"),
        el("th", {}, "generation"),
        el("th", {}, "seq"),
        el("th", {}, "createdAt"),
        el("th", {}, "bytes"),
        el("th", {}, "动作"),
      ),
    ),
  );
  const tbody = el("tbody");
  for (const save of body.saves || []) {
    tbody.append(
      el(
        "tr",
        {},
        cell(el("b", {}, fmt(save.id)), save.latest ? " " : ""),
        cell(fmt(save.generation)),
        cell(el("span", { class: "num" }, fmt(save.seq))),
        cell(fmtTime(save.createdAt)),
        cell(el("span", { class: "num" }, fmtBytes(save.bytes))),
        cell(
          el("a", { href: `/api/runs/${enc(runId)}/saves/${enc(save.id)}`, target: "_blank" }, "原始 JSON"),
          " ",
          body?.panel?.available
            ? el(
                "a",
                {
                  href: "#",
                  onclick: (ev) => {
                    ev.preventDefault();
                    openPanel(save.id, body.panel.url);
                  },
                },
                "在面板打开",
              )
            : el("span", { class: "muted" }, fmt(body?.panel?.reason)),
        ),
      ),
    );
  }
  table.append(tbody);
  view.replaceChildren(
    el("h2", {}, `局 ${fmt(runId)} 的存档`),
    summary,
    table,
    el("div", { id: "panel-area" }),
  );
}

function openPanel(saveId, panelUrl) {
  // URL 形态：/panel/<run-id>/<entry>?save=<sid>（panelUrl 由服务端拼好 run-id 段；
  // query 传参，可收藏可刷新）。runId 不在客户端拼接，无死参。
  const area = $("#panel-area");
  const url = `${panelUrl}?save=${enc(saveId)}`;
  area.replaceChildren(
    el(
      "div",
      { class: "actions" },
      el("a", { href: url, target: "_blank" }, "新标签打开面板 ↗"),
    ),
    el("div", { class: "iframe-wrap" }, el("iframe", { src: url, title: "mod 面板" })),
  );
  area.scrollIntoView({ behavior: "smooth" });
}

// ---- 路由、返回导航与刷新 ----

// 单次 decode；畸形编码（如悬 `%`）按字面量呈现，不抛错（防御性纪律）。
function decodeOnce(raw) {
  try {
    return decodeURIComponent(raw);
  } catch {
    return raw;
  }
}

function parseHash() {
  const hash = location.hash || "#/";
  let match = hash.match(/^#\/runs\/([^/]+)$/);
  if (match) return { view: "run", runId: decodeOnce(match[1]) };
  match = hash.match(/^#\/mods\/([^/]+)$/);
  if (match) return { view: "mod", modKey: decodeOnce(match[1]) };
  return { view: "mods" };
}

// 返回导航：由当前路由推导而非点击历史——渲染即有意义，结构上不可能 no-op。
// 根视图→空（不留任何点击无效的入口）；二级→← mod 列表；三级→← <mod basename> 实例，
// 父级未知（截断/已删）降级为 ← mod 列表。
function setNav(route) {
  let link = null;
  if (route.view === "mod") {
    link = el("a", { href: "#/" }, "← mod 列表");
  } else if (route.view === "run") {
    link = route.modKey
      ? el("a", { href: `#/mods/${enc(route.modKey)}` }, `← ${basenameOf(route.modKey)} 实例`)
      : el("a", { href: "#/" }, "← mod 列表");
  }
  $("#nav-back").replaceChildren(...(link ? [link] : []));
}

async function refresh() {
  const route = parseHash();
  if (route.view !== "run") setNav(route); // run 视图的返回链由 renderRun 解析父级后设置
  if (route.view === "run") await renderRun(route.runId);
  else if (route.view === "mod") await renderModInstances(route.modKey);
  else await renderMods();
}

function setSseState(kind) {
  const node = $("#sse-state");
  node.className = `sse sse-${kind}`;
  node.textContent = { stream: "SSE 实时", poll: "轮询 2s", reconnect: "重连中…", init: "连接中" }[kind] || kind;
}

let pollTimer = null;

function startPolling() {
  if (pollTimer) return;
  pollTimer = setInterval(() => void refresh(), 2000);
}

function startStream() {
  const source = new EventSource("/api/stream");
  source.addEventListener("change", (ev) => {
    try {
      const frame = JSON.parse(ev.data);
      $("#version").textContent = `version ${fmt(frame.version)}`;
    } catch {
      // 帧体不可解析：仍然重取（变更事实已到达）。
    }
    void refresh();
  });
  source.onopen = () => {
    setSseState("stream");
    if (pollTimer) {
      clearInterval(pollTimer);
      pollTimer = null;
    }
  };
  source.onerror = () => {
    source.close();
    setSseState("poll");
    startPolling();
  };
}

setSseState("init");
void refresh().then(startStream);
window.addEventListener("hashchange", () => void refresh());
