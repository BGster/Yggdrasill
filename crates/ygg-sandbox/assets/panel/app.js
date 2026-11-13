// Yggdrasill · 面板页逻辑（无构建 ES module）。
// 路由：#/（默认=最新局详情）→ #/runs（历史局，mod 分组列表）→ #/mods/<key>（实例列表）
//       → #/runs/<id>（指定局详情，与默认共用同一渲染器；非最新局显示「← 最新局」）；
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

// ---- 局详情投影辅助（只读展示投影；权威=mod 回执/CLI。P11 裁决落档：CONTEXT D-MA16）----

const EVENT_TAIL = 500; // 与 serve 封顶一致（EVENT_LIMIT_MAX）

// 决策事件族：时间线与需求卡决策行只投影这些；工具桥/压缩等过程事件不进投影。
const DECISION_EVENTS = new Set([
  "requirement.add", "requirement.dispatch", "requirement.amend", "requirement.split",
  "requirement.merge", "requirement.conflict", "requirement.resolve",
  "proposal.submit", "proposal.accept", "proposal.reject",
  "gate.approve", "gate.reject", "lane.claim", "lane.release",
  "stage.advance", "verify.submit", "review.note",
]);

// 事件 payload 是 JSON 编码原文（数字 0、字符串带引号、或对象字面量）：防御性解析，
// 解不出来按原文截断显示，不猜不抛错（语义清零）。
function payloadSegments(raw) {
  if (typeof raw !== "string") return [];
  let value;
  try {
    value = JSON.parse(raw);
  } catch {
    return raw.split("|");
  }
  return typeof value === "string" ? value.split("|") : [];
}

function payloadText(raw) {
  if (typeof raw !== "string") return raw == null ? "" : String(raw);
  try {
    const value = JSON.parse(raw);
    return typeof value === "string" ? value : JSON.stringify(value);
  } catch {
    return raw;
  }
}

function truncate(text, max) {
  const s = String(text ?? "");
  const n = max || 40;
  return s.length > n ? `${s.slice(0, n)}…` : s;
}

function kvStr(kv, key) {
  const v = kv?.[key];
  if (typeof v === "string") return v;
  if (v == null) return "";
  return String(v);
}

function kvNum(kv, key) {
  const n = Number(kv?.[key]);
  return Number.isFinite(n) ? n : 0;
}

// 归因：载荷里第一段形如 rN 的段（proposal.*/gate.*/requirement.amend 等在首段，
// lane.claim 在第二段；requirement.add 载荷无 rN 段，如实归「—」不猜。
// proposal.accept 载荷是提案 id（"p5"）：经 KV prop 行确定性联接回 rN（prop 行 req/stage 不可变，
// 非猜测）；联接不上归「—」。
function reqAttribution(segments, propById) {
  const hit = segments.find((s) => /^r\d+$/.test(s));
  if (hit) return hit;
  const p = segments.find((s) => /^p\d+$/.test(s));
  const req = p ? propById?.[p]?.req : null;
  return req > 0 ? `r${req}` : "";
}

// 各决策事件的摘要段（与 src/handlers/requirements 与 operate 的载荷契约一一对应）。
function eventSummary(event, segments) {
  if (event === "proposal.submit") return segments[3] || ""; // rN|rev|stage|summary|...
  if (event === "gate.approve" || event === "gate.reject") return segments[1] || ""; // rN|note|feedback
  if (event === "requirement.add") return segments[0] || ""; // title|description
  if (event === "stage.advance") return segments[0] || ""; // 目标阶段（runtime/flow.lua 唯一 emit 点）
  return "";
}

// 复刻 agent-seed `req_phase_name`（.ygg/mods/agent-seed/src/domain/requirements.lua :220）——
// 展示投影纯函数：status + gate + 待决提案 + plan_ok 派生 14 名之一；
// 未知组合一律显示存储态原词（诚实降级），权威仍是 mod 回执/CLI。
function reqPhase(req, propsByStage) {
  const pending = (stageName) => (propsByStage[stageName] || []).some((p) => p.status === "pending");
  const status = req.status;
  if (status === "draft") return "draft";
  if (status === "analyzing") return pending("analysis") ? "analysis_review" : "analyzing";
  if (status === "planning") {
    if (req.gate === "plan") return "plan_gate";
    if (pending("analysis")) return "analysis_review";
    if (req.planOk === true) return "ready";
    if (pending("plan")) return "plan_review";
    return "planning";
  }
  if (status === "developing") {
    if (req.gate === "accept") return "accept";
    if (pending("development")) return "dev_review";
    return "developing";
  }
  return status || "—";
}

