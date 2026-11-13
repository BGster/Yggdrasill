# Yggdrasill · serve 观察面契约

> **定位**：只读观察台：CLI 形态、四级面板形态、GET API 总表、pack.json panel 段、SSE 帧协议与安全边界。
> **replaces**：DESIGN §3.7（L606–732）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../README.md) · **相关**：[mod-package.md](mod-package.md) · [cli-daemon.md](cli-daemon.md)
> **搬运**：以下正文自 DESIGN.md L606–L732 逐字迁移，不重写语义。

### 3.7 serve 观察面契约（只读观察台 + 注册表化观察根 + 内置面板页 + mod 面板 iframe 托管）

> r1 · rev2（注册表化）。`serve` 是继单发 `--event` / 交互 / `--daemon` 之后的**第四模式**。
> r1 起为**空根启动 + 运行时 API 注册**：`--runs-root` 旗标与 `~/.config/ygg/serve.json`
> 根集链退役；观察根经 `POST /api/roots` 注册（runs 父目录或单个 run-root 均可）、
> `DELETE /api/roots` 注销，注册表仅进程内有效（重启为空）。面板默认只读，
> gate/accept 类人类门禁（`proposal.accept` / `gate.approve` / `coordinate.submit` /
> `verify.submit` / `review.note`）在 serve 面上**不存在写路径**——数据端点只登记 GET，
> 唯一例外 = `/api/roots` 注册面（POST/DELETE；**进程内观察配置**，不写存档、不写
> mod KV、不写 run.json/coordination、不建目录——与局状态写严格对象分离，AGENTS §0b）。

#### 3.7.1 CLI 形态与观察根注册表

```text
egg serve [--port <1..65535>] [--panel-dir <目录>] [--poll-ms <ms>]
```

- **空根启动**：不携带任何观察根；`--runs-root` 旗标已退役（用法错误退出 2，
  文案指向 POST 注册）。`~/.config/ygg/serve.json` 存在时启动打印迁移指引
  （列出旧 roots 与对应注册命令，**不自动导入**——「面板在观察什么」必须显式注册）。
- **观察根注册表**（进程内 `RwLock<Vec<RootEntry>>`；重启为空）：

  | 端点 | 载荷 | 语义 |
  | --- | --- | --- |
  | `POST /api/roots` | `{"root": "/abs/…"}` | 注册。**形状校验**：绝对路径（相对/`~` = 400）、`is_dir` 必须为真（404，零创建）、上限 32 根（409 `registry_full`）。**幂等**：同一 canonical 根重复注册 = `added:false`。注册时按 `<root>/run.json` 存在性归类：`singleRun=true`（单局根，只观察该局）或父目录根（枚举其子目录各局）。成功 → 根级 SSE 变更帧 |
  | `DELETE /api/roots?root=<绝对路径>` | query | 注销。**幂等**：缺席 = `removed:false`。只从注册表摘除（不删盘上任何字节）→ 根级变更帧 |
  | `GET /api/roots` | — | `{"roots":[{"root","singleRun"}], "cap":32}`（单实例探针的正面证据面） |

- `--port` 缺省 7800；恒绑定 **`127.0.0.1` 常量**（无旗标）；被占用 → 报错退出 1（不自动
  递增）。单实例语义**按端口**成立：7800 是全局唯一缺省实例，显式换端口 = 显式逃生门
  （独立实例独立注册表）。
- `--poll-ms` 缺省 700，钳制 [200, 5000]（watcher 变更检测周期）。
- `--panel-dir`：内置两级列表页的盘上覆盖（开发期改 HTML 刷新即生效；缺失文件 404）。
- 与其它全部模式互斥（`usage_error`，退出 2）；不接受位置参数 / new / open。
- **单实例探针（零锁文件，端口即事实）**：启动前手写只读 `GET /api/roots`（1s 超时）——
  自家实例（应答含 `roots` 数组）→ 幂等复用（打印面板 URL **退出 0**，nohup 脚本安全；
  **不再比较根集**——注册表由调用方 POST 扩容，启动面无根集可比较）；旧代实例
  （无 `/api/roots` 端点，404/405）→ **退出 1** + 重启指引；无自家实例 → 照常 bind
  （探针↔bind 竞态由 bind 失败路径兑住）。
