//! r1 模块装载内核的 **Lua 侧**：受控 `require` 闭包、模块缓存与副作用影子表。
//!
//! # 两段式布局（r3 起跨 crate，边界不变）
//!
//! - **① 纯解析面** → 独立 crate [`ygg_modules`]：名字规范化 [`normalize`]、
//!   双向路径映射 [`path_of`] / [`name_of_path`]、上限常量 [`MAX_MODULES`] /
//!   [`MAX_DEPTH`] / [`MAX_CODE_BYTES`]、装载栈 [`LoadStack`]（循环与深度判定的
//!   唯一数据结构）。**这一段零 mlua 依赖**，可脱离 Lua 状态单测，也被打包
//!   （`ygg_sandbox::pack`）与静态检查（`ygg-lint --pack`）复用——三处共用
//!   同一份判定，避免「能装载的 Mod 被 lint 判非法」这类双份口径。
//! - **② Lua 注入口**（本文件的 `binding` 子模块）：`require` 闭包、装载上下文
//!   与缓存。与 ① 的边界是**模块边界**，解析面因此拿不到 mlua 面。
//!
//! 本文件对 ① 做 `pub use` 全量再导出：`ygg_sandbox::modules::*` 的路径与语义
//! 与拆 crate 之前逐字节一致（既有调用与测试零迁移）。
//!
//! # require 语义
//!
//! - **同一 Lua 状态、同一 globals 表**执行模块 chunk：模块顶层 `local` 不
//!   跨模块可见，跨模块共享只经返回值（别的模块 require 到的就是它 return 的值）。
//! - **按规范名缓存**：同模块重复 require 只执行一次，返回同一对象（table
//!   身份一致）；模块 `return nil` 或无返回值 → 缓存 `true`，同样只执行一次。
//! - **chunk 名** = `@<相对路径>`（如 `@src/util.lua`；入口 chunk 仍是
//!   `@mod.lua`），traceback 与错误文本因此携带真实文件与行号。
//! - **循环依赖**报完整链条：`circular require: src.a -> src.b -> src.a`。
//! - **装载深度**显式判栈（[`MAX_DEPTH`]），不靠 Lua/Rust 栈溢出兜底。
//! - **调用时机不设边界**：`require` 在装载期与运行期（handler 内部）一视同仁。
//!   注册禁令是另一条约束，由 [`crate::host_api::HostApi::seal`] 单独封 `ygg.on`
//!   承担——两件事不共享开关（早期版本把 `require` 一并封在装载期，那是把
//!   「注册必须在装载期」误扩成「模块必需装载期拉齐」，代价是懒加载直接不成立）。
//! - **模块顶层是纯导出面**（[`ShadowBanned`]）：模块 chunk 执行期间 `ygg` 的
//!   禁止面（清单在 [`ygg_modules::effects`]）被原位换为拒绝桩，执行完恢复。
//!   这样「文件怎么拆」不影响 handler 派发序，重构保行为；也正因为模块纯了，
//!   `require` 才能在任何时机调用。入口 chunk **不**受此约束（入口就是装配清单）。
//! - **装载期独立燃料**（[`crate::fuel::LoadBudget`]）：模块装载消耗装载预算，
//!   不挤占单转换预算——否则模块越多、每次转换可用步数越少（隐性退化）。
//! - **错误**：装载器自判的拒绝（名字非法 / 未登记 / 循环 / 超深）带**调用点**
//!   （`src/a.lua:3: ...`）与 require 链；模块 chunk 自身的错误按 Lua 原生错误链
//!   浮出（每层 `path:line:` 前缀 + traceback 即链条）。两者最终都由驱动统一
//!   映射为 `KernelError::ModLoad`（装载期）或 handler 拒绝（运行期）。
//!   注意：Lua 侧 `pcall(require, ...)` 拿到的错误对象是 mlua 包装的 error
//!   userdata（对 Rust 侧错误一贯如此），`tostring(err)` 即人读文案；不经
//!   `pcall` 则同一文案直接经内核错误面浮出。

pub use ygg_modules::{
    effects, name_of_path, normalize, path_of, LoadStack, ModuleError, ModuleSet, MAX_CODE_BYTES,
    MAX_DEPTH, MAX_MODULES, MAX_NAME_LEN, MAX_SEGMENT_LEN, MODULE_EXTENSION,
};

// ───────────────────────── ② Lua 注入口（与 ① 以模块边界隔开） ─────────────────────────

mod binding {
    use std::cell::RefCell;
    use std::rc::Rc;

    use mlua::{Error, Function, Lua, MultiValue, RegistryKey, Table, Value};

