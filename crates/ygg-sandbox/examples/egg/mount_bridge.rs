use super::*;

/// 静态配置的包内路径（pack.json 条目 `ygg:agent/config`；缺声明时按约定回落）。
pub(crate) fn agent_config_path(dir: &Path) -> String {
    let fallback = "data/config.json".to_owned();
    let Some(pack) = fs::read_to_string(dir.join(PACK_FILE))
        .ok()
        .and_then(|text| serde_json::from_str::<Json>(&text).ok())
    else {
        return fallback;
    };
    pack["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| entry["id"].as_str() == Some("ygg:agent/config"))
        .and_then(|entry| entry["path"].as_str())
        .map_or(fallback, str::to_owned)
}

// ============================================================================
// §Mod 目录装载（运行期读盘——agent 必须支持任意 mod 目录，不能像
//   adventure.rs 那样 include_str! 编译期固化）
//
//   读盘/声明对账只走 `ygg_sandbox::modload`（与走查 playtest 同一份实现）：
//   本文件多出来的诉求只有两条——① 热载要**反复**从同一份参数重建驱动
//   （签名变了才重读），所以持有 `ModBundle` 而非一次性内核；② 错误文案要带
//   宿主口径的前缀。所以下面只有两层薄包装，不复制任何读盘/对账逻辑。
// ============================================================================

/// [`ModBundle::kernel`] 的宿主错误文案（装载失败 = 启动失败）。
pub(crate) fn kernel_from(bundle: &ModBundle) -> Result<KernelFacade, String> {
    bundle
        .kernel()
        .map_err(|err| format!("内核装载失败：{err}"))
}

/// [`ModBundle::restore`] 的宿主错误文案（热载/续局重建失败 = 回滚到旧驱动）。
pub(crate) fn restore_from(bundle: &ModBundle, save_bytes: &[u8]) -> Result<KernelFacade, String> {
    bundle
        .restore(save_bytes)
        .map_err(|err| format!("内核重装载失败：{err}"))
}

/// CLI 固定的 sim 种子（激活触点 S-1）。
///
/// 固定常量 = 确定性可复现；per-run seed 属后续需求，不在此处发明。
pub(crate) const SIM_CLI_SEED: u64 = 20261121;

/// 把 `ygg_store::MountStore` 接成沙箱挂载桥（宿主侧适配器；形状与
/// `tests/mounts.rs` 的 `StoreBridge` 同型——examples 不能 use tests，就地重写）。
pub(crate) struct CliMountBridge {
    pub(crate) store: MountStore,
}

pub(crate) fn bridge_kind(kind: ygg_store::MountKind) -> MountKind {
    match kind {
        ygg_store::MountKind::File => MountKind::File,
        ygg_store::MountKind::Dir => MountKind::Dir,
        ygg_store::MountKind::Missing => MountKind::Missing,
    }
}

impl MountFs for CliMountBridge {
    fn read(&self, path: &str) -> Result<Option<Vec<u8>>, String> {
        self.store.read(path).map_err(|err| err.to_string())
    }

    fn write(&mut self, path: &str, content: &[u8]) -> Result<MountWritten, String> {
        let written = self
            .store
            .write(path, content)
            .map_err(|err| err.to_string())?;
        Ok(MountWritten {
            path: written.path,
            bytes: written.bytes,
            sha256: written.sha256,
        })
    }

    fn stat(&self, path: &str) -> Result<MountStat, String> {
        let stat = self.store.stat(path).map_err(|err| err.to_string())?;
        Ok(MountStat {
            exists: stat.exists,
            bytes: stat.bytes,
            sha256: stat.sha256,
            kind: bridge_kind(stat.kind),
        })
    }

    fn list(&self, path: &str) -> Result<Vec<MountEntry>, String> {
        let rows = self.store.list(path).map_err(|err| err.to_string())?;
        Ok(rows
            .into_iter()
            .map(|row| MountEntry {
                name: row.name,
                path: row.path,
                bytes: row.bytes,
                kind: bridge_kind(row.kind),
            })
            .collect())
    }
}

/// 激活触点（S-1，用户授权账：charter 2026-11-21「允许」）：把既有库面装配进
/// 内核——fs 桥（`pack.json` 的 `mounts` 段 → `MountStore::attach` →
/// `set_mount_fs`）与 sim（固定常量种子 → `set_sim`）。
///
/// 语义零新增：不声明 mounts 的 mod，attach 幂等通过；sim 表常注册，未注入
/// 才会 `YGG-SIM-UNMOUNTED`。调用方定失败语义：
/// - 开局：`?` 传播 = 装载失败同语义（fail-closed 退出）；
/// - 热载：失败沿用旧内核（与契约现读同待遇）。
pub(crate) fn activate_host_faces(
    kernel: &mut KernelFacade,
    run_dir: &Path,
    mod_dir: &Path,
) -> Result<(), String> {
    let pack_text = fs::read_to_string(mod_dir.join(PACK_FILE))
        .map_err(|err| format!("读取 {}/{PACK_FILE} 失败：{err}", mod_dir.display()))?;
    let pack: Json = serde_json::from_str(&pack_text)
        .map_err(|err| format!("{}/{PACK_FILE} 不是合法 JSON：{err}", mod_dir.display()))?;
    let store = MountStore::attach(run_dir, &pack, mod_dir)
        .map_err(|err| format!("挂载桥装配失败：{err}"))?;
    kernel.set_mount_fs(Some(Box::new(CliMountBridge { store })));
    kernel.set_sim(Some(Box::new(ygg_sim::Sim::new(SIM_CLI_SEED))));
    Ok(())
}