- 先起 serve 再注册再开局**合法**（一级列表动态出现新局）；局收口后继续提供终局数据；
  run 目录被外部删除 → 相关 API 404 + watcher 摘除，serve 不退出。

#### 3.7.2 四级形态与宿主/mod 边界

| 级 | 归属 | 内容 | 数据源 |
| --- | --- | --- | --- |
| 一级：历史局（mod 分组列表） | egg 原生 | `#/runs`；按 `meta.mod` 分组的实例枚举（组按组内最新 `updated_at` 降序） | **注册表扫描**（行带 `root` 标识）+ 存储探针（前端对 `/api/runs` 分组推导，serve 零改动） |
| 二级：mod 实例列表 | egg 原生 | `#/mods/<key>`；该 mod 的 runs 枚举 | 同上（前端按 key 过滤） |
| 三级：局详情 | egg 原生 | `#/runs/<id>`；存档折叠段 + 投影区（见下注） | `ygg_store::observe` 只读观察面 |
| 四级：存档视图 | **mod 自行维护** | mod 自带 UI（详情页存档折叠段内 iframe 入口） | 存档数据 API（裸数据） |

> **默认视图 = 最新局详情（内置页展示投影，只读；裁决见 CONTEXT `D-MA17`）**：`#/` 与空 hash
> 渲染 `/api/runs` 首行的局详情——局头条（宏观阶段/会话活性/批次/模式/退役徽章）、
> `coordinate → review(批 n) → done` 三节点主流程状态条（r2 rev2 后 intake 节点已注销）、需求卡（分 stage 决策产物行、
> 被拒灰显）、近期决策时间线（尾窗 500 内、决策事件族、倒序前 20）、本局存档折叠段。
> 投影数据源全部是既有只读端点：`/saves/latest`（KV 键族）+ `/events`（尾窗两步取）+ `/artifacts`；
> 对无 `req.*` 键族的 mod 无害退化为通用摘要（判据见 D-MA17）。
>
> **已知限制（裁决 R14）**：内置面板徽章的 `stageByStatus` 映射按旧状态名建档，D-MA23 剧情双层重写后旧名不再出现，徽章将退化为默认样式——接受为已知外观限制（宿主资产 `assets/panel` 是宿主代码红线，本轮不改），宿主侧跟进归后续决策。

宿主服务面对第四级**仅托管静态文件 + 裸数据 API**，零解析零渲染任何 mod 领域语义（P11 字面边界不动）；展示投影只发生在内置页前端资产层。

hash 路由（纯前端，URL=状态、可收藏可刷新）：`#/`（与空 hash）= 最新局详情（默认视图，SSE/轮询重取时自动跟随新最新局）；`#/runs` = 历史局列表；`#/mods/<encodeURIComponent(meta.mod)>` = 实例列表；`#/runs/<id>` = 指定局详情（与默认视图共用同一渲染器；非最新局显示「← 最新局」返回）。返回导航由前端按当前路由推导（`#nav-back` 空容器 + 常驻「历史局」入口；历史局自身不渲染任何入口，防 no-op；局详情父级解析失败——截断/已删——降级为「← mod 列表」），不由点击历史推导——渲染即有意义，结构上不可能 no-op。`meta.mod` 缺失/解析失败的行归键 `-` 的「未知 mod」组，诚实呈现、不做客户端别名合并（别名归宿主 serve 侧，只服务面板托管，不服务分组）。

#### 3.7.3 API 契约总表（数据面全部 GET；/api/roots 白名单 POST/DELETE；未知路径 → 404）

