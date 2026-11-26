use std::cell::RefCell;
use std::rc::Rc;

use ygg_core::bus::HandlerRegistry;
use ygg_core::entity::EntityStore;
use ygg_core::kv::KvStore;
use ygg_core::save::SaveFile;

use super::{resource_error_to_kernel, KernelError, KernelFacade};
use crate::host_api::{self, HostApi, HostHandles};
use crate::inject::{self, DeterminismHandle};
use crate::modules::{ModuleSet, ModuleTable};
use crate::resources::ResourceTable;
use crate::sandbox::Sandbox;
use crate::stdlib;

/// S10/S11 装载管线的构件包（[`build_kernel`] 产出、
/// [`KernelFacade::from_parts`] 消费；字段顺序即生命周期挂靠面）。
struct KernelParts {
    sandbox: Sandbox,
    determinism: DeterminismHandle,
    host: HostApi,
    registry: HandlerRegistry,
    entities: Rc<RefCell<EntityStore>>,
    kv: Rc<RefCell<KvStore>>,
}

/// 装载序单一管线（S10 锁定序 + S11 资源步 + r1 模块表 + r2 chunk 覆盖表；`new` /
/// `with_resources` / `with_modules` / `with_resources_and_modules` /
/// `with_resources_and_module_table` 共享）。
///
/// `resources = Some(table)` 时在 host API 之后、stdlib 之前安装 `ygg.data`；
/// `None` 时跳过该步——`KernelFacade::new` 全序行为与 S10 逐字节一致（S10
/// 测试不加改动保持全绿即本函数的回归证据）。`modules = Some(table)` 时在入口
/// chunk 之前注入模块表与 chunk 名覆盖表；`None` 时 require 保持空表（单文件
/// Mod 的既有行为）。
fn build_kernel(
    mod_source: &str,
    resources: Option<&ResourceTable>,
    modules: Option<ModuleTable>,
) -> Result<KernelParts, KernelError> {
    // ① 沙箱构造：stdlib 裁剪 + 内存上限 + 白名单环境（ygg 占位表 + require 面就位）。
    let sandbox = Sandbox::new().map_err(KernelError::Sandbox)?;
    let lua = sandbox.lua().clone();
    // ② 确定性注入：math 精确集 + 内核种子 random + ygg.ticks。
    let determinism = inject::install(&lua).map_err(KernelError::Sandbox)?;
    // ③ canonical store 先建：host 桥句柄与驱动共享同一 Rc（统一 store）。
    let entities = Rc::new(RefCell::new(EntityStore::new()));
    let kv = Rc::new(RefCell::new(KvStore::new()));
    // ④ host API：装载期 ygg 完整面（on/emit/narrate/choice/kv/entity/sim）。
    let host = host_api::install(
        &lua,
        HostHandles {
            kv: Rc::clone(&kv),
            entities: Rc::clone(&entities),
        },
    )
    .map_err(KernelError::Sandbox)?;
    // ④' S11 资源步：ygg.data 只读数据面（host 之后、stdlib 之前、
    // Mod chunk 之前——Mod 顶层即可读资源）。
    if let Some(table) = resources {
        crate::resources::install(&lua, table).map_err(KernelError::Sandbox)?;
    }
    // ⑤ stdlib：命名空间 + fsm + narrative（先于 Mod chunk，注册序即派发序）。
    stdlib::install(&lua).map_err(KernelError::Sandbox)?;
    // ⑤' r1 模块表注入：入口 chunk 之前（入口顶层即可 require）；空模块表时
    //     跳过——require 面仍在，只是对任何名字可读拒绝。r2 的 chunk 名覆盖表
    //     必须与模块表同一时机注入：入口顶层就 `require("pkg.*")` 时，第一次
    //     装载就要拿到真实路径，否则首次 traceback 就是假路径。
    if let Some(table) = modules {
        let (set, chunk_paths) = table.into_parts();
        sandbox.modules().set_modules(set);
        sandbox.modules().set_chunk_paths(chunk_paths);
    }
    // ⑥ Mod chunk：装载期注册面收集 Mod handler；失败 = ModLoad。chunk
    //    命名 `@mod.lua`（与 stdlib `@stdlib/<name>` 同法，迭代 C3）：
    //    handler 报错 traceback 携带 mod 侧位置，排障不再只见 Lua 桥行号
    //    （迭代 D2 实证收口：mlua `CallbackError.traceback` 在 Rust 回调
    //    返回错误时捕获调用点 Lua 栈，错误文本已含 `mod.lua:<line>` 帧——
    //    host 回调拒绝同样可定位，无穿透修复必要）。
    if let Err(err) = lua.load(mod_source.to_owned()).set_name("@mod.lua").exec() {
        return Err(KernelError::ModLoad(err.to_string()));
    }
    // ⑦ 分相收口：seal（运行期注册禁令）→ 注册转出进注册表。
    //    只封 `ygg.on`——`require` 不在装载期边界上（模块顶层是纯导出面，
    //    执行时机不影响派发序，见 `crate::modules::ShadowBanned`）。
    host.seal(&lua).map_err(KernelError::Sandbox)?;
    let mut registry = HandlerRegistry::new();
    host.register_into(&mut registry, "mod");
    Ok(KernelParts {
        sandbox,
        determinism,
        host,
        registry,
        entities,
        kv,
    })
}

