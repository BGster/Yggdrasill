#!/usr/bin/env python3
# gen-scene-readfirst.py —— 幕必读块（[READ FIRST]）生成器：docs/context 固定头 → core.json 的 scenes.<node>。
#
# 定位：**投影生成器**，不是纪律检查器。输入 = docs/context/scenes.json（场景→doc-id 表）
#       + 各文件固定头（id / 触发 / 读不到代价 / ver）；输出 = mod 台词里可直接打印的必读块。
#       「谁该读什么」的规则在 AGENTS.md §0c（权威）；本脚本只保证「打印出来的东西 == 磁盘上的文档头」。
#
# 为什么落在 scenes.<node>（而不是新文案键）：新增键要动 pack.json entries + render.lua（src 冻结）；
# scenes.<node> 已有渲染者（render.lua 的 RENDER[phase] 第一条 narrate），追加续行即可零-src 生效。
#
# 幂等：块 = `<台词原文>` + "\n" + 块行；先剥旧块再拼，连跑两次 drift=0。
# 漂移：--check 只读比对（不写盘），drift>0 / missing 非空 / stale=1 → ok=0 且 exit 1（供门禁 fail-closed）。
# 键数：不新增任何键，只改 scenes 五个既有键的值（键账 409 beats + 36 core = 445 不变）。
#
# 用法：
#   scripts/gen-scene-readfirst.py [--check] [--signature=read|none] [--readset] [--root <dir>] [--help]
#   --readset  额外把 readPlan / readGate（ver 台账 + reqActs + gates）写进 data/config.json 的生成区
#              （r4 必读集的机器可读形；与 scenes 块**同一批**产出，ver 单源在文档头）
# 输出：
#   scenes=<n>;blocks=<k>;missing=[...];stale=<s>;drift=<d>;readset=<absent|present|drift>;ok=<0|1>
# 退出码：0 = ok（或 write 模式成功）／1 = --check 发现漂移或缺件／2 = 用法错／4 = 输入不可读

import json
import os
import re
import sys
from collections import OrderedDict
from typing import Any, NoReturn

NODES = ("prologue", "charter", "coordinate", "settle", "release")
BLOCK_MARK = "\n  [READ FIRST · "
TRIGGER_MAX = 60
# 提案 stage → reqStage 里承载它的那些需求相位（读哪一份 req-acts）。
STAGE_SOURCE = {
    "plan": ("plan_gate", "planning", "draft"),
    "development": ("ready", "developing", "accept", "accept_gate"),
}
# 门禁 → 它交出下一幕的那份 req-acts（批准计划门禁 = 开始开发；批准验收门禁 = 开始集成）。
GATE_SOURCE = {
    "plan": ("ready", "developing"),
    "accept": ("integrating", "done"),
}
READSET_KEYS = ("readPlan", "readGate")
CONFIG_REL = os.path.join(".ygg", "mods", "agent-seed", "data", "config.json")
USAGE = """scripts/gen-scene-readfirst.py [--check] [--signature=read|none] [--readset] [--root <dir>]
  --check            只比对不写盘；漂移/缺件 → ok=0 且 exit 1
  --signature=read   块尾打可抄的收据命令 config.set read.<id>=<ver>（缺省）
  --signature=none   块尾改印「本回合汇报里写明 reads: …」（r3 未落地时的过渡形态）
  --readset          同时把 readPlan / readGate（ver 台账）写进 data/config.json（r4 必读集）
  --root <dir>       仓库根（缺省 = 本脚本所在目录的父目录）
"""


def die(code: int, msg: object) -> NoReturn:
    """硬退出：调用点之后的代码都不可达（NoReturn 让类型检查器知道这条路径已终止）。"""
    sys.stderr.write(str(msg).rstrip() + "\n")
    sys.exit(code)


def read_text(path: str) -> str:
    """读文本；失败一律 exit 4（本脚本没有「部分成功」的合法状态）。"""
    try:
        with open(path, encoding="utf-8") as handle:
            return handle.read()
    except OSError as err:
        die(4, f"读不到 {path}：{err}")


def write_text(path: str, content: str) -> None:
    try:
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(content)
    except OSError as err:
        die(4, f"写不了 {path}：{err}")


def parse_json(text: str, source: str) -> Any:
    try:
        return json.loads(text)
    except json.JSONDecodeError as err:
        die(4, f"{source} 不是合法 JSON：{err}")


