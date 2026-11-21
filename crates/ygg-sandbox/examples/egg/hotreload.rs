use super::*;

/// 单文件监视快照（热载变更判据：mtime + 字节数）。
///
/// mtime 是**宿主侧**运行时信息，不涉内核确定性面（内核只消费事件与状态）；
/// 时钟 / 文件系统本就是宿主管辖，与「内核零 IO」纪律不冲突。
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct FileStamp {
    pub(crate) path: String,
    pub(crate) len: u64,
    pub(crate) modified: Option<SystemTime>,
}

/// 监视文件清单：五件套 + pack.json 声明的每个条目 / schema 文件 / 代码模块
/// （pack.json 本身损坏时退化为固定三件，仍能检测修复动作）。
///
/// **`code` 段必须在这里纳管**（r2）：只监视 `mod.lua` 时，改 `src/*.lua` 而入口
/// 一字未动 → 签名不变 → 热载不触发、续局静默跑旧代码，无任何报错。签名本身
/// 只回答「变没变」（路径 + 字节数 + mtime），内容精度由 manifest 的 sha256
/// 在装载时承担——两步分工是既定口径，不为 resume 重算全量哈希。
pub(crate) fn watched_files(dir: &Path) -> Vec<String> {
    let mut files = vec![
        MOD_SOURCE_FILE.to_owned(),
        MANIFEST_FILE.to_owned(),
        PACK_FILE.to_owned(),
    ];
    // 配置的实际解析结果必须纳管：pack 未声明 `ygg:agent/config` 时
    // [`agent_config_path`] 回落 `data/config.json`，而下面只收**声明过的**
    // 路径——只改回落配置（如 `drive.maxSteps`）会让签名不变，热载直接 return，
    // 新阈值与结算口径永不生效且无告警。收「解析结果」而不是「声明」才能对齐
    // 监视面与实际使用面。
    files.push(agent_config_path(dir));
    if let Ok(text) = fs::read_to_string(dir.join(PACK_FILE)) {
        if let Ok(pack) = serde_json::from_str::<Json>(&text) {
            for entry in pack["entries"].as_array().into_iter().flatten() {
                if let Some(path) = entry["path"].as_str() {
                    files.push(path.to_owned());
                }
            }
            for file in pack["schemaFiles"].as_array().into_iter().flatten() {
                if let Some(path) = file.as_str() {
                    files.push(path.to_owned());
                }
            }
            // r2：代码模块（含入口——声明了 `code.entry` 时入口名不一定叫 mod.lua）。
            if let Some(entry) = pack["code"]["entry"].as_str() {
                files.push(entry.to_owned());
            }
            for module in pack["code"]["modules"].as_array().into_iter().flatten() {
                if let Some(path) = module.as_str() {
                    files.push(path.to_owned());
                }
            }
        }
    }
    files.sort();
    files.dedup();
    files
}

/// mod 目录现状快照（与 [`Session::signature`] 逐字段比对判定「变没变」）。
pub(crate) fn mod_signature(dir: &Path) -> Vec<FileStamp> {
    watched_files(dir)
        .into_iter()
        .map(|rel| {
            let meta = fs::metadata(dir.join(&rel)).ok();
            FileStamp {
                path: rel,
                len: meta.as_ref().map_or(0, fs::Metadata::len),
                modified: meta.and_then(|meta| meta.modified().ok()),
            }
        })
        .collect()
}

