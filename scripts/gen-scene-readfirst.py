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
#   scripts/gen-scene-readfirst.py [--check] [--signature=read|none] [--root <dir>] [--help]
# 输出：
#   scenes=<n>;blocks=<k>;missing=[...];stale=<s>;drift=<d>;ok=<0|1>
# 退出码：0 = ok（或 write 模式成功）／1 = --check 发现漂移或缺件／2 = 用法错／4 = 输入不可读

import json
import os
import re
import sys
from typing import Any, NoReturn

NODES = ("prologue", "charter", "coordinate", "settle", "release")
BLOCK_MARK = "\n  [READ FIRST · "
TRIGGER_MAX = 60
USAGE = """scripts/gen-scene-readfirst.py [--check] [--signature=read|none] [--root <dir>]
  --check            只比对不写盘；漂移/缺件 → ok=0 且 exit 1
  --signature=read   块尾打可抄的收据命令 config.set read.<id>=<ver>（缺省）
  --signature=none   块尾改印「本回合汇报里写明 reads: …」（r3 未落地时的过渡形态）
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
    root = None
    args = list(argv[1:])
    while args:
        arg = args.pop(0)
        if arg == "--check":
            check = True
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
        print(f"scenes=n/a;blocks=0;missing=[];stale={stale};drift=0;ok={0 if (check and stale) else 1}")
        return 1 if (check and stale) else 0

    table = parse_json(read_text(scenes_path), scenes_path)
    if not isinstance(table, dict):
        die(4, "scenes.json 顶层必须是对象")
    blocks, missing = compose(ctx_dir, table, signature)
    current = read_text(core_path)
    expected = patch_core(current, blocks)
    drift = 0 if expected == current else 1
    ok = 1 if (not drift and not missing) else 0
    print(f"scenes={len(plan_of(table))};blocks={len(blocks)};missing=[{','.join(missing)}];stale=0;drift={drift};ok={ok}")

    if check:
        if drift:
            sys.stderr.write("漂移：core.json 的 scenes.* 块与 docs/context 固定头不一致（重跑本脚本可刷新）\n")
        for item in missing:
            sys.stderr.write(f"缺件：{item}（scenes.json 指向的文档不存在，或缺固定头 / 头里 id 与表不符）\n")
        return 0 if ok else 1

    if drift:
        write_text(core_path, expected)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