function collectReqs(kv) {
  const count = kvNum(kv, "req.count");
  const reqs = [];
  for (let i = 1; i <= count; i++) {
    reqs.push({
      index: i,
      id: `r${i}`,
      status: kvStr(kv, `req.${i}.status`),
      title: kvStr(kv, `req.${i}.title`),
      rev: kvNum(kv, `req.${i}.rev`),
      gate: kvStr(kv, `req.${i}.gate`),
      planOk: kv?.[`req.${i}.plan_ok`] === true,
      lane: kvNum(kv, `req.${i}.lane`),
      lastLane: kvNum(kv, `req.${i}.last_lane`),
      conflict: kvStr(kv, `req.${i}.conflict`),
      artifacts: kvStr(kv, `req.${i}.artifacts`),
    });
  }
  return reqs;
}

function collectProps(kv) {
  const count = kvNum(kv, "prop.count");
  const props = [];
  for (let i = 1; i <= count; i++) {
    const req = kvNum(kv, `prop.${i}.req`);
    if (req <= 0) continue; // 畸形行跳过：不猜不抛错
    props.push({
      id: `p${i}`,
      req,
      stage: kvStr(kv, `prop.${i}.stage`),
      summary: kvStr(kv, `prop.${i}.summary`),
      artifacts: kvStr(kv, `prop.${i}.artifacts`),
      status: kvStr(kv, `prop.${i}.status`) || "—",
    });
  }
  return props;
}

function laneText(kv, req) {
  if (req.lane > 0) {
    const branch = kvStr(kv, `lane.${req.lane}.branch`);
    return `lane-${req.lane}${branch ? ` @ ${branch}` : ""}`;
  }
  if (req.lastLane > 0) {
    const branch = kvStr(kv, `lane.${req.lastLane}.branch`);
    return `lane-${req.lastLane}（已释放）${branch ? ` @ ${branch}` : ""}`;
  }
  return "—";
}

function executorText(kv, req) {
  // 执行者台账 settings.executor.<rN>.<stage>；当前 stage 由状态唯一映射（终态无「当前」列全量）。
  const stageByStatus = { analyzing: "analysis", planning: "plan", developing: "development" };
  const stage = stageByStatus[req.status];
  if (stage) {
    const v = kvStr(kv, `settings.executor.${req.id}.${stage}`);
    if (v) return `${v}·${stage}`;
  }
  const found = ["analysis", "plan", "development"]
    .map((s) => [s, kvStr(kv, `settings.executor.${req.id}.${s}`)])
    .filter(([, v]) => v)
    .map(([s, v]) => `${v}·${s}`);
  return found.length ? found.join(" / ") : "—";
}

// KV 产物字段是逗号分隔的 VFS 全路径（artifacts/ 前缀由 store 门强制）；
// 链接 URL 段 = 含 artifacts/ 前缀的全路径（serve 测试自证形态）。
function artifactLinks(runId, csv) {
  const paths = String(csv || "").split(",").map((s) => s.trim()).filter(Boolean);
  if (!paths.length) return document.createTextNode("—");
  const kids = [];
  for (const p of paths) {
    if (kids.length) kids.push(" · ");
    kids.push(el("a", { href: `/api/runs/${enc(runId)}/artifacts/${p.split("/").map(enc).join("/")}`, target: "_blank" }, p));
  }
  return el("span", { class: "artifacts" }, ...kids);
}

// 尾窗两步取：先 limit=1 拿信封 latest（since 缺省 0 时 records 是最早一条，但 latest 是
// 全局最新 seq），再 since=latest-499&limit=500 取真正尾窗——since 是闭区间升序 take，不是倒数。
async function fetchEventTail(runId) {
  try {
    const head = await fetchJson(`/api/runs/${enc(runId)}/events?limit=1`);
    const latest = Number(head?.latest) || 0;
    if (latest <= 0) return { latest: 0, records: [] };
    const since = Math.max(0, latest - (EVENT_TAIL - 1));
    const tail = await fetchJson(`/api/runs/${enc(runId)}/events?since=${since}&limit=${EVENT_TAIL}`);
    return { latest, records: tail?.records || [] };
  } catch (err) {
    return { error: err };
  }
}