impl Session {
    /// 热载检查：mod 目录文件 mtime + 字节数变化 → 状态无损换新 mod。
    ///
    /// 序：现态存档（`save` 信封即基线）→ 按新包 `restore_with_resources`
    /// （重装载 + 回灌单点原子完成）→ 成功即换驱动；失败保留旧驱动继续。
    /// 签名在**尝试后**前移：同一份破损 mod 只告警一次，修好文件 mtime 再变
    /// 即触发下一次尝试。
    pub(crate) fn maybe_reload(&mut self) {
        let signature = mod_signature(&self.mod_dir);
        if signature == self.signature {
            return;
        }
        self.signature = signature;

        // ① 现态存档（热载基线；状态不变，故不写 autosave 文件）。
        let save_envelope = parse_envelope(&self.kernel.dispatch("save", ""));
        let save = &save_envelope["save"];
        if save.is_null() {
            let message = format!("热载中止：现态存档不可用（{}）", error_text(&save_envelope));
            self.notices.push(message.clone());
            self.reload = Some(Reload {
                reloaded: false,
                source: "watch",
                message: Some(message),
            });
            return;
        }
        let save_text = save.to_string();

        // ② 新包重装载 + 回灌（一次原子构造；失败即回滚到旧驱动）。
        //    契约（agent / inboxRoutes / drive）也在这一步现读：任一项不合法即整体
        //    判定热载失败——**不**存在「内核换了、契约沿用旧的」半更新态。
        let outcome =
            ModBundle::load_with_packages_root(&self.mod_dir, self.packages_root.as_deref())
                .and_then(|bundle| {
                    let contract = AgentContract::load(&self.mod_dir)?;
                    let inbox_routes = InboxRoutes::load(&self.mod_dir)?;
                    let limits = DriveLimits::load(&self.mod_dir.join(&self.config_rel))?;
                    let settlement = SettlementPolicy::load(&self.mod_dir.join(&self.config_rel))?;
                    // 工具白名单也是契约：热载同一把尺（半声明 / 未放行 = 整体热载失败，
                    // 不存在「内核换了、工具声明沿用旧的」半更新态）。
                    let tools = ToolRegistry::load(&self.mod_dir)?;
                    let tool_access = ToolAccess::load(&self.mod_dir.join(&self.config_rel))?;
                    require_tool_allowlist(
                        &tools,
                        &tool_access,
                        &self.mod_dir.join(&self.config_rel),
                    )?;
                    restore_from(&bundle, save_text.as_bytes()).map(|kernel| {
                        (
                            kernel,
                            contract,
                            inbox_routes,
                            limits,
                            settlement,
                            tools,
                            tool_access,
                        )
                    })
                });
        match outcome {
            Ok((mut kernel, contract, inbox_routes, limits, settlement, tools, tool_access)) => {
                // 激活触点（S-1 热载）：新 pack 可能变 mounts 声明 → 重 attach
                //（三态幂等，零新建状态）；失败 = 整体热载失败，沿用旧内核
                //（与契约现读同待遇，不分裂「内核换了、桥没换」的半更新态）。
                if let Err(message) = activate_host_faces(&mut kernel, &self.run_dir, &self.mod_dir)
                {
                    self.notices
                        .push(format!("热载失败，沿用旧 mod 继续：{message}"));
                    self.reload = Some(Reload {
                        reloaded: false,
                        source: "watch",
                        message: Some(message),
                    });
                    return;
                }
                self.kernel = kernel;
                // 换了 Mod 就换了契约（事件名 / inbox 路由 / 摘要约定 / 推进阈值 / 工具声明）。
                self.contract = contract;
                self.inbox_routes = inbox_routes;
                self.limits = limits;
                self.settlement = settlement;
                self.tools = tools;
                self.tool_access = tool_access;
                self.mount_tool_runner();
                self.summary_cache = None;
                self.reload = Some(Reload {
                    reloaded: true,
                    source: "watch",
                    message: None,
                });
                // 签名前移后即时落盘：下一进程按新包比对，不重复报「变过」。
                self.write_meta();
            }
            Err(message) => {
                // 旧 mod 继续跑：热载是增强面，不能把在局中的会话打崩。
                self.notices
                    .push(format!("热载失败，沿用旧 mod 继续：{message}"));
                self.reload = Some(Reload {
                    reloaded: false,
                    source: "watch",
                    message: Some(message),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests;