| 路由 | 语义 | 响应形状（关键字段） |
| --- | --- | --- |
| `GET /` | 内置面板页 `index.html`（默认视图=最新局详情，见 §3.7.2 注） | text/html |
| `GET /assets/<name>` | 内置页静态资产（白名单 `app.js` / `style.css`） | 按扩展名映射 + nosniff |
| `GET /api/runs` | **第一级**：runs 枚举（多根） | `{version, generatedAt, roots:[<canonical 绝对路径>], truncated?, errors?:[{root,message}], runs:[{runId, root:<绝对路径>, meta:<run.json 原样>, coord:<原样\|null>, backend, probe:{…}, panel:{available,url?,reason?}, metaError?}]}`（updated_at 降序；封顶 200；个别根不可读 → `errors` 记录、其余根照常出行，整体仍 200） |
| `GET /api/runs/<id>/saves?limit=` | **第二级**：存档列表 | `{runId, backend, generation, saves:[{id,generation,seq,createdAt,bytes,latest}], panel:{available,url?}}`（limit 缺省 50 封顶 200；JSON 局恒单行 `latest`——单档后端的物理事实） |
| `GET /api/runs/<id>/saves/<sid>` | **存档数据 API**：原始快照字节 | `200 application/json` + `X-Ygg-Run/X-Ygg-Save-Id/X-Ygg-Generation/X-Ygg-Seq/X-Ygg-Created-At/X-Ygg-Bytes`；body = 存档原字节（零包装零重序）；`sid` ∈ `latest \| [0-9]+`（否则 400） |
| `GET /api/runs/<id>/events?since=&limit=` | 事件日志分页（seq 升序） | `{runId, generation, latest, since, records:[{seq,event,payload,ok,timestamp,ending}]}`；`limit` 缺省 200 封顶 500；`payload` 原文透传（`actor=`/`actor_session=` 段随 r3 自动出现） |
| `GET /api/runs/<id>/artifacts` | 当前代制品元信息 | `{artifacts:[{path,bytes,sha256,updatedAt}]}`（path 升序） |
| `GET /api/runs/<id>/artifacts/<path..>` | 制品原文 | 原字节（`validate_artifact_path` 门复用；读侧封顶 1 MiB → 413） |
| `GET /api/stream` | SSE 变更流 | `text/event-stream`（§3.7.5） |
| `POST /api/roots` | **注册观察根**（唯一写面例外：进程内观察配置，非局状态） | `{"ok":true,"added":<bool>,"root","singleRun","roots":[…]}`（幂等；400 形状 / 404 不存在 / 409 满） |
| `DELETE /api/roots?root=` | 注销观察根 | `{"ok":true,"removed":<bool>,"roots":[…]}`（幂等；不删盘上字节） |
| `GET /api/roots` | 注册表自描述 | `{"roots":[{"root","singleRun"}],"cap":32}` |
| `GET /panel/<run-id>/[rel]` | mod 面板托管 | 文件字节（§3.7.4；未声明/不可达 → 404 JSON 带 reason） |

run 寻址统一跨根定位 `locate_run`：0 命中 → 404（数据端点 `run_not_found` / `/panel`
`panel_unavailable`，既有文案不变）；1 命中 → 服务；**>1 命中（跨根撞名）→ 409
`run_id_ambiguous`**，error 对象内附 `roots` 列表（不静默任选一份）。run-id 语法门不动，
不把根编码进 id；显式根寻址（`?root=`）留作增量接口。

错误统一 `{"ok":false,"error":{"code":…,"message":…}}`；响应一律 `Cache-Control: no-store` +
`X-Content-Type-Options: nosniff`。

#### 3.7.4 pack.json `panel` 段契约（挂进 §3.2 的 pack.json 契约旁）

- 声明：`"panel": { "entry": "panel/index.html" }`（**可选顶层段，v1 全部面 = 单字段 `entry`**；
  未来字段只能向后兼容式增量）。语法门 = 不以 `/` 开头、不含 `..`、以 `.html` 结尾；
  违规 → `panel.available=false` + 具体reason，**不是装载错误**（面板声明损坏不能炸 mod 装载）。
- 兼容性：pack.json 解析是宽松 Value 读取，旧宿主忽略未知顶层段；旧 mod 无段 → serve 侧
  `panel.available=false`（降级矩阵五态见实现测试）。面板文件**不进** `entries[]`（内核资源面）
  也**不进** `watched_files`（签名/热载面）——改面板刷新浏览器即生效。
