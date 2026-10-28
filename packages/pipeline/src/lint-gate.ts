// lint-gate.ts — ygg-lint 子进程门（FR-5 / D-8 / D-14）：mod.lua 文本先落
// 临时文件，`cargo run -q -p ygg-lint --bin ygg-lint -- --json <file>`，解析
// stdout JSON 诊断数组。退出码 0/1 都是合法门结果（1 = 有 error 级诊断）；
// 2 或输出不可解析 = 工具面故障（INTERNAL issue，重试不救）。

import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { CommandRunner, GateIssue } from "./types.js";

export const LINT_COMMAND = [
  "cargo",
  "run",
  "--quiet",
  "-p",
  "ygg-lint",
  "--bin",
  "ygg-lint",
  "--",
  "--json",
] as const;

interface LintDiagnostic {
  file: string;
  line: number;
  column: number;
  code: string;
  severity: "error" | "warning";
  message: string;
  fix?: string;
}

function internal(message: string): GateIssue {
  return { gate: "lint", file: "mod.lua", code: "INTERNAL", severity: "error", message };
}

export async function lintGate(
  runner: CommandRunner,
  cwd: string,
  modLua: string,
): Promise<GateIssue[]> {
  const tempDir = mkdtempSync(join(tmpdir(), "ygg-lint-"));
  try {
    const tempFile = join(tempDir, "mod.lua");
    writeFileSync(tempFile, modLua, "utf8");
    const result = await runner.run(LINT_COMMAND[0], [...LINT_COMMAND.slice(1), tempFile], {
      cwd,
    });
    if (result.exitCode !== 0 && result.exitCode !== 1) {
      return [internal(`ygg-lint 工具失败（exit ${result.exitCode}）：${result.stderr.slice(0, 400)}`)];
    }
    let diagnostics: LintDiagnostic[];
    try {
      diagnostics = JSON.parse(result.stdout) as LintDiagnostic[];
      if (!Array.isArray(diagnostics)) {
        return [internal("ygg-lint --json 输出不是诊断数组")];
      }
    } catch {
      return [internal("ygg-lint --json 输出不是合法 JSON")];
    }
    // D-11 / bin 退出码契约：仅 error 级诊断计为门失败；warning 放行（人读
    // 报告面由 ygg-lint codespan 渲染承载，不进管线失败集）。
    return diagnostics
      .filter((diagnostic) => diagnostic.severity === "error")
      .map((diagnostic) => ({
        gate: "lint" as const,
        file: "mod.lua", // 包内相对名（管线↔修复回路的文件面口径）。
        line: diagnostic.line,
        column: diagnostic.column,
        code: diagnostic.code,
        severity: diagnostic.severity,
        message: diagnostic.message,
        hint: diagnostic.fix,
      }));
  } finally {
    rmSync(tempDir, { recursive: true, force: true });
  }
}