// ---- 历史局：mod 分组列表（#/runs；对 /api/runs 按 meta.mod 原始串分组，纯前端推导）----

async function renderMods() {
  const view = $("#view");
  let body;
  try {
    body = await fetchJson("/api/runs");
  } catch (err) {
    view.replaceChildren(el("p", { class: "error" }, `读取失败：${err.message}`));
    return;
  }
  $("#roots").textContent = `roots：${(body.roots || []).map(rootLabel).join("、")}`;
  $("#roots").title = (body.roots || []).join("\n") || "—";
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
  const parts = [el("h2", {}, "历史局"), table];
  if (body.truncated) {
    parts.push(el("p", { class: "muted" }, "runs 数量超过 200，已截断（分组计数可能不全）。"));
  }
  if (!(body.runs || []).length) {
    parts.push(el("p", { class: "hint" }, "各观察根下还没有已开局的 run。"));
  }
  view.replaceChildren(...parts);
}

function basenameOf(mod) {
  if (!mod) return "—";
  const parts = String(mod).split("/");
  return parts[parts.length - 1];
}

// ---- 历史局二级：mod 实例列表（#/mods/<key>；runs 表去掉 mod 列，按 key 过滤）----

async function renderModInstances(modKey) {
  const view = $("#view");
  let body;
  try {
    body = await fetchJson("/api/runs");
  } catch (err) {
    view.replaceChildren(el("p", { class: "error" }, `读取失败：${err.message}`));
    return;
  }
  $("#roots").textContent = `roots：${(body.roots || []).map(rootLabel).join("、")}（${(body.roots || []).length}）`;
  $("#roots").title = (body.roots || []).join("\n") || "—";
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
        el("th", {}, "root"),
        el("th", {}, "root"),
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
        cell(el("span", { class: "muted", title: row.root || "—" }, rootLabel(row.root))),
        cell(el("span", { class: "muted", title: row.root || "—" }, rootLabel(row.root))),
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

// ---- 局详情（默认视图=最新局；#/runs/<id>=指定局，共用同一渲染器）----

// 双取数窗口内可能已切路由：过期渲染直接丢弃，防跨视图覆盖。
function staleDetailRoute(pinnedRunId, runId) {
  const route = parseHash();
  if (route.view === "run") return route.runId !== runId;
  return pinnedRunId != null; // 指定局渲染只接受仍在该局路由；默认视图只接受最新局路由
}

function rootLabel(root) {
  // 根标识：剥掉规范化布局的 `.ygg/runs`（或 `runs`）尾巴露出项目名；
  // 全路径放在 title 上（悬停可见），一级列表保持一屏可读。
  const parts = String(root || "").split("/").filter(Boolean);
  if (parts.length >= 3 && parts.slice(-2).join("/") === ".ygg/runs") {
    return parts[parts.length - 3];
  }
  if (parts.length >= 2 && parts[parts.length - 1] === "runs") {
    return parts[parts.length - 2];
  }
  return String(root || "—");
}

function renderNoRuns(view, runsBody) {
  if (runsBody) {
    $("#version").textContent = `version ${fmt(runsBody.version)}`;
    $("#roots").textContent = `roots：${(runsBody.roots || []).map(rootLabel).join("、")}（${(runsBody.roots || []).length}）`;
    $("#roots").title = (runsBody.roots || []).join("\n") || "—";
  }
  view.replaceChildren(
    el("p", { class: "hint" }, "各观察根下还没有已开局的 run。"),
    el("p", { class: "hint" }, "起一局（人读起局命令）："),
    el(
      "pre",
      { class: "cmd" },
      ".ygg/bin/egg --mod .ygg/mods/agent-seed --run-id <名> --runs-root .ygg/runs --event event.start",
    ),
  );
}

function runHeader(runId, row, kv, hasWorkflow) {
  const badges = [];
  if (hasWorkflow) {
    // 宏观阶段：fsm.stage 缺席=intake（渲染层显示 intake 而非 —，与 mod stage() 兼容口径一致）。
    badges.push(badge(kvStr(kv, "fsm.stage") || "intake"));
    badges.push(el("span", { class: "badge" }, `批 ${kvNum(kv, "run.batch") || 1}`));
    const mode = kvStr(kv, "settings.play.mode");
    if (mode) {
      const authorized = kvStr(kv, "settings.goal.authorization") !== "";
      badges.push(
        el(
          "span",
          {
            class: `badge${mode === "goal" && !authorized ? " badge-goal-warn" : ""}`,
            title: mode === "goal" && !authorized ? "goal 模式未登记 settings.goal.authorization" : "",
          },
          mode,
        ),
      );
    }
    if (kvStr(kv, "ending") === "done") badges.push(el("span", { class: "badge badge-finalized" }, "已退役"));
  }
  badges.push(badge(phaseOf(row)));
  return el(
    "div",
    { class: "run-head-wrap" },
    el(
      "div",
      { class: "run-head" },
      el("h2", {}, `局 ${fmt(runId)}`),
      el("div", { class: "badges" }, ...badges),
    ),
    el(
      "p",
      { class: "summary" },
      el("span", {}, "backend ", el("b", {}, fmt(row?.backend))),
      el("span", {}, "startedAt ", el("b", {}, fmtTime(row?.meta?.started_at))),
      el("span", {}, "updatedAt ", el("b", {}, fmtTime(row?.meta?.updated_at))),
      el("span", {}, "openerActor ", el("b", {}, fmt(row?.meta?.openerActor))),
      el("span", {}, "mod ", el("b", {}, row?.meta?.mod ? basenameOf(row.meta.mod) : "—")),
    ),
  );
}

// 主流程状态条：intake → coordinate ⇄ review(批 n) → done；当前节点高亮；
// done 仅退役后点亮（诚实口径：不预告未到达的终局）。
function stageBarNode(kv, eventsTail) {
  const current = kvStr(kv, "fsm.stage") || "intake";
  const retired = kvStr(kv, "ending") === "done";
  const batch = kvNum(kv, "run.batch") || 1;
  const advancedAt = {};
  for (const rec of eventsTail.records || []) {
    if (rec?.event === "stage.advance") advancedAt[payloadText(rec.payload)] = rec;
  }
  const node = (name, label) => {
    const adv = advancedAt[name];
    return el(
      "span",
      {
        class: `stage-node${name === current ? " now" : ""}${name === "done" && retired ? " lit" : ""}`,
        title: adv ? `最近转场 seq ${fmt(adv.seq)} @ ${fmtTime(adv.timestamp)}` : "",
      },
      label,
    );
  };
  const sep = (mark) => el("span", { class: "stage-sep" }, mark);
  return el(
    "div",
    { class: "stage-bar" },
    node("intake", "intake"),
    sep("→"),
    node("coordinate", "coordinate"),
    sep("⇄"),
    node("review", `review(批 ${batch})`),
    sep("→"),
    node("done", "done"),
  );
}

// 门禁标签：工作流固定序只有两道门（plan→plan_gate、development→accept，D-MA1）；
// 归属不了的显示中性 gate，不猜。
const GATE_LABEL = { plan: "plan_gate", development: "accept" };

function propChipNode(p) {
  if (p.status === "pending") {
    return [
      el(
        "span",
        { class: "prop-chip prop-pending", title: p.summary },
        `${p.id} 待验收${p.summary ? `（${truncate(p.summary)}）` : ""}`,
      ),
      " ",
    ];
  }
  if (p.status === "rejected") {
    return [el("span", { class: "prop-chip prop-rejected", title: p.summary }, `${p.id} 被拒`), " "];
  }
  if (p.status === "stale") {
    return [el("span", { class: "prop-chip prop-rejected" }, `${p.id} 已失效`), " "];
  }
  return [];
}

function gateMarkNode(g, stage) {
  const refused = Number(g.ok) === 0; // 被拒的派发尝试：决策史不可抹，灰显保留
  let cls;
  let text;
  if (refused) {
    cls = "refused";
    text = `${g.event}(seq ${fmt(g.seq)})·被拒`;
  } else if (g.event === "gate.approve") {
    cls = "ok";
    text = `${GATE_LABEL[stage] || "gate"} ✓(seq ${fmt(g.seq)})`;
  } else {
    cls = "bad";
    text = `${GATE_LABEL[stage] || "gate"} ✗(seq ${fmt(g.seq)})`;
  }
  return [el("span", { class: `gate-mark ${cls}` }, text), " "];
}

// 门禁事件归属扫描：返回每需求每 stage 的门禁事件桶（含归属不了的中性 gate 桶）。
// accept 载荷是提案 id，经 KV prop 行联接出 req+stage；联接不上归入中性 gate 桶，不猜。
function gateBucketsFor(records, propById) {
  const lastAccepted = {};
  const gateByStage = {};
  for (const rec of records) {
    const segs = payloadSegments(rec?.payload);
    if (rec?.event === "proposal.accept") {
      const who = reqAttribution(segs, propById);
      const prop = segs.find((s) => /^p\d+$/.test(s));
      if (who) lastAccepted[who] = segs[2] || (prop ? propById?.[prop]?.stage : "") || "";
    } else if (rec?.event === "gate.approve" || rec?.event === "gate.reject") {
      const who = reqAttribution(segs, propById);
      if (who) (gateByStage[lastAccepted[who] || "gate"] ||= []).push({ rec, reqId: who });
    }
  }
  return gateByStage;
}

// 状态内决策与产物行（按 stage 分组）：提案（待决/被拒灰显）+ 产物链接 + 门禁结果。
function stageLinesNode(req, byStage, records, runId, propById) {
  const gateAll = gateBucketsFor(records, propById);
  const gateByStage = {};
  for (const [stage, marks] of Object.entries(gateAll)) {
    const mine = marks.filter((m) => m.reqId === req.id).map((m) => m.rec);
    if (mine.length) gateByStage[stage] = mine;
  }
  const stages = ["analysis", "plan", "development"].filter(
    (s) => (byStage[s] || []).length || (gateByStage[s] || []).length,
  );
  if ((gateByStage["gate"] || []).length) stages.push("gate"); // 归属不了的门禁也可见，不静默丢失
  if (!stages.length) return null;
  const wrap = el("div", { class: "req-stages" });
  for (const stage of stages) {
    const kids = [el("b", {}, stage), el("span", { class: "muted" }, " → ")];
    for (const p of byStage[stage] || []) {
      kids.push(...propChipNode(p));
      if (p.artifacts) kids.push(artifactLinks(runId, p.artifacts), " ");
    }
    for (const g of gateByStage[stage] || []) {
      kids.push(...gateMarkNode(g, stage));
    }
    wrap.append(el("div", { class: "stage-line" }, ...kids));
  }
  return wrap;
}

function reqCard(kv, req, props, records, runId, propById) {
  const byStage = {};
  for (const p of props) {
    if (p.req === req.index) (byStage[p.stage] ||= []).push(p);
  }
  const card = el(
    "div",
    { class: "req-card" },
    el(
      "div",
      { class: "req-head" },
      el("b", {}, `${req.id} ${fmt(req.title)}`),
      badge(reqPhase(req, byStage)),
      el("span", { class: "muted" }, `rev ${fmt(req.rev)}`),
    ),
    el(
      "div",
      { class: "req-line" },
      el("span", {}, "泳道 ", el("b", {}, laneText(kv, req))),
      el("span", {}, "执行者 ", el("b", {}, executorText(kv, req))),
      req.conflict ? el("span", { class: "conflict" }, `⚠ 冲突 ${fmt(req.conflict)}`) : el("span", {}),
    ),
  );
  const stageLines = stageLinesNode(req, byStage, records, runId, propById);
  if (stageLines) card.append(stageLines);
  if (req.artifacts) {
    card.append(
      el("div", { class: "req-line" }, el("span", {}, "交付产物 ", artifactLinks(runId, req.artifacts))),
    );
  }
  return card;
}

function reqCardsNode(kv, records, runId, props, propById) {
  const reqs = collectReqs(kv);
  const wrap = el("div", { class: "req-list" });
  for (const req of reqs) wrap.append(reqCard(kv, req, props, records, runId, propById));
  return el("section", { class: "detail-sec" }, el("h3", {}, `需求（${reqs.length}）`), wrap);
}

// 近期决策时间线：全局倒序、可见上限 20 条（限决策事件族）；ok=0 的被拒派发标红保留。
function timelineNode(eventsTail, propById) {
  const sec = el("section", { class: "detail-sec" }, el("h3", {}, "近期决策"));
  if (eventsTail.error) {
    sec.append(el("p", { class: "error" }, `事件读取失败：${eventsTail.error.message}`));
    return sec;
  }
  const rows = [];
  for (const rec of eventsTail.records || []) {
    if (!DECISION_EVENTS.has(rec?.event)) continue;
    const segs = payloadSegments(rec?.payload);
    rows.push({
      seq: rec.seq,
      ts: rec.timestamp,
      event: rec.event,
      refused: Number(rec.ok) === 0,
      who: reqAttribution(segs, propById),
      summary: eventSummary(rec.event, segs),
    });
  }
  if (!rows.length) {
    sec.append(el("p", { class: "hint" }, "决策事件族在窗口内暂无记录。"));
  } else {
    const tbody = el("tbody");
    const visible = Math.min(rows.length, 20);
    for (let i = 0; i < visible; i++) {
      const r = rows[rows.length - 1 - i]; // 记录升序到达，展示倒序（最新在前，最多 20 条）
      tbody.append(
        el(
          "tr",
          { class: r.refused ? "refused" : "" },
          cell(el("span", { class: "num" }, fmt(r.seq))),
          cell(fmtTime(r.ts)),
          cell(fmt(r.event)),
          cell(fmt(r.who)),
          cell(truncate(r.summary, 60)),
        ),
      );
    }
    sec.append(
      el(
        "table",
        { class: "timeline-table" },
        el(
          "thead",
          {},
          el("tr", {}, el("th", {}, "seq"), el("th", {}, "时刻"), el("th", {}, "事件"), el("th", {}, "归因"), el("th", {}, "摘要")),
        ),
        tbody,
      ),
    );
  }
  if (eventsTail.latest > EVENT_TAIL) {
    sec.append(el("p", { class: "muted" }, `仅最近 ${EVENT_TAIL} 事件（本局共 ${fmt(eventsTail.latest)} 条）。`));
  }
  return sec;
}

function savesTable(runId, savesBody) {
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
  for (const save of savesBody.saves || []) {
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
          savesBody?.panel?.available
            ? el(
                "a",
                {
                  href: "#",
                  onclick: (ev) => {
                    ev.preventDefault();
                    openPanel(save.id, savesBody.panel.url);
                  },
                },
                "在面板打开",
              )
            : el("span", { class: "muted" }, fmt(savesBody?.panel?.reason)),
        ),
      ),
    );
  }
  table.append(tbody);
  return table;
}

