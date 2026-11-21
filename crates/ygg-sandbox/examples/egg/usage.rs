use super::*;

pub(crate) fn usage(code: i32) -> ! {
    let text = "\
用法（r1 语义二分：new = 开新局；open = 读档续跑；存档目录即身份）:
  egg new  --mod <mod目录> [--in <runs 父目录>] [--store <后端>] [--force]
           [--event <名称> [--payload <JSON>]] [--raw]
                                                                新局：自动生成 run-id，建 <in>/<id>/
                                                                （--in 缺省 ./runs）；--event 可同调用开局
  egg open <run-root> [--at g<代>] [--event <名称> [--payload <JSON>]] [--raw]
                                                                读档/单发：取最新存档点续跑；
                                                                --at g<代> 回退到指定存档点（仅 sqlite 局）
  egg open <run-root> [--watch] [--drive <方式>]                交互附着（人 / 脚本驱动）
  egg open <run-root> --settle                                 局末结算（结算已收口的局）
  egg open <run-root> --requirement '标题|描述'                 异步补需求（只追本局后端的 inbox）
  egg open <run-root> --daemon                                 常驻守护模式（JSON-RPC over stdio）
  egg open <run-root> --wait-tools [--tool-timeout <ms>]        等在飞工具落地后 drain 一次 inbox
  egg open <run-root> --artifact-put <artifacts/名>=<源文件>    制品写入本局制品面
                                                                （载荷含 `=` 与 `/`，shell 里必须整段加单引号：
                                                                 '--artifact-put artifacts/x.md=/tmp/y.md'）
  egg open <run-root> --artifact-get <artifacts/名>             制品原文直出 stdout
  egg serve [--port <端口>] [--panel-dir <目录>] [--poll-ms <ms>]
                                                                常驻只读观察台（空根启动，默认 127.0.0.1:7800）
                                                                观察根经运行时 API 注册：
                                                                POST /api/roots {\"root\":\"<绝对路径>\"}

退役旗标（fail-closed 可读拒绝，不静默映射）:
  --run-id       → run-root 即身份：new 自动生成；open 位置参数给出
  --runs-root    → new 用 --in；serve 用 POST /api/roots 注册观察根
  --resume       → open 自动取最新存档点；回退用 --at g<代>

选项:
  --mod <目录>       Mod 目录（五件套：pack.json / manifest.json / mod.lua /
                     schemas/*.json / data/*.json）。new 必填；open 可省略
                     （改读 run.json 记录的本局 mod 路径，两者都没有则报错）
  --in <目录>        new 的 runs 父目录（默认 ./runs；自动 id 落在其下）
  --packages <目录>  能力包根显式覆盖（缺省 = 环境变量 $YGG_PACKAGES_ROOT >
                     <mod>/../../packages/libs 布局推导；仅装载路径消费）
  --force            new 的显式重置许可：目录已有旧产物时按新局重置（归档留证据）
  --at g<代>         open 的回退存档点（如 g0 / g3；代 = sqlite generation）。
                     取该代末快照建新代续跑，旧代证据全留库；JSON 局拒绝（无代概念）
  --watch            交互模式下每步检查 mod 目录 mtime 变化并热载
  --event <名称>     单发模式：派发一个事件后退出（stdout 回执：缺省人读文本，
                     --raw 输出单行 JSON 契约）
  --payload <JSON>   事件 payload（缺省 = Mod 契约的 agent.defaultPayload；必须 JSON 标量）
  --raw              单发回执输出单行 JSON 信封（机器契约）。仅 --event 单发路径生效。
                     解析顺序 = --raw > 环境变量 YGG_FORMAT（text|json）> 缺省 text
  --actor <id>       提交者身份（Agent 工具句柄，与 exec=/profile handle 同一命名空间；
                     ≤64 字节、无空白、无 `|`）。解析链 --actor > 环境变量 YGG_ACTOR
                     > daemon 请求字段。声明了 actor 段的路由会由宿主注入 `|actor=<id>`
                     并拦截调用方自带的同名段（ACTOR_FORGED）；携带身份的成功写命令
                     /成功派发会先到先得认领 run.json 的 opener_actor（不可变）
  --drive <方式>     交互模式推方式（默认 stdin；宿主不替你做决策）：
                       stdin      读 stdin：数字=选择 / event <名称> [JSON] / s / l / i / config / q
                       cmd:<路径> 不读 stdin：每个待选动作交给决策命令（stdout 单行 `N` 或
                                  `HUMAN[:理由]`）；HUMAN / 超时 / 非零退出一律安全暂停并退出 0
                                  （环境变量 YGG_DECIDE_CMD 可临时覆盖路径）
                       none       只打印当前回执就退出（不推进、不读 stdin）
  --requirement <串> 只把 '标题|描述' 追加到本局后端的 inbox 后退出
                     （json 局 = <run 目录>/inbox.jsonl；sqlite 局 = run.sqlite 的 inbox 表）；
                     不读写 autosave，可在别一进程跑本局时安全并发调用
                     （旗标是否可用由 Mod 的 inboxRoutes.<kind>.write 声明）
  --amend <串>       --split <串>    --merge <串>    --conflict <串>
  --resolve <串>     --proposal <串>
  --tool-result <串> 只把工具结果 wire payload 追加到本局后端的 inbox 后退出
                     （kind / 字段 / 写入形态由 Mod 的 inboxRoutes 声明；场景 2 的补票入口）
  --wait-tools       等本局在飞工具全部落地（结果由脱体 worker 写 inbox）后消费一次
                     inbox 并打印回执；**不重置本局、不推进状态**（只观察）
  --tool-timeout <ms> --wait-tools 的总等待上限（缺省 120000）
  --artifact-put <artifacts/名>=<源文件>
                     把源文件写进本局后端的制品面（内容寻址，≤256 KiB），
                     stdout 打印 {\"ok\":true,\"path\":…,\"bytes\":…,\"sha256\":…}
                     （不新造 run：本局必须已存在；不推进状态）
                     **shell 引号陷阱**：载荷含 `=`，不加引号会被当成位置参数而
                     报「需要恰好一个位置参数」——整段加单引号：
                     '--artifact-put artifacts/x.md=/tmp/y.md'
  --artifact-get <artifacts/名>
                     把制品原文直出 stdout（**不补换行**；缺失 = 退出 1，stdout 保持空）
   --settle           读本局后端的状态事实写 settlement.json（json 局 = autosave.json + events.jsonl）。
                     语义：结算**已收口**的局（常驻局的收口 = run.retire 武装后的退役）；
                     对未收口的常驻局预跑 = reached=open, ok=false 且退回 running（无害但不收口）
  --daemon           常驻守护模式：stdin 逐行读 JSON 请求
                     （dispatch/summary/state/vfs_put/vfs_get/vfs_list/settle/ping/shutdown），
                     stdout 逐行回 JSON。请求可带 \"id\" 字段（原样回显）；dispatch 请求可带
                     \"actor\"/\"actor_session\" 字段（提交者身份，仅在旗标与环境变量都缺席时兑底）。
                     stdout 只有协议面，人读输出一律走 stderr。
                     `settle` 与独立 --settle 同一实现并终结本局
  --harness <h>      harness 适配器：auto（缺省，按现场探测）/ pi / claude-code / codex / generic。
                     解析顺序 = --harness > 环境变量 YGG_HARNESS > <mod>/data/config.json 的
                     harness.adapter > auto；只决定 `--builtin-tool archive-sessions` 默认归档哪个
                     会话库（generic 没有会话库：记 moved=0;remaining=0），不改变任何状态判定
  --actor-session <id> 提交者的原生会话 id（不透明审计字段：事件行 → 磁盘会话文件的
                     grep 链；永不参与判定、不做 resume 寻址键；≤128 字节、无空白、无 `|`）。
                     解析链与 --actor 对称：旗标 > YGG_ACTOR_SESSION > daemon 请求字段；
                     两者可独立携带，inbox 记录盖宿主保留键 actor/actor_session（None 不写键）
serve 模式（只读观察台；数据端点 GET-only + 唯一例外 /api/roots 观察根注册，
与其它全部模式互斥）:
  （无 --runs-root）观察根在启动后经 API 注册：
    POST   /api/roots {\"root\": \"<绝对路径>\"}   注册（幂等；runs 父目录或单个 run-root 均可）
    DELETE /api/roots?root=<绝对路径>            注销（幂等；不删盘上任何字节）
    GET    /api/roots                            当前注册表
    注册表仅在当前进程内有效（重启为空）；~/.config/ygg/serve.json 已退役
    （存在时启动给迁移指引，不自动导入）
  --port <端口>      监听端口（默认 7800；恒绑定 127.0.0.1；被占用 = 报错退出 1，不自动递增）。
                     单实例语义**按端口**成立：7800 是全局唯一缺省实例，显式换端口 = 显式逃生门
  --panel-dir <目录> 内置两级列表页的盘上覆盖（开发期改 HTML 刷新即生效；缺失文件 404）
  --poll-ms <ms>     变更检测周期（默认 700ms，钟制 200..=5000；SSE 通知 ≤ 一个周期）
  全局唯一实例（零锁文件，端口即事实）：启动前只读探针 GET /api/roots——
  自家实例 → 幂等复用（打印面板 URL 退出 0，nohup 脚本安全）；
  旧版实例（无 /api/roots 端点）→ 退出 1 提示重启；无自家实例 → 照常启动。
  面板三级：① runs 列表（egg 原生）② 局内存档列表（egg 原生）③ mod 自带 UI
  （iframe 托管，pack.json 可选顶层 panel 段声明 entry，如 panel/index.html；
  egg 不解析不渲染任何 mod 领域语义，只供存档数据 API 裸数据）。
  只观察既有局：先起 serve 再注册再开局合法；gate/accept 类人类门禁在 serve 面上
  **没有写路径**，仍走 CLI 票据 / 适配器路径。
  --store <后端>     存储后端断言：sqlite（默认，<run 目录>/run.sqlite）|
                     json（兼容后端，autosave.json + events.jsonl）。run.json 的
                     \"store\" 字段是权威：与 --store 不一致即拒绝开局（零写入）；
                     旧局无该字段时按产物推断（纯 JSON 目录 → json）。
                     以 --no-default-features 构建时 sqlite 不可用，--store sqlite 报错退出。
  -h, --help         本帮助

host 契约（**必填、无默认值**，宿主代码不含任何 Mod 语义字面量）：
  <mod>/pack.json       agent{entryEvent,choiceEvent,summaryEvent,endingPrefix,
                              metaKey,rejectionPrefixes,replayPrefix,defaultPayload}
                        inboxRoutes{externalIdField,payloadField,extMarker,
                                    <kind>:{action,fields[,write]}}
  <mod>/data/config.json settlement{requireEvents,requireNoFailedSteps,
                        requireEnding,requireInboxConsumed}
                        drive{maxSteps,stallLimit,stepsPerRequirement}（正整数）
  缺段 / 缺字段 / 类型不符 / 非正整数 → 装载期失败（点名缺失字段 + 应有形状 +
  文档定位：.ygg/mods/agent-seed/README.md 的「host 契约」章节）。

内置工具（`--builtin-tool <op>`；不读 Mod、不推进状态；stdout 单行
`key=value;key=value`，失败 = stdout 空 + stderr 一行 + 退出 1；用法错 = 退出 2）：
  worktree-claim    --lane <k> --req <rN> --branch <b> [--root <d>] [--repo <d>]
                    → lane=..;req=..;branch=..;base=..;clean=1;path=..
  worktree-release  --lane <k> [--root <d>] [--repo <d>]        → lane=..;released=1
  lane-probe        --lane <k> --base <sha> --branch <b> [--root <d>] [--repo <d>]
                    → lane=..;branch=..;base=..;head=..;commits=..;clean=..;files=..
  artifact-probe    --path <artifacts/名> [--run-dir <d>]
                    → path=..;exists=<1|0>;bytes=..;sha256=..（**恒退出 0**：exists=0 是合法答案）
  verify-suite      [--mode fast|full] [--repo <d>]             → passed=1;summary=<n>/<m>
  archive-sessions  --parent <id> --batch <名> [--sessions-dir <d>] [--trash <d>]
                    → moved=..;remaining=..;dest=..（remaining>0 = 退出 1）
                    缺 --sessions-dir / YGG_SESSIONS_DIR 时按 `--harness` 适配器推导会话库：
                    pi=$HOME/.pi/agent/sessions/--<cwd_slug>--；claude-code=$HOME/.claude/projects/-<slug>；
                    codex=$HOME/.codex/sessions；目录不存在 = 0/0（dest=<适配器>:empty:<batch>）
  pack.json 里写 \"program\": \"@self\"（或 \"egg\"）即可调用——两者都解析到本可执行文件，
  因此不依赖 $PATH。本局路径用 $ENV{YGG_RUN_DIR} / $ENV{YGG_RUN_ID} / $ENV{YGG_RUNS_ROOT}
  / $ENV{YGG_MOD_DIR} 引用（宿主注入 worker 环境；.env / .env.local 不得覆写）。

自动推进的失控防护：基础步数 + 每条需求额外步数 + 连续无进展判停；阈值读
  <mod>/data/config.json 的 drive 段（必填）。r9 常驻批循环：mod 自报
  lifecycle=batchDrained（批排空且未武装退役）且 inbox 无未消费行 → 打印等待文案后
  安全暂停（exit 0）：空批绝不连点 coordinate.submit，投递 --requirement 后重跑继续。

异步需求（inbox）：每个自动动作前、stdin 每次提示前、单发每次派发前都会消费
  inbox.jsonl 的新行，按 inboxRoutes 声明派发；dispatch 成功且存档后才前移 cursor，
  重放靠 Mod 侧幂等键去重（字段名由 inboxRoutes.externalIdField 声明）；
  局已收口后仍有未消费行 → 非零退出。";
    if code == 0 {
        println!("{text}");
    } else {
        eprintln!("{text}");
    }
    exit(code);
}

/// serve 模式的互斥矩阵输入面（parse_args 局部旗标的投影；纯数据，可测）。
#[derive(Default)]
pub(crate) struct ServeFlags {
    pub(crate) settle: bool,
    pub(crate) event: bool,
    pub(crate) wait_tools: bool,
    pub(crate) inbox: bool,
    pub(crate) run_tool_worker: bool,
    pub(crate) artifact_put: bool,
    pub(crate) artifact_get: bool,
    pub(crate) daemon: bool,
    pub(crate) watch: bool,
    pub(crate) store: bool,
    pub(crate) payload: bool,
    pub(crate) harness: bool,
    pub(crate) tool_timeout: bool,
    pub(crate) drive_is_stdin: bool,
    pub(crate) positional: bool,
}

/// serve 与其它模式/旗标的互斥判定（纯函数；命中即返回人读冲突文案）。
/// serve 是常驻只读观察台：一切「单发/常驻/推进/写面」旗标都没有意义。
pub(crate) fn serve_usage_conflict(flags: &ServeFlags) -> Option<String> {
    if flags.settle
        || flags.event
        || flags.wait_tools
        || flags.inbox
        || flags.run_tool_worker
        || flags.artifact_put
        || flags.artifact_get
        || flags.daemon
        || flags.watch
        || flags.store
        || flags.payload
        || flags.harness
        || flags.tool_timeout
        || !flags.drive_is_stdin
    {
        return Some(
            "serve（只读观察台）不能与任何推进/写面旗标（--event / --settle / --watch / --store / --payload / --drive / inbox 写命令 / 制品旗标 / --daemon / --wait-tools / --harness / --builtin-tool）同时使用"
                .to_owned(),
        );
    }
    if flags.positional {
        return Some(
            "serve 空根启动（观察根经 POST /api/roots 运行时注册）：不接受位置参数 / new / open"
                .to_owned(),
        );
    }
    None
}

pub(crate) fn usage_error(message: &str) -> ! {
    eprintln!("error: {message}");
    eprintln!();
    usage(2)
}