# ── 固定头解析 ────────────────────────────────────────────────────────────────
def read_header(ctx_dir: str, doc_id: str) -> dict[str, str] | None:
    """返回 {id, trigger, cost, ver}；文件缺失/头不全/id 不符 → None（调用方记 missing）。"""
    path = os.path.join(ctx_dir, f"{doc_id}.md")
    if not os.path.isfile(path):
        return None
    head = "\n".join(read_text(path).splitlines()[:8])
    ident = re.search(r"\*\*id\*\*\s+([^\s·]+)", head)
    trigger = re.search(r"\*\*触发\*\*\s+(.*?)\s*(?:·\s*\*\*|$)", head, re.M)
    cost = re.search(r"\*\*读不到代价\*\*\s+(.*?)\s*(?:·\s*\*\*|$)", head, re.M)
    ver = re.search(r"\*\*ver\*\*\s+([0-9a-f]{6})\b", head)
    if not (ident and ver):
        return None
    if ident.group(1) != doc_id:  # r1 scenes.json 一致性规则②：文件头 id 必须与表里的 id 逐字相等
        return None
    return {
        "id": doc_id,
        "trigger": trigger.group(1).strip() if trigger else "",
        "cost": cost.group(1).strip() if cost else "none",
        "ver": ver.group(1),
    }


def short_trigger(text: str) -> str:
    """触发短语：去掉尾部括号补充，超长再压到一屏（不在词中间硬切）。"""
    if not text:
        return ""
    flat = re.sub(r"（[^（）]*）\s*$", "", text).strip()
    if len(flat) <= TRIGGER_MAX:
        return flat
    cut = flat[:TRIGGER_MAX]
    for sep in ("，", " / ", "；", " "):
        pos = cut.rfind(sep)
        if pos > TRIGGER_MAX // 2:
            return cut[:pos].rstrip()
    return cut.rstrip() + "…"


def cost_key(text: str) -> str:
    """代价：取固定头声明的四值域关键字（hard-refusal / irreversible / silent-drift / none）。"""
    return re.sub(r"[（(].*$", "", (text or "none").strip()).strip() or "none"


# ── 块渲染（纯函数：同一输入 → 同一字节）──────────────────────────────────────
def render_block(node: str, docs: list[dict[str, str]], signature: str) -> str:
    lines = [f"  [READ FIRST · {node}] 本幕必读 {len(docs)} 件："]
    for doc in docs:
        gloss = short_trigger(doc["trigger"])
        tail = f"—— {gloss}" if gloss else ""
        lines.append(f"  · docs/context/{doc['id']}.md (ver {doc['ver']}) {tail}｜读不到：{cost_key(doc['cost'])}")
    if signature == "read":
        for doc in docs:
            lines.append(f"  → 读毕签名：config.set read.{doc['id']}={doc['ver']}")
    else:
        joined = ",".join(f"{doc['id']}@{doc['ver']}" for doc in docs)
        lines.append("  → 读毕：在本回合汇报里写明 reads: " + joined)
    return "\n".join(lines)


def plan_of(table: dict[str, Any]) -> dict[str, list[str]]:
    """scenes.json 归一化：兼容 `scene` / `scenes` 两种键名与 数组 / 字符串 两种值形。"""
    raw = table.get("scene") or table.get("scenes") or {}
    return {node: (list(value) if isinstance(value, list) else [value]) for node, value in raw.items()}


def compose(ctx_dir: str, table: dict[str, Any], signature: str) -> tuple[dict[str, str], list[str]]:
    plan = plan_of(table)
    blocks, missing = {}, []
    for node in NODES:
        docs = []
        for doc_id in plan.get(node, []):
            header = read_header(ctx_dir, doc_id)
            if header is None:
                missing.append(f"{node}:{doc_id}")
            else:
                docs.append(header)
        if docs:
            blocks[node] = render_block(node, docs, signature)
    missing.extend(f"{node}:unknown-scene" for node in plan if node not in NODES)
    return blocks, missing


# ── r4 读签必读集（readPlan / readGate）────────────────────────────────────
def req_stage_docs(table: dict[str, Any]) -> dict[str, list[str]]:
    """reqStage 归一化：相位 → doc-id 列表（值可以是数组或单串）。"""
    raw = table.get("reqStage") or {}
    out: dict[str, list[str]] = {}
    for phase, value in raw.items():
        docs = list(value) if isinstance(value, list) else [value]
        out[str(phase)] = [str(item) for item in docs]
    return out


def first_phase(stages: dict[str, list[str]], names: tuple[str, ...]) -> list[str]:
    """按优先级取第一个存在的相位清单（缺失则空 = 该维度不拦）。"""
    for name in names:
        docs = stages.get(name)
        if docs:
            return docs
    return []