impl KernelFacade {
    /// 按锁定装载序构造驱动（见模块文档）；Mod chunk 失败 =
    /// [`KernelError::ModLoad`]（含 mlua 可读错误文本）。无资源路径：资源表
    /// 为空基线（[`ResourceTable::empty`]），其余序与 `with_resources` 全同。
    pub fn new(mod_source: &str) -> Result<Self, KernelError> {
        let parts = build_kernel(mod_source, None, None)?;
        Ok(Self::from_parts(
            parts,
            Rc::new(ResourceTable::empty()),
            mod_source,
        ))
    }

    /// r1：多文件 Mod 构造——除入口 source 外另给已登记的模块表（`src/*.lua`
    /// 拆出的子模块）。模块表在入口 chunk 之前注入，所以入口顶层就能
    /// `require("src.util")`；入口之后的 handler 里同样可以 `require`（懒加
    /// 载），模块 chunk 受影子表约束（只能纯导出）。
    ///
    /// 无资源路径（资源表为空基线）；带资源的多文件 Mod 走
    /// [`KernelFacade::with_resources_and_modules`]。
    pub fn with_modules(mod_source: &str, modules: ModuleSet) -> Result<Self, KernelError> {
        let parts = build_kernel(mod_source, None, Some(ModuleTable::new(modules)))?;
        Ok(Self::from_parts(
            parts,
            Rc::new(ResourceTable::empty()),
            mod_source,
        ))
    }

    /// S11：带资源包构造驱动。资源表构建**先于任何 Lua 状态**（fail-fast：
    /// 预算/寻址/校验失败零 Lua 侧副作用）；通过后走与 [`KernelFacade::new`]
    /// 全同的装载管线（`resources::install` 在 host 之后、stdlib 之前）。
    pub fn with_resources(
        mod_source: &str,
        manifest_bytes: &[u8],
        schemas: &[(String, String)],
        resources: &[(String, Vec<u8>)],
    ) -> Result<Self, KernelError> {
        let table = ResourceTable::build(manifest_bytes, schemas, resources)
            .map_err(resource_error_to_kernel)?;
        let parts = build_kernel(mod_source, Some(&table), None)?;
        Ok(Self::from_parts(parts, Rc::new(table), mod_source))
    }

    /// S11 + r1：带资源包的多文件 Mod——资源表 fail-fast 与模块表注入的
    /// 唯一组合面（装载序：资源 → stdlib → 模块表 → 入口 chunk → seal）。
    pub fn with_resources_and_modules(
        mod_source: &str,
        modules: ModuleSet,
        manifest_bytes: &[u8],
        schemas: &[(String, String)],
        resources: &[(String, Vec<u8>)],
    ) -> Result<Self, KernelError> {
        Self::with_resources_and_module_table(
            mod_source,
            ModuleTable::new(modules),
            manifest_bytes,
            schemas,
            resources,
        )
    }