    use super::{effects, normalize, path_of, LoadStack, ModuleError, ModuleSet};
    use crate::fuel::LoadBudget;

    /// 装载上下文：`Rc<RefCell>` 单实例，require 闭包与驱动句柄共享。
    ///
    /// `cache` 存 Lua registry 表（key = 规范名，value = 模块返回值；`nil` /
    /// 无返回值统一记 `true`）。用 [`RegistryKey`] 而不是裸 `Table`：key 不持有
    /// Lua 状态引用，其 `Drop` 只把自己还给状态的 unref 列表——上下文与 Lua
    /// 状态的销毁顺序因此无关（不会出现「表句柄比状态活得久」的悬垂面）。
    struct ModuleCtx {
        set: ModuleSet,
        cache: RegistryKey,
        stack: LoadStack,
        budget: LoadBudget,
    }

    /// 缓存命中（`Ok(None)` = 未装载过）。
    fn cached(cache: &Table, name: &str) -> Result<Option<Value>, ModuleError> {
        let value: Value = cache.raw_get(name).map_err(|err| ModuleError::Loader {
            details: err.to_string(),
        })?;
        Ok(if value.is_nil() { None } else { Some(value) })
    }

    /// registry 表句柄（内部面失败 → [`ModuleError::Loader`]）。
    fn cache_table(lua: &Lua, key: &RegistryKey) -> Result<Table, ModuleError> {
        lua.registry_value(key).map_err(|err| ModuleError::Loader {
            details: err.to_string(),
        })
    }

    /// require 的失败面：装载器自判的拒绝 vs 模块 chunk 自身的错误。
    ///
    /// 两者必须分开：拒绝面由装载器格式化（带调用点 + require 链）；chunk 错误
    /// 按 Lua 原生错误链原样浮出（每层已带 `path:line:`），再包一层只会让
    /// 深层依赖的错误文本平方级膨胀。
    enum RequireError {
        /// 装载器自判（名字非法 / 未登记 / 循环 / 超深 / 内部面）。
        Denied(ModuleError),
        /// 模块 chunk 自身执行失败（mlua 错误原样浮出）。
        Chunk(Error),
    }

    impl From<ModuleError> for RequireError {
        fn from(err: ModuleError) -> Self {
            RequireError::Denied(err)
        }
    }

    /// 模块装载面句柄（沙箱持有；驱动经此注入模块表、装载结束收口）。
    #[derive(Clone)]
    pub struct ModuleHandle {
        ctx: Rc<RefCell<ModuleCtx>>,
    }

    impl ModuleHandle {
        /// 注入模块表（覆盖式；驱动在入口 chunk 之前一次注入）。
        pub fn set_modules(&self, set: ModuleSet) {
            self.ctx.borrow_mut().set = set;
        }

        /// 装载期燃料句柄（驱动转交 `fuel::install_with_load_budget`：装载
        /// 消耗独立预算，不挤占单转换预算）。
        pub fn load_budget(&self) -> LoadBudget {
            self.ctx.borrow().budget.clone()
        }

        /// 已登记模块数（诊断面）。
        pub fn module_count(&self) -> usize {
            self.ctx.borrow().set.len()
        }

        /// 是否有模块正在执行（诊断面）。
        pub fn is_loading(&self) -> bool {
            self.ctx.borrow().stack.depth() > 0
        }
    }

    /// 在（已替换的）globals 表上安装受控 `require`。
    ///
    /// 必须先于任何 Mod chunk 执行；模块表由 [`ModuleHandle::set_modules`] 后
    /// 注入——空表的 `require` 对任何名字都给可读拒绝（单文件 Mod 的既有行为：
    /// 白名单面不放行外部模块）。
    pub fn install(lua: &Lua, globals: &Table) -> Result<ModuleHandle, Error> {
        let cache = lua.create_table()?;
        let ctx = Rc::new(RefCell::new(ModuleCtx {
            set: ModuleSet::new(),
            cache: lua.create_registry_value(cache)?,
            stack: LoadStack::new(),
            budget: LoadBudget::new(crate::fuel::FUEL_LOAD_BUDGET),
        }));
        let require = {
            let ctx = Rc::clone(&ctx);
            lua.create_function(move |lua, name: String| -> Result<Value, Error> {
                require_module(lua, &ctx, &name).map_err(|err| match err {
                    // 拒绝面：`<调用点>: <原因>`（与模块 chunk 错误的
                    // `path:line:` 前缀同形，排障口径统一）。
                    RequireError::Denied(denied) => {
                        Error::runtime(format!("{}: {denied}", call_site(lua)))
                    }
                    RequireError::Chunk(err) => err,
                })
            })?
        };
        globals.raw_set("require", require)?;
        Ok(ModuleHandle { ctx })
    }

