// runner.ts — 真实子进程 runner（playtest / lint 门共用）。
// 从 cli.ts 拆出（A2）：dev 生成端点（client-web 的 /__generate）与 CLI 需要同一
// 实现——cli.ts 顶层带「主模块自执行」判据（import.meta.url === argv[1]），被别处
// import 时会触发副作用，故把无副作用的 runner 单独成文件。

import { execFile } from "node:child_process";
import { promisify } from "node:util";
import type { CommandRunner } from "./types.js";

/** 真实子进程 runner（非零退出 = 门结果，ENOENT = 环境错误）。 */
export function execRunner(): CommandRunner {
  const run = promisify(execFile);
  return {
    async run(command, args, options) {
      try {
        const { stdout, stderr } = await run(command, args, {
          cwd: options.cwd,
          maxBuffer: 16 * 1024 * 1024,
        });
        return { exitCode: 0, stdout, stderr };
      } catch (error) {
        const err = error as { code?: number | string; stdout?: string; stderr?: string };
        if (typeof err.code === "number") {
          return { exitCode: err.code, stdout: err.stdout ?? "", stderr: err.stderr ?? "" };
        }
        throw error;
      }
    },
  };
}
