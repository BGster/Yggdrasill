// locate.ts — JSON Pointer（RFC 6901）→ pretty 文本 (line, column) 映射
// （schema 门错误定位；资源保持 pretty-print 使行映射稳定，research Q1
// 既定形态）。M1 资源是纯对象嵌套（无数组元素），只实现对象 token；同名键
// 跨层级时的 includes 匹配为已知近似（生成键集为普通文本，概率极低）。

export interface SourcePosition {
  line: number;
  column: number;
}

/** pointer "/scenes/tavern" → 2 空格缩进 JSON 文本中的键名行 1 起位置。 */
export function locatePointer(jsonText: string, pointer: string): SourcePosition | null {
  const tokens = pointer
    .split("/")
    .slice(1)
    .map((token) => token.replace(/~1/g, "/").replace(/~0/g, "~"));
  if (tokens.length === 0) {
    return { line: 1, column: 1 };
  }
  const lines = jsonText.split("\n");
  let searchFrom = 0;
  for (let depth = 0; depth < tokens.length; depth++) {
    const needle = `"${escapeJsonKey(tokens[depth]!)}":`;
    let found = -1;
    for (let i = searchFrom; i < lines.length; i++) {
      if (lines[i]!.includes(needle)) {
        found = i;
        break;
      }
    }
    if (found === -1) {
      return null;
    }
    searchFrom = found + 1;
    if (depth === tokens.length - 1) {
      const line = lines[found]!;
      return { line: found + 1, column: line.indexOf(needle) + 1 };
    }
  }
  return null;
}

function escapeJsonKey(key: string): string {
  // JSON.stringify 的键转义口径（生成键集为普通文本，控制字符场景为已知近似）。
  return JSON.stringify(key).slice(1, -1);
}