def build_readset(ctx_dir: str, table: dict[str, Any]) -> tuple[dict[str, Any], list[str]]:
    """从 scenes.json + 各文件固定头算出 readPlan / readGate（ver 单源在文档头，不另造版本源）。"""
    plan = plan_of(table)
    stages = req_stage_docs(table)
    missing: list[str] = []

    ordered: list[str] = []
    for node in NODES:
        for doc_id in plan.get(node, []):
            if doc_id not in ordered:
                ordered.append(doc_id)
    for docs in stages.values():
        for doc_id in docs:
            if doc_id not in ordered:
                ordered.append(doc_id)

    ver: dict[str, str] = {}
    for doc_id in ordered:
        header = read_header(ctx_dir, doc_id)
        if header is None:
            missing.append(f"readset:{doc_id}")
            continue
        ver[doc_id] = header["ver"]

    read_plan = OrderedDict((node, list(plan[node])) for node in NODES if plan.get(node))
    req_acts = OrderedDict()
    for stage, sources in STAGE_SOURCE.items():
        docs = first_phase(stages, sources)
        if docs:
            req_acts[stage] = docs
    gates = OrderedDict()
    for kind, sources in GATE_SOURCE.items():
        docs = first_phase(stages, sources)
        if docs:
            gates[kind] = docs
    read_gate = OrderedDict((("ver", OrderedDict(ver)), ("reqActs", req_acts), ("gates", gates)))
    return {"readPlan": read_plan, "readGate": read_gate}, missing


def diff_keys(expected: Any, actual: Any, prefix: str = "") -> list[str]:
    """逐键比对，返回差异键的**完整键名**（--check 的 fail-closed 必须点名到键）。"""
    if isinstance(expected, dict) and isinstance(actual, dict):
        out: list[str] = []
        for key in expected:
            child = f"{prefix}.{key}" if prefix else str(key)
            if key not in actual:
                out.append(f"{child} (missing)")
            else:
                out.extend(diff_keys(expected[key], actual[key], child))
        for key in actual:
            if key not in expected:
                out.append(f"{prefix}.{key} (extra)" if prefix else f"{key} (extra)")
        return out
    if expected != actual:
        return [f"{prefix} (table {expected!r} != header {actual!r})"]
    return []


def patch_readset(config_text: str, readset: dict[str, Any]) -> str:
    """保留式 JSON 合并：既有键位置/缩进不动，只就地更新或追加 readPlan / readGate 两个键。"""
    try:
        data = json.loads(config_text, object_pairs_hook=OrderedDict)
    except json.JSONDecodeError as err:
        die(4, f"{CONFIG_REL} 不是合法 JSON：{err}")
    if not isinstance(data, dict):
        die(4, f"{CONFIG_REL} 顶层必须是对象")
    baseline = json.dumps(data, ensure_ascii=False, indent=2)
    if baseline != config_text.rstrip("\n"):
        die(4, f"{CONFIG_REL} 不是 2 空格缩进的规范 JSON（保留式合并会重排无关键）——请先人エ归位")
    for key, value in readset.items():
        data[key] = value
    return json.dumps(data, ensure_ascii=False, indent=2) + "\n"


# ── core.json 外科式替换（不重排文件）──────────────────────────────────────────
def scenes_span(text: str) -> tuple[int, int]:
    """定位 `"scenes": {…}` 的左右花括号下标（字符串/转义绕行），避免误打到 prompts 里的同名键。"""
    anchor = re.search(r'"scenes"\s*:\s*\{', text)
    if anchor is None:
        die(4, "core.json: 找不到 scenes 对象")
    start = anchor.end() - 1
    depth = 0
    in_string = False
    escaped = False
    for index in range(start, len(text)):
        char = text[index]
        if in_string:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                in_string = False
            continue
        if char == '"':
            in_string = True
        elif char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return start, index
    die(4, "core.json: scenes 对象没有闭合")


def patch_core(core_text: str, blocks: dict[str, str]) -> str:
    left, right = scenes_span(core_text)
    head, body, tail = core_text[:left], core_text[left : right + 1], core_text[right + 1 :]
    for node, block in blocks.items():
        pattern = re.compile(rf'^(?P<indent>[ \t]*)"{re.escape(node)}": "(?P<value>.*)"(?P<comma>,?)$', re.M)
        found = list(pattern.finditer(body))
        if len(found) != 1:
            die(4, f"core.json: scenes.{node} 期望恰好 1 行，实际 {len(found)} 行（键形变了，需人看）")
        match = found[0]
        base = parse_json(f'"{match.group("value")}"', f"scenes.{node} 的值")
        if not isinstance(base, str):
            die(4, f"core.json: scenes.{node} 不是字符串")
        if BLOCK_MARK in base:  # 幂等：先剥旧块
            base = base.split(BLOCK_MARK)[0]
        if "{{" in block:
            die(4, f"块内出现 `{{{{`（会被 fill() 当变量）：{node}")
        value = json.dumps(base + "\n" + block, ensure_ascii=False)
        body = body[: match.start()] + match.group("indent") + f'"{node}": ' + value + match.group("comma") + body[match.end() :]
    text = head + body + tail
    parse_json(text, "生成的 core.json")  # 结构自检：写坏立刻炸，不落半成品
    return text