    /// S11 + r2：带资源包的多文件 Mod，且带 chunk 名覆盖表（包模块面）。
    ///
    /// 与 [`KernelFacade::with_resources_and_modules`] 同一装载序，差别只在
    /// 覆盖表：包模块的名字不属于 [`crate::modules::path_of`] 的双射
    /// （`pkg.fixture` 的真实文件是 `packages/libs/fixture/init.lua`），所以
    /// traceback 要指到真实文件就必须显式告知。
    pub fn with_resources_and_module_table(
        mod_source: &str,
        modules: ModuleTable,
        manifest_bytes: &[u8],
        schemas: &[(String, String)],
        resources: &[(String, Vec<u8>)],
    ) -> Result<Self, KernelError> {
        let table = ResourceTable::build(manifest_bytes, schemas, resources)
            .map_err(resource_error_to_kernel)?;
        let parts = build_kernel(mod_source, Some(&table), Some(modules))?;
        Ok(Self::from_parts(parts, Rc::new(table), mod_source))
    }

    /// S11：带资源包的读档重建——`from_bytes` 先判存档形态，再走
    /// [`KernelFacade::with_resources`] 全序（资源表 fail-fast → 装载管线），
    /// 最后 [`KernelFacade::apply_save`] 覆写状态面（资源不在存档内、不受
    /// 读档影响——资源是数据不是状态）。
    pub fn restore_with_resources(
        save_bytes: &[u8],
        mod_source: &str,
        manifest_bytes: &[u8],
        schemas: &[(String, String)],
        resources: &[(String, Vec<u8>)],
    ) -> Result<Self, KernelError> {
        let save = SaveFile::from_bytes(save_bytes)?;
        let mut kernel = Self::with_resources(mod_source, manifest_bytes, schemas, resources)?;
        kernel.apply_save(&save)?;
        Ok(kernel)
    }

    /// S11 + r2：带资源包的多文件 Mod 读档重建——[`KernelFacade::restore_with_resources`]
    /// 的模块表变体。热载路径必须走这条：模块文件变了而入口没变时，宿主签名
    /// 会变并触发重装载，若重装载丢掉模块表（只重供资源），跑起来的仍是旧代码
    /// 或入口 require 直接失败——两种都不是「热载成功」。
    pub fn restore_with_resources_and_modules(
        save_bytes: &[u8],
        mod_source: &str,
        modules: ModuleSet,
        manifest_bytes: &[u8],
        schemas: &[(String, String)],
        resources: &[(String, Vec<u8>)],
    ) -> Result<Self, KernelError> {
        Self::restore_with_resources_and_module_table(
            save_bytes,
            mod_source,
            ModuleTable::new(modules),
            manifest_bytes,
            schemas,
            resources,
        )
    }

    /// S11 + r2：带 chunk 名覆盖表的读档重建（热载路径的完整形态）。
    ///
    /// 与 [`KernelFacade::restore_with_resources_and_modules`] 同一分派：模块
    /// 文件变了而入口没变时宿主签名会变并走到这里，丢覆盖表只会让 traceback
    /// 退回假路径——所以两者同进同出。
    pub fn restore_with_resources_and_module_table(
        save_bytes: &[u8],
        mod_source: &str,
        modules: ModuleTable,
        manifest_bytes: &[u8],
        schemas: &[(String, String)],
        resources: &[(String, Vec<u8>)],
    ) -> Result<Self, KernelError> {
        let save = SaveFile::from_bytes(save_bytes)?;
        let mut kernel = Self::with_resources_and_module_table(
            mod_source,
            modules,
            manifest_bytes,
            schemas,
            resources,
        )?;
        kernel.apply_save(&save)?;
        Ok(kernel)
    }

    /// 构件包 + 资源表 + Mod 源 → 驱动实例（字段装配单点；三个构造入口共用）。
    fn from_parts(parts: KernelParts, resources: Rc<ResourceTable>, mod_source: &str) -> Self {
        Self {
            _sandbox: parts.sandbox,
            _determinism: parts.determinism,
            host: parts.host,
            registry: parts.registry,
            entities: parts.entities,
            kv: parts.kv,
            resources,
            mod_source: mod_source.to_owned(),
            pending_choice: None,
            external_log: Vec::new(),
            dispatching: false,
        }
    }

    /// 从存档 bytes 重建驱动：`new(mod_source)` 全序重建（handler 为代码
    /// 不入档、重装载即恢复）后 [`KernelFacade::apply_save`] 覆写状态面。
    pub fn restore(save_bytes: &[u8], mod_source: &str) -> Result<Self, KernelError> {
        let save = SaveFile::from_bytes(save_bytes)?;
        let mut kernel = Self::new(mod_source)?;
        kernel.apply_save(&save)?;
        Ok(kernel)
    }
}