    /// 装载一个模块（`require` 的本体）。
    ///
    /// **借用纪律**：执行 Lua chunk 期间绝不持有 `ctx` 的 `RefCell` 借用——
    /// 嵌套 require 会对同一上下文再次借用，长借用必然 panic。所有上下文
    /// 读写都收紧在 chunk 执行之前 / 之后的小临界区里。
    fn require_module(
        lua: &Lua,
        ctx: &Rc<RefCell<ModuleCtx>>,
        raw_name: &str,
    ) -> Result<Value, RequireError> {
        let name = normalize(raw_name)?;

        // ① 相态 + 缓存：已装载过即返回同一对象（table 身份一致），不再执行。
        {
            let c = ctx.borrow();
            let cache = cache_table(lua, &c.cache)?;
            if let Some(value) = cached(&cache, &name)? {
                return Ok(value);
            }
        }

        // ② 查表：装载期只查已登记模块（不做路径拼接）。
        let code = {
            let c = ctx.borrow();
            match c.set.get(&name) {
                Some(code) => code.to_vec(),
                None => {
                    let chain = c.stack.chain(&name);
                    return Err(ModuleError::NotFound { name, chain }.into());
                }
            }
        };

        // ③ chunk 名：`@<相对路径>`（traceback 带真实文件与行号）。`path_of`
        //    与 `normalize` 同源，今天不会失败；仍在**判栈之前**算——否则万一
        //    两者口径分叉，失败会留下一个存活帧，让后续的循环/深度诊断报出
        //    一条假链条（r4 报告的健壮性 nit）。
        let chunk_name = format!("@{}", path_of(&name)?);

        // ④ 循环 / 深度判栈（纯 Rust；失败不入栈）。
        ctx.borrow_mut().stack.push(&name)?;

        // ⑤ 执行：影子表覆盖 chunk 的**整个执行期**（含它自己嵌套 require 的
        //    子树——嵌套层再装一层 guard，恢复是严格栈序，外层不受影响）。
        //    guard 在闭包结束（= chunk 返回或抛错）时 Drop，成败都恢复。
        //    装载期预算由 require 边界进出（hook 据此分流，不挤占转换预算）。
        let outcome: Result<MultiValue, Error> = {
            let budget = ctx.borrow().budget.clone();
            budget.begin();
            let outcome = ShadowBanned::install(lua)
                .and_then(|_guard| lua.load(code).set_name(chunk_name).call(()));
            budget.end();
            outcome
        };

        // ⑥ 出栈（成败都出）+ 成功写缓存（`nil` / 无返回值 → `true`）。
        match outcome {
            Ok(values) => {
                let value = match values.into_vec().into_iter().next() {
                    Some(Value::Nil) | None => Value::Boolean(true),
                    Some(value) => value,
                };
                {
                    let mut c = ctx.borrow_mut();
                    c.stack.pop();
                    let cache = cache_table(lua, &c.cache)?;
                    cache.raw_set(name.as_str(), value.clone()).map_err(|err| {
                        ModuleError::Loader {
                            details: err.to_string(),
                        }
                    })?;
                }
                Ok(value)
            }
            Err(err) => {
                ctx.borrow_mut().stack.pop();
                Err(RequireError::Chunk(err))
            }
        }
    }

    /// 副作用影子表：模块 chunk 执行期间把**禁止面**原位换成拒绝桩（清单在
    /// [`effects`]），执行完恢复。
    ///
    /// 为什么需要它：模块顶层一旦能调 `ygg.on`，注册序 = require 序，派发序就
    /// 由「文件怎么拆」决定，重构不再安全。把禁止面拦在运行时，`require` 的
    /// 调用时机才不需要与注册时机绑在一起（懒加载因此成立）。静态检查
    /// （`ygg-lint --pack`）在作者侧提前报同一件事，两者共用同一份清单。
    ///
    /// **异常安全靠 `Drop`**：模块 chunk 抛错时也必须恢复原值，否则拒绝桩会
    /// 残留到后续所有模块与 handler，污染整个会话。所以不写成
    /// `install(); ...; restore();`——中间任何一条 `?` 都会跳过恢复。
    ///
    /// 入口 chunk **不**受它约束：入口就是装配清单，要注册、要初始化。
    struct ShadowBanned {
        /// 被替换的现场（持有者表, 成员名, 原值），按安装序记录；`Drop` 时
        /// 逆序放回。
        saved: Vec<(Table, &'static str, Value)>,
    }