- 托管：运行期现读 + 双守卫（`canonicalize` + 前缀校验，拦 `..` 穿越与 symlink 逃逸）。
- **信任模型（诚实声明）**：mod 面板 JS 与内置页同源（127.0.0.1 同端口）执行，信任级别 = mod
  本身（mod 代码本就持 daemon 工具执行权限）。serve 不消毒不沙箱；防护边界 = 路径双守卫 +
  只读 + localhost 绑定 + nosniff。
- **宿主不提供 SDK/桥**：mod UI 的取数契约 = 同源 `fetch('/api/runs/<id>/saves/<sid>')`（`X-Ygg-*`
  头带元数据）+ `fetch('/api/runs/<id>/events?since=..')` + `new EventSource('/api/stream')`
  （过滤 `frame.run === 本局` 后自行重取）。iframe 传参 = query（`?save=<sid>`，可收藏可刷新）。

**内核存档信封（mod UI 作者依赖面）**：存档本体 = `{engine_version, entities, format_version,
kv:{entries,revision}, pending_choice}`。mod 自己的状态在 `kv.entries`；`format_version` 判版本。
`meta.summary`（`contract.summaryEvent` 派发面）是**内存态派生产、不持久化**——serve 不装载内核故
不供；需要 profiles 的 mod UI 应从自己的存档状态取数（mod 自己写入 kv 的注册表副本）。

#### 3.7.5 SSE 帧协议

- 单一端点 `/api/stream`；watcher 是唯一变更源（轮询 runs-root：名单比对 + 每 run 的
  run.json/coordination.json stat + `save_probe`）。检测到变更 → 全局 `version += 1` → 广播。
- 帧：`event: change` + `data: {"version":<n>,"run":"<runId>","root":"<绝对路径\|null>"}`
  （r9 加性 `root` 字段点名变更局所属根；根级变更 `"run":null` 且 `root` 指明变化的根；
  单根时代语义不变——root 恒为该根）。
  **没有数据载荷**——客户端收帧后重取自己正在看的端点；每 poll 每 run 至多一帧（天然合流）。
- 保活：`: ping` 每 15s；断线自动重连 + `onerror` 降级 2s 轮询（同一批 API）。无
  `Last-Event-ID`（变更帧无连续性语义）。通知 ≤ 一个 poll 周期（最终一致通知，读恒为当下事实）。

#### 3.7.6 安全边界与线程模型

- 绑定 `127.0.0.1` 常量（单测断言字面量）；**方法白名单**：数据面 GET-only（非 GET → 405），
  唯一例外 `/api/roots` 的 POST/DELETE（注册表维护；请求体封顶 8 KiB）；无凭据面；
  **单实例探针 = 只读 GET**（启动前 `GET /api/roots` 一次，1s 超时；**零锁文件**——端口即事实，
  无盘上状态，复用/拒绝判定见 §3.7.1）；**零写不变式跨注册面成立**：注册/注销只改进程内
  `RwLock` 注册表，`registry_writes_nothing_on_disk` 测试锁（操作前后目录树逐字节一致）；
  watcher 注册表扫描仍是纯 stat + `save_probe`（指纹键 = (根, 局) 对），409/404/复用路径
  零盘上副作用；解析健壮性（32 KiB 头上限 / 5s 超时 / 64 连接封顶超出 503 / 路径白名单 /
  query 数值钳制 / run-id 语法门）；无 CORS 头；`/panel` 双守卫 + nosniff；观察面零写
  （`query_only` + 无 migrate + 先探测后连接，`observe_creates_nothing` /
  `observe_writes_nothing` 测试锁）。
- **零内核**：serve 不装载 ModBundle/kernel/mlua、不构造 `Session`——数据面只剩文件 stat 与轻量
  SQL（毫秒级），发布/订阅快照机制失去存在理由；mlua 非 Send 约束以「进程无 Lua」最强成立。
  物理锁：`serve_source_has_no_write_path`（生产面源扫描禁写路径）。
- 线程编排：主线程 accept（每连接一线程，封顶 64）；watcher 线程纯变更检测；SSE 每连接
  `wait_timeout` 等广播。共享可变状态 = `version` 广播对（Mutex + Condvar）+ `roots`
  注册表（RwLock，读多写少；锁不跨 IO——先克隆快照再扫盘）。

---