// 本局存档段：默认折叠；内含 saves 表（r3 renderRun 原样下沉）+ 制品全清单 + mod 面板入口。
function savesSection(runId, savesBody, artifactsBody) {
  const details = el("details", { class: "saves-details", id: "saves-details" });
  details.append(el("summary", {}, `本局存档（${fmt((savesBody?.saves || []).length)}）`));
  if (!savesBody) {
    details.append(el("p", { class: "error" }, "存档列表读取失败。"));
  } else {
    details.append(
      el(
        "p",
        { class: "summary" },
        el("span", {}, "generation ", el("b", {}, fmt(savesBody.generation))),
        el("span", {}, "面板 ", el("b", {}, savesBody?.panel?.available ? "可用" : fmt(savesBody?.panel?.reason))),
      ),
      savesTable(runId, savesBody),
    );
  }
  if (!artifactsBody) {
    details.append(el("p", { class: "muted" }, "制品清单读取失败。"));
  } else {
    const arts = artifactsBody?.artifacts || [];
    const kids = [];
    for (const a of arts) {
      if (kids.length) kids.push(" · ");
      kids.push(
        el("a", { href: `/api/runs/${enc(runId)}/artifacts/${String(a?.path || "").split("/").map(enc).join("/")}`, target: "_blank" }, fmt(a?.path)),
        el("span", { class: "muted" }, `(${fmtBytes(a?.bytes)})`),
      );
    }
    if (!kids.length) kids.push("—");
    details.append(el("p", { class: "summary" }, el("span", {}, "制品 "), ...kids));
  }
  details.append(el("div", { id: "panel-area" }));
  return details;
}