def main(argv: list[str]) -> int:
    check = False
    signature = "read"
    readset_flag = False
    root = None
    args = list(argv[1:])
    while args:
        arg = args.pop(0)
        if arg == "--check":
            check = True
        elif arg == "--readset":
            readset_flag = True
        elif arg.startswith("--signature="):
            signature = arg.split("=", 1)[1]
            if signature not in ("read", "none"):
                die(2, "用法错误：--signature 只接受 read | none")
        elif arg == "--root":
            if not args:
                die(2, "用法错误：--root 需要参数")
            root = args.pop(0)
        elif arg in ("-h", "--help"):
            sys.stdout.write(USAGE)
            return 0
        else:
            die(2, f"用法错误：未知参数 {arg}")
    if root is None:
        root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    root = os.path.abspath(root)
    ctx_dir = os.path.join(root, "docs/context")
    scenes_path = os.path.join(ctx_dir, "scenes.json")
    core_path = os.path.join(root, ".ygg/mods/agent-seed/data/texts/core.json")
    if not os.path.isdir(ctx_dir) or not os.path.isfile(core_path):
        die(4, f"输入不可读：{ctx_dir} 或 {core_path} 不存在")

    if not os.path.isfile(scenes_path):
        # r1 未落地 / 已回滚：一个字节都不写。但若块还在，说明回滚顺序错了（stale）。
        stale = 1 if BLOCK_MARK in read_text(core_path) else 0
        if check and stale:
            sys.stderr.write("残留：scenes.json 已不在，但 core.json 仍带 [READ FIRST] 块（回滚顺序错）\n")
        print(f"scenes=n/a;blocks=0;missing=[];stale={stale};drift=0;readset=n/a;ok={0 if (check and stale) else 1}")
        return 1 if (check and stale) else 0

    table = parse_json(read_text(scenes_path), scenes_path)
    if not isinstance(table, dict):
        die(4, "scenes.json 顶层必须是对象")
    blocks, missing = compose(ctx_dir, table, signature)
    current = read_text(core_path)
    expected = patch_core(current, blocks)
    drift = 0 if expected == current else 1

    # r4：readPlan / readGate 生成区（ver 台账与文档头**同一批**产出；缺席 = 判据 inert）
    readset, readset_missing = build_readset(ctx_dir, table)
    missing.extend(readset_missing)
    config_path = os.path.join(root, CONFIG_REL)
    config_text = read_text(config_path) if os.path.isfile(config_path) else ""
    present, key_drift = False, []
    if config_text:
        try:
            current_config = json.loads(config_text, object_pairs_hook=OrderedDict)
        except json.JSONDecodeError as err:
            die(4, f"{CONFIG_REL} 不是合法 JSON：{err}")
        present = any(key in current_config for key in READSET_KEYS)
        if present:
            for key in READSET_KEYS:
                key_drift.extend(diff_keys(readset.get(key, {}), current_config.get(key, {}), key))
    readset_state = "absent"
    if present:
        readset_state = "drift" if key_drift else "present"
    elif readset_flag and not check:
        readset_state = "present"

    ok = 1 if (not drift and not key_drift and not missing) else 0
    print(
        f"scenes={len(plan_of(table))};blocks={len(blocks)};missing=[{','.join(missing)}];stale=0;"
        f"drift={drift};readset={readset_state};ok={ok}"
    )

    if check:
        if drift:
            sys.stderr.write("漂移：core.json 的 scenes.* 块与 docs/context 固定头不一致（重跑本脚本可刷新）\n")
        if key_drift:
            # fail-closed 必须点名到键（r4 用户约束①②）：先说键，再说怎么修。
            for key in key_drift:
                sys.stderr.write(f"漂移：{CONFIG_REL} 的 {key} 与文档头不一致（重跑本脚本可刷新）\n")
        for item in missing:
            sys.stderr.write(f"缺件：{item}（scenes.json 指向的文档不存在，或缺固定头 / 头里 id 与表不符）\n")
        return 0 if ok else 1

    if drift:
        write_text(core_path, expected)
    if readset_flag and config_text:
        merged = patch_readset(config_text, readset)
        if merged != config_text:
            write_text(config_path, merged)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
