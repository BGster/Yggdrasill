# @ygg/client-web — Yggdrasill M0 叙事面板

游戏 + 平台「Yggdrasill」M0 里程碑的 Web 表现层：React 叙事面板，装载
`packages/mods/test-adventure` 资源包，经 `crates/ygg-wasm` 的 wasm 绑定
驱动内核（A1 JSON 信封契约，见 `src/kernel.ts`）。

## 内核源选择

- 默认：**真实 wasm**（`src/kernel-wasm.ts` → `crates/ygg-wasm`，构建产物
  `pkg/` 不入 git，先跑 `./scripts/build-wasm.sh`，见仓库根 AGENTS.md §五）；
- wasm 装载失败：自动降级 **回放内核**（`ReplayKernel`，逐字回放
  `src/fixtures/adventure-transcript.json`），页面顶部落可见横幅；
- `?mock=1`：强制回放模式（离线开发 / 确定性验证）。

## 生成 Mod（`?mod=generated/<slug>`）

`?mod=generated/<slug>` 载入 `packages/mods/generated/<slug>/`（pipeline 产物，
gitignored，仅本地 dev，D-6）：

- 既有 slug：`import.meta.glob` 静态图直取（模块转换期收集）；
- **dev server 运行期新生成的 slug**：vite 6 glob 不重扫新增文件（M1 spike
  实证）——由 dev middleware 兑接（`vite.plugin.mods.ts`，`apply: "serve"`）：
  `GET /__generated-mod/<slug>` 按需读盘返回 wire 契约 JSON，客户端经
  `assembleModBundle` 组装（wire 类型单源 = `src/mod-loader.ts` 的
  `GeneratedModFiles`），无需重启 dev server；
- 缺失/不完整 slug（middleware 404）→ `GeneratedModError` 降级横幅 + 回退
  默认冒险，与 glob 路径共用同一降级面；
- 生产构建不挂 middleware（`generated/` 不入包），加载路径不变。

## 命令

```bash
pnpm install                     # 仓库根执行（pnpm workspace）
pnpm --filter @ygg/client-web dev        # 开发服务器
pnpm --filter @ygg/client-web test       # vitest（jsdom；纯逻辑 + UI 路径）
pnpm --filter @ygg/client-web typecheck  # tsc --noEmit
pnpm --filter @ygg/client-web build      # vite build → dist/（wasm 构建后）
```

wasm 依赖路径（真实内核装载 / dispatch / 存读档）由 node smoke
（`node crates/ygg-wasm/smoke.mjs`）与浏览器活链验证承担，vitest 不覆盖。

## 部署（GitHub Pages 预案——启用前不生效）

部署预案已就位：`.github/workflows/deploy-pages.yml`（push 到 master 触发：
构建 wasm → 构建 `client-web` → `dist/` 以 Pages artifact 部署；
`base: "./"` 相对路径，项目站 `/<repo>/` 前缀安全）。

- **一次性仓库设置（启用前提）**：Settings → Pages → Build and deployment
  → **Source = GitHub Actions**。
- **启用属用户显式批准动作**：工作流文件本身存在无害——在上述设置完成前，
  不发生任何部署。首次部署完成后由 Pages 环境回填访问 URL。
- 部署只认 master（与仓库 AGENTS.md 部署纪律一致：非 master 内容不部署）。