// 详情目标解析：默认视图取最新局（/api/runs 首行，updated_at 降序由服务端保证）；
// 指定局按 runId 查行（截断/已删时 row=null，isLatest 不可知=false）；无目标返回 null（空局态）。
function resolveDetailTarget(pinnedRunId, runs) {
  if (pinnedRunId == null) {
    if (!runs.length) return null;
    return { runId: runs[0].runId, row: runs[0], isLatest: true };
  }
  const row = runs.find((r) => r?.runId === pinnedRunId) || null;
  return {
    runId: pinnedRunId,
    row,
    isLatest: runs.length > 0 && runs[0].runId === pinnedRunId,
  };
}

async function renderRunDetail(pinnedRunId) {
  const view = $("#view");
  let runsBody = null;
  try {
    runsBody = await fetchJson("/api/runs");
  } catch {
    runsBody = null;
  }
  if (staleDetailRoute(pinnedRunId, pinnedRunId)) return;
  const runs = runsBody?.runs || [];
  if (runsBody) {
    $("#roots").textContent = `roots：${(runsBody.roots || []).map(rootLabel).join("、")}（${(runsBody.roots || []).length}）`;
    $("#roots").title = (runsBody.roots || []).join("\n") || "—";
  }
  const target = resolveDetailTarget(pinnedRunId, runs);
  if (!target) {
    setNav({ view: "latest-empty" });
    renderNoRuns(view, runsBody);
    return;
  }
  const { runId, row, isLatest } = target;
  // 并行取数：最新存档（KV 投影）、存档列表（折叠段）、制品（折叠段）；事件尾窗两步取。
  const kvPromise = fetchJson(`/api/runs/${enc(runId)}/saves/latest`).catch(() => null);
  const savesPromise = fetchJson(`/api/runs/${enc(runId)}/saves`).catch(() => null);
  const artifactsPromise = fetchJson(`/api/runs/${enc(runId)}/artifacts`).catch(() => null);
  const eventsTail = await fetchEventTail(runId);
  const [kvBody, savesBody, artifactsBody] = await Promise.all([kvPromise, savesPromise, artifactsPromise]);
  if (staleDetailRoute(pinnedRunId, runId)) return;

  const kv = kvBody?.kv?.entries && typeof kvBody.kv.entries === "object" ? kvBody.kv.entries : null;
  // agent-seed 工作流判定 = KV 含 req.* 键族；无键族的 mod 无害退化为通用摘要（D-MA16 判据）。
  const hasWorkflow = !!kv && Object.keys(kv).some((k) => typeof k === "string" && k.startsWith("req."));
  const props = hasWorkflow ? collectProps(kv) : [];
  const propById = Object.fromEntries(props.map((p) => [p.id, p]));

  setNav({ view: "run", modKey: row?.meta?.mod || null, showLatestBack: !isLatest });
  const parts = [runHeader(runId, row, kv, hasWorkflow)];
  if (hasWorkflow) {
    parts.push(stageBarNode(kv, eventsTail));
    parts.push(reqCardsNode(kv, eventsTail.records || [], runId, props, propById));
    parts.push(timelineNode(eventsTail, propById));
  } else if (kvBody === null) {
    parts.push(el("p", { class: "error" }, "最新存档读取失败：工作流投影不可用。"));
  }
  parts.push(savesSection(runId, savesBody, artifactsBody));
  // 轮询/SSE 重渲染会重建 DOM：折叠态与已打开的 mod 面板 iframe 原样保留。
  const prevOpen = $("#saves-details")?.open || false;
  const prevIframe = $("#panel-area iframe")?.src || "";
  view.replaceChildren(...parts);
  const details = $("#saves-details");
  if (details && prevOpen) details.open = true;
  if (prevIframe) {
    $("#panel-area")?.replaceChildren(
      el("div", { class: "iframe-wrap" }, el("iframe", { src: prevIframe, title: "mod 面板" })),
    );
  }
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
  if (hash === "#/runs") return { view: "runs" };
  return { view: "latest" }; // #/ 与空 hash = 最新局详情（默认视图）
}

