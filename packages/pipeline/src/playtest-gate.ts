// playtest-gate.ts — 无头走查子进程门（FR-6 / D-8 / D-12）：候选五件套物化
// 到临时目录（D-15：真实 outDir 不在门期落盘）→
// `cargo run -q -p ygg-sandbox --example playtest -- <dir> --max-steps 64`
// → 解析 stdout JSON 报告。报告 errors 与 reached 语义见 example 头注契约。

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { emitModBundle } from "./emit.js";
import type { CommandRunner, GateIssue, ModCandidate } from "./types.js";

export const PLAYTEST_COMMAND = [
  "cargo",
  "run",
  "--quiet",
  "-p",
  "ygg-sandbox",
  "--example",
  "playtest",
  "--",
] as const;
export const PLAYTEST_MAX_STEPS = 64;

interface PlaytestReport {
  ok: boolean;
  steps: number;
  reached: string | null;
  narrativeSample: string[];
  warnings: string[];
  eventsProcessed: number;
  errors: string[];
}

export async function playtestGate(
  runner: CommandRunner,
  cwd: string,
  candidate: ModCandidate,
): Promise<GateIssue[]> {
  const tempRoot = mkdtempSync(join(tmpdir(), "ygg-playtest-"));
  try {
    const { dir } = emitModBundle(tempRoot, safeDirName(candidate.story.slug), candidate);
    const result = await runner.run(
      PLAYTEST_COMMAND[0],
      [...PLAYTEST_COMMAND.slice(1), dir, "--max-steps", String(PLAYTEST_MAX_STEPS)],
      { cwd },
    );
    let report: PlaytestReport | null = null;
    try {
      report = JSON.parse(result.stdout) as PlaytestReport;
    } catch {
      report = null;
    }
    if (report === null || typeof report.ok !== "boolean") {
      return [
        {
          gate: "playtest",
          file: "mod.lua",
          code: "INTERNAL",
          severity: "error",
          message: `playtest 输出不是合法 JSON 报告（exit ${result.exitCode}）：${result.stderr.slice(0, 400)}`,
        },
      ];
    }
    const issues: GateIssue[] = report.errors.map((message) => ({
      gate: "playtest" as const,
      file: "mod.lua",
      code: "PLAYTEST",
      severity: "error" as const,
      message,
      hint: `reached=${report.reached ?? "n/a"} steps=${report.steps}`,
    }));
    if (report.ok) {
      return issues;
    }
    if (issues.length === 0) {
      issues.push({
        gate: "playtest",
        file: "mod.lua",
        code: "PLAYTEST",
        severity: "error",
        message: `走查未达验收锚（reached=${report.reached ?? "n/a"} steps=${report.steps}）`,
        hint: "硬门 = 装载成功 ∧ 全程无错误信封 ∧ 序章叙事非空 ∧ 至少一条路径推进至 choice/结局",
      });
    }
    return issues;
  } finally {
    rmSync(tempRoot, { recursive: true, force: true });
  }
}

function safeDirName(slug: string): string {
  return slug.replace(/[^a-z0-9-]+/gi, "-") || "candidate";
}