    impl ShadowBanned {
        /// 装上影子表（任一成员取不到即报错——此时已装的成员由局部 guard 的
        /// `Drop` 回滚，不会留半套）。
        fn install(lua: &Lua) -> Result<Self, Error> {
            let mut guard = Self { saved: Vec::new() };
            let ygg: Table = lua.globals().raw_get("ygg")?;
            for &name in effects::BANNED_CALLS {
                guard.ban(lua, &ygg, &[name])?;
            }
            for &(table, member) in effects::BANNED_TABLE_MEMBERS {
                guard.ban(lua, &ygg, &[table, member])?;
            }
            Ok(guard)
        }

        /// 把一个访问路径换成拒绝桩。
        ///
        /// `path` 形如 `["on"]` / `["kv", "set"]`。**取不到就跳过**，不为它
        /// 凭空造一个成员：单文件 Mod 没有 `fsm` / `narrative`（stdlib 未装），
        /// 此时 `ygg.fsm.define(..)` 本来就由 Lua 自己报 nil-index；替它造一个
        /// 桩反而会多出一条无中生有的成员。
        fn ban(&mut self, lua: &Lua, ygg: &Table, path: &[&'static str]) -> Result<(), Error> {
            let (owner, member) = match path {
                [name] => (ygg.clone(), *name),
                [table, member] => match ygg.raw_get::<Value>(*table)? {
                    Value::Table(sub) => (sub, *member),
                    _ => return Ok(()),
                },
                _ => return Ok(()),
            };
            let original: Value = owner.raw_get(member)?;
            if original.is_nil() {
                return Ok(());
            }
            owner.raw_set(member, banned_stub(lua, path)?)?;
            self.saved.push((owner, member, original));
            Ok(())
        }
    }

    impl Drop for ShadowBanned {
        fn drop(&mut self) {
            // 逆序放回（每个成员在一个 guard 里只被替换一次，顺序其实无关；
            // 逆序只是为了与安装对称）。Drop 不能传播错误：`raw_set` 只可能
            // 在 Lua 状态已关闭时失败——那时已没有任何「后续执行」会被残留
            // 的桩污染。
            while let Some((owner, member, original)) = self.saved.pop() {
                let _ = owner.raw_set(member, original);
            }
        }
    }

    /// 禁止面的拒绝桩（任意实参都接：`ygg.kv.set(k, v)` 与 `ygg.on(e, f)`
    /// 的 arity 不同，不为参数形状再分一套）。
    fn banned_stub(lua: &Lua, path: &[&'static str]) -> Result<Function, Error> {
        let name = path.join(".");
        lua.create_function(move |lua, _: MultiValue| -> Result<(), Error> {
            Err(Error::runtime(banned_message(lua, &name)))
        })
    }

    /// 模块顶层碰禁止面的报错：一次说清三件事——**做了什么**（哪一行、调了
    /// 什么）、**该怎么做**（模块纯导出 + 入口显式装配）、**为什么**（注册序
    /// 就是派发序，否则拆文件就在改行为）。
    fn banned_message(lua: &Lua, name: &str) -> String {
        format!(
            "{site}: 模块顶层不能调用 `ygg.{name}`\n  \
             模块顶层只做纯导出（定义函数与常量），不注册、不写状态、不产生输出；\n  \
             装配集中在入口 {entry}：\n    \
             1) 模块里写 function M.install(deps) ... ygg.{name}(...) ... end（在函数体内调用是允许的）\n    \
             2) 入口 require 该模块并调用 M.install(deps)\n  \
             原因：handler 的注册序就是派发序。让模块自己注册，等于让「文件怎么拆」决定「运行时行为」，\n  \
             重构就不再安全。模块纯了以后，require 才能在 handler 里按需调用。\n  \
             见 docs/mod-authoring.md §5 多文件 Mod",
            site = call_site(lua),
            entry = crate::modload::MOD_SOURCE_FILE,
        )
    }

    /// require / 影子表桩的调用点文本（`src/a.lua:3` / `mod.lua:7`）：装载器
    /// 自判的拒绝与运行时拒绝桩因此与模块 chunk 错误同形——都带「路径 + 行号」。
    /// 取不到就退回 `?`（调用栈异常时才发生，不影响拒绝语义）。
    fn call_site(lua: &Lua) -> String {
        let site = lua.inspect_stack(1, |debug| {
            let source = debug.source();
            let short = source
                .short_src
                .as_deref()
                .or(source.source.as_deref())
                .unwrap_or("?");
            match debug.current_line() {
                Some(line) => format!("{short}:{line}"),
                None => short.to_owned(),
            }
        });
        site.unwrap_or_else(|| "?".to_owned())
    }
}

pub use binding::{install, ModuleHandle};