// 返回导航：由当前路由推导而非点击历史——渲染即有意义，结构上不可能 no-op。
// run 详情→（非最新局时）← 最新局 + ← <mod basename> 实例（父级未知降级 ← mod 列表）；
// 实例列表→← mod 列表；历史局根视图无返回入口；除历史局自身外常驻「历史局」入口。
function setNav(route) {
  const kids = [];
  if (route.view === "run") {
    if (route.showLatestBack) kids.push(el("a", { href: "#/" }, "← 最新局"));
    kids.push(
      route.modKey
        ? el("a", { href: `#/mods/${enc(route.modKey)}` }, `← ${basenameOf(route.modKey)} 实例`)
        : el("a", { href: "#/runs" }, "← mod 列表"),
    );
  } else if (route.view === "mod") {
    kids.push(el("a", { href: "#/runs" }, "← mod 列表"));
  }
  if (route.view !== "runs") kids.push(el("a", { href: "#/runs", class: "nav-history" }, "历史局"));
  $("#nav-back").replaceChildren(...kids);
}

async function refresh() {
  const route = parseHash();
  if (route.view === "run") return renderRunDetail(route.runId);
  if (route.view === "mod") {
    setNav(route);
    return renderModInstances(route.modKey);
  }
  if (route.view === "runs") {
    setNav(route); // 历史局是根视图：不渲染任何入口（含「历史局」自身，防 no-op）
    return renderMods();
  }
  return renderRunDetail(null); // 默认视图 = 最新局详情（SSE/轮询重取时自动切新最新局）
}

