// 纯渲染层：只管把组件表与格子涂到画布/DOM 上，不懂任何游戏规则。

export const CELL_SIZE = 56;
export const BOARD_PAD = 12;

const COLOR_BG = "#0d1117";
const COLOR_OPEN = "#161f2a";
const COLOR_WALL = "#3b4754";
const COLOR_GRID = "#263240";
const COLOR_SELECT = "#f2cc60";
const PALETTE = ["#388bfd", "#f85149", "#3fb950", "#d2a8ff"];

/**
 * 把线格式坐标编码（x * 1000 + y）还原为画布格子坐标。
 * @param {number} encoded
 * @returns {{ x: number, y: number }}
 */
export function decodePos(encoded) {
  return { x: Math.floor(encoded / 1000), y: encoded % 1000 };
}

/**
 * 画布像素坐标 → 棋盘格坐标；落在棋盘外返回 null。
 * @param {HTMLCanvasElement} canvas
 * @param {{ width: number, height: number }} board
 * @param {number} clientX
 * @param {number} clientY
 * @returns {{ x: number, y: number } | null}
 */
export function cellFromPointer(canvas, board, clientX, clientY) {
  const rect = canvas.getBoundingClientRect();
  const sx = canvas.width / rect.width;
  const sy = canvas.height / rect.height;
  const px = (clientX - rect.left) * sx - BOARD_PAD;
  const py = (clientY - rect.top) * sy - BOARD_PAD;
  const x = Math.floor(px / CELL_SIZE);
  const y = Math.floor(py / CELL_SIZE);
  if (x < 0 || y < 0 || x >= board.width || y >= board.height) return null;
  return { x, y };
}

/**
 * 重绘棋盘与所有实体方块（仅读状态表，不做任何规则判断）。
 * @param {HTMLCanvasElement} canvas
 * @param {{
 *   board: { width: number, height: number, cells: number[] },
 *   components: Array<{ id: number, name: string, kind: string }>,
 *   entities: Map<number, Map<number, unknown>>,
 *   selectedId: number | null,
 * }} view
 */
export function drawBoard(canvas, view) {
  const { board, components, entities, selectedId } = view;
  const needW = board.width * CELL_SIZE + BOARD_PAD * 2;
  const needH = board.height * CELL_SIZE + BOARD_PAD * 2;
  if (canvas.width !== needW || canvas.height !== needH) {
    canvas.width = needW;
    canvas.height = needH;
  }

  const ctx = canvas.getContext("2d");
  ctx.fillStyle = COLOR_BG;
  ctx.fillRect(0, 0, canvas.width, canvas.height);

  // 协议字段名索引（只用于读取各列原始值，不做业务计算）。
  const byName = new Map(components.map((c) => [c.name, c.id]));
  const posCid = byName.get("pos");
  const hpCid = byName.get("hp");
  const teamCid = byName.get("team");
  const movedCid = byName.get("moved");

  // 1. 画格子底色与网格线
  for (let y = 0; y < board.height; y++) {
    for (let x = 0; x < board.width; x++) {
      const gx = BOARD_PAD + x * CELL_SIZE;
      const gy = BOARD_PAD + y * CELL_SIZE;
      const kind = board.cells[y * board.width + x] ?? 0;
      ctx.fillStyle = kind === 1 ? COLOR_WALL : COLOR_OPEN;
      ctx.fillRect(gx + 1, gy + 1, CELL_SIZE - 2, CELL_SIZE - 2);
      ctx.strokeStyle = COLOR_GRID;
      ctx.strokeRect(gx + 0.5, gy + 0.5, CELL_SIZE - 1, CELL_SIZE - 1);
    }
  }

  // 2. 把每个实体按 pos 涂到对应格子，标出 eid 与数值列
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  for (const [eid, cols] of entities) {
    const pos = posCid !== undefined ? cols.get(posCid) : undefined;
    if (typeof pos !== "number") continue;
    const { x, y } = decodePos(pos);
    const gx = BOARD_PAD + x * CELL_SIZE;
    const gy = BOARD_PAD + y * CELL_SIZE;
    const team = teamCid !== undefined ? Number(cols.get(teamCid) ?? 0) : 0;
    const hp = hpCid !== undefined ? cols.get(hpCid) : undefined;
    const moved = movedCid !== undefined ? Number(cols.get(movedCid) ?? 0) : 0;

    ctx.fillStyle = PALETTE[Math.abs(team) % PALETTE.length];
    ctx.globalAlpha = moved ? 0.62 : 1.0;
    ctx.fillRect(gx + 7, gy + 7, CELL_SIZE - 14, CELL_SIZE - 14);
    ctx.globalAlpha = 1.0;

    if (eid === selectedId) {
      ctx.strokeStyle = COLOR_SELECT;
      ctx.lineWidth = 3;
      ctx.strokeRect(gx + 5, gy + 5, CELL_SIZE - 10, CELL_SIZE - 10);
      ctx.lineWidth = 1;
    }

    ctx.fillStyle = "#ffffff";
    ctx.font = "bold 13px ui-monospace, monospace";
    ctx.fillText(`#${eid}`, gx + CELL_SIZE / 2, gy + CELL_SIZE / 2 - 7);
    if (hp !== undefined) {
      ctx.font = "11px ui-monospace, monospace";
      ctx.fillText(`hp:${hp}`, gx + CELL_SIZE / 2, gy + CELL_SIZE / 2 + 10);
    }
  }
}

/**
 * 把 ready.components 元数据画成表头列，再把各实体的当前组件值填入表身。
 * @param {HTMLTableRowElement} headRow
 * @param {HTMLTableSectionElement} tbody
 * @param {{
 *   components: Array<{ id: number, name: string, kind: string }>,
 *   entities: Map<number, Map<number, unknown>>,
 *   selectedId: number | null,
 * }} view
 * @param {(eid: number) => void} onSelect
 */
export function drawComponentTable(headRow, tbody, view, onSelect) {
  const { components, entities, selectedId } = view;

  headRow.replaceChildren();
  const thEid = document.createElement("th");
  thEid.textContent = "eid";
  headRow.appendChild(thEid);
  for (const comp of components) {
    const th = document.createElement("th");
    th.textContent = `${comp.name} (${comp.kind})`;
    headRow.appendChild(th);
  }

  tbody.replaceChildren();
  const sortedIds = [...entities.keys()].sort((a, b) => a - b);
  for (const eid of sortedIds) {
    const cols = entities.get(eid);
    const tr = document.createElement("tr");
    if (eid === selectedId) tr.classList.add("selected");
    tr.addEventListener("click", () => onSelect(eid));

    const tdId = document.createElement("td");
    tdId.textContent = String(eid);
    tr.appendChild(tdId);

    for (const comp of components) {
      const td = document.createElement("td");
      const val = cols.get(comp.id);
      if (comp.name === "pos" && typeof val === "number") {
        const { x, y } = decodePos(val);
        td.textContent = `${x},${y}`;
      } else {
        td.textContent = val === undefined ? "—" : String(val);
      }
      tr.appendChild(td);
    }
    tbody.appendChild(tr);
  }
}
