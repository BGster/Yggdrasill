// Yggdrasill · 两级列表页逻辑（无构建 ES module）。
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
  const phase = row?.coord?.phase;
  return phase ? String(phase) : "未开局";
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

// ---- 一级：runs 列表（#/）----

async function renderRuns() {
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
        el("th", {}, "mod"),
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
  for (const row of body.runs || []) {
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
        cell(modBasename(row)),
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
  const parts = [table];
  if (body.truncated) {
    parts.push(el("p", { class: "muted" }, "runs 数量超过 200，已截断。"));
  }
  if (!(body.runs || []).length) {
    parts.push(el("p", { class: "hint" }, "当前 runs-root 下还没有已开局的 run。"));
  }
  view.replaceChildren(...parts);
}

function modBasename(row) {
  const mod = row?.meta?.mod;
  if (!mod) return "—";
  const parts = String(mod).split("/");
  return parts[parts.length - 1];
}

// ---- 二级：局内存档列表（#/runs/<id>）----

async function renderRun(runId) {
  const view = $("#view");
  let body;
  try {
    body = await fetchJson(`/api/runs/${enc(runId)}/saves`);
  } catch (err) {
    view.replaceChildren(el("p", { class: "error" }, `读取失败：${err.message}`));
    return;
  }
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
                    openPanel(runId, save.id, body.panel.url);
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

function openPanel(runId, saveId, panelUrl) {
  // URL 形态：/panel/<run-id>/<entry>?save=<sid>（query 传参，可收藏可刷新）。
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

// ---- 路由与刷新 ----

function parseHash() {
  const hash = location.hash || "#/";
  const match = hash.match(/^#\/runs\/([^/]+)$/);
  return match ? { view: "run", runId: decodeURIComponent(match[1]) } : { view: "runs" };
}

async function refresh() {
  const route = parseHash();
  if (route.view === "run") await renderRun(route.runId);
  else await renderRuns();
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