function setSseState(kind) {
  const node = $("#sse-state");
  node.className = `sse sse-${kind}`;
  node.textContent = { stream: "SSE 实时", poll: "轮询 2s", reconnect: "重连中…", init: "连接中" }[kind] || kind;
}

// ---- 观察根注册表（r1：进程内配置；POST/DELETE /api/roots，不写局状态）----

async function renderRootsPanel() {
  const summary = $("#roots-summary");
  const list = $("#roots-list");
  let body;
  try {
    body = await fetchJson("/api/roots");
  } catch (err) {
    summary.textContent = `观察根（读取失败：${err.message}）`;
    return;
  }
  const roots = body.roots || [];
  summary.textContent = `观察根（${roots.length}）`;
  // 空根引导：面板默认展开 + 空态文案（新启动的 serve 第一眼就知道下一步）。
  $("#roots-panel").open = roots.length === 0;
  list.replaceChildren(
    ...roots.map((entry) =>
      el(
        "li",
        {},
        el("code", {}, entry.root || "—"),
        entry.singleRun ? el("span", { class: "hint" }, "（单局）") : "",
        el(
          "button",
          {
            type: "button",
            onclick: async () => {
              $("#root-error").hidden = true;
              try {
                const res = await fetch(`/api/roots?root=${enc(entry.root)}`, { method: "DELETE" });
                const result = await res.json();
                if (!res.ok) throw new Error(result?.error?.message || `HTTP ${res.status}`);
                await renderRootsPanel();
                void refresh();
              } catch (err) {
                const node = $("#root-error");
                node.textContent = `注销失败：${err.message}`;
                node.hidden = false;
              }
            },
          },
          "注销",
        ),
      ),
    ),
  );
  if (roots.length === 0) {
    list.replaceChildren(el("li", { class: "hint" }, "尚未注册观察根——粘贴 run-root 或 runs 目录的绝对路径注册。"));
  }
}

$("#root-form").addEventListener("submit", async (ev) => {
  ev.preventDefault();
  $("#root-error").hidden = true;
  const root = $("#root-input").value.trim();
  if (!root) return;
  try {
    const res = await fetch("/api/roots", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ root }),
    });
    const result = await res.json();
    if (!res.ok) throw new Error(result?.error?.message || `HTTP ${res.status}`);
    $("#root-input").value = "";
    await renderRootsPanel();
    void refresh();
  } catch (err) {
    const node = $("#root-error");
    node.textContent = `注册失败：${err.message}`;
    node.hidden = false;
  }
});

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
void renderRootsPanel();
window.addEventListener("hashchange", () => void refresh());
