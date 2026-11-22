//! `ygg.rt.*` 实时宿主面（S31 R1：帧循环 + 输入面；契约
//! `docs/design/host/rt-face.md`）。
//!
//! - **帧循环是宿主的，不是 Mod 的**：常驻宿主（`examples/rt/`）每帧派发一次
//!   `rt.tick` 事件（载荷 = 全整数规范串），Mod 只在 handler 里读
//!   [`RtFrame`] 的投影（`ygg.rt.loop.info` / `ygg.rt.input.*`）。
//!   本文件只持有**每帧被宿主闩锁的快照**：调度（hz / catch-up）、录制与
//!   回放都在 [`crate::rt`]（引擎面），此处零调度逻辑。
//! - **输入是轮询式帧快照，不是回调**（D2）：每帧一条规范记录——帧内多键沿
//!   折叠（净变化），`pressed` / `released` 按键码升序（规范序，回放等价的
//!   前提）。快照由宿主写入（`KernelFacade::rt_handle` 的共享 `Rc`），
//!   **没有 Lua 侧写入口**——Mod 伪造输入 = 破坏回放证据链，结构性缺席。
//! - **规范串**（[`RtInput::canonical`]）：`k=<位串|->;p=<csv|->;r=<csv|->;m=<dx>,<dy>,<b>`
//!   全整数；`rt.tick` 载荷 = `n=<n>;dt=<dt>;<规范串>`（单字符串过驱动标量门）。
//!   录制存规范串，回放按串重建快照重派——篡改任一 dt 或键序都会改变
//!   载荷字节，被逐帧哈希链抓出（first_mismatch）。
//! - **`ygg.rt.config` 装载期限定**：与 `ygg.on` 同一条运行期禁令——seal 时
//!   原位替换为可读报错桩（[`super::HostApi::seal`]）。hz / catch-up 不影响
//!   确定性（dt 与输入都逐帧照录），但装载期声明让「一局一套调度参数」
//!   成为契约。

use super::HostState;
use mlua::{Error, Lua, Table, Value as LuaValue};
use std::cell::RefCell;
use std::rc::Rc;

/// 键码上界（位串宽度有界：最长 64 字符，规范串长度可预算）。
pub const RT_KEY_MAX: i64 = 64;
/// 默认帧率（计划 D1：60Hz）。
pub const RT_DEFAULT_HZ: u32 = 60;
/// hz 合法区间（1..=1000：1 覆盖回合制退化，1000 封顶防极端配置）。
pub const RT_HZ_MAX: u32 = 1000;
/// 默认落后补帧上限（计划 D1：max_catchup=4，螺旋死亡防护）。
pub const RT_DEFAULT_MAX_CATCHUP: u32 = 4;
/// max_catchup 合法区间。
pub const RT_MAX_CATCHUP_CAP: u32 = 64;

/// `ygg.rt.config` 的 seal 后拒绝码（装载期限定）。
pub const RT_SEALED: &str = "YGG-RT-SEALED";
/// `ygg.rt.*` 参数形状拒绝码。
pub const RT_SHAPE: &str = "YGG-RT-SHAPE";

/// 宿主形态（D1）：run 模式（egg 单派发 / `--drive`）与 rt 模式（常驻循环）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RtMode {
    /// run 模式（缺省）：每进程一派发，`game.tick` 是 dt=1 的退化帧。
    Run,
    /// rt 模式：常驻宿主逐帧派发 `rt.tick`。
    Rt,
}

impl RtMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RtMode::Run => "run",
            RtMode::Rt => "rt",
        }
    }
}

/// 一帧的输入快照（轮询面；宿主闩锁，Mod 只读）。
///
/// 三个键序向量都**升序去重**（规范序）：`pressed` = 本帧净按下沿、
/// `released` = 本帧净抬起沿（帧内按下又抬起 = 净零，两侧都不出现——
/// 折叠语义）。held 是本帧帧末按住集。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RtInput {
    pub held: Vec<i64>,
    pub pressed: Vec<i64>,
    pub released: Vec<i64>,
    /// 自上帧以来的量化位移（整数，可负）。
    pub mouse_dx: i64,
    pub mouse_dy: i64,
    /// 按键位掩码（bit0=左键）。
    pub mouse_buttons: i64,
}

impl RtInput {
    /// 中性快照（run 模式 / 无输入帧）：什么都没发生。
    pub fn neutral() -> Self {
        Self {
            held: Vec::new(),
            pressed: Vec::new(),
            released: Vec::new(),
            mouse_dx: 0,
            mouse_dy: 0,
            mouse_buttons: 0,
        }
    }

    /// 规范串（确定性字节，回放等价的前提）：键序升序、字段定序、全整数。
    ///
    /// `k=` 位串：位置 i（1 起）= 键码 i 是否按住，长度 = 本帧出现过的最大
    /// 键码；三向量全空时整段记 `-`（避免零长歧义）。
    pub fn canonical(&self) -> String {
        let max_code = self
            .held
            .iter()
            .chain(self.pressed.iter())
            .chain(self.released.iter())
            .copied()
            .max()
            .unwrap_or(0);
        let held_part = if max_code == 0 {
            "-".to_owned()
        } else {
            let mut bits = String::with_capacity(max_code as usize);
            for code in 1..=max_code {
                bits.push(if self.held.contains(&code) { '1' } else { '0' });
            }
            bits
        };
        let csv = |codes: &[i64]| -> String {
            if codes.is_empty() {
                "-".to_owned()
            } else {
                codes
                    .iter()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            }
        };
        format!(
            "k={};p={};r={};m={},{},{}",
            held_part,
            csv(&self.pressed),
            csv(&self.released),
            self.mouse_dx,
            self.mouse_dy,
            self.mouse_buttons
        )
    }

    /// [`RtInput::canonical`] 的严格逆（回放侧重建快照）。任何偏离即错误——
    /// 录制文件可能被手改，解析不静默吞。
    pub fn parse(text: &str) -> Result<Self, String> {
        let fields: Vec<&str> = text.split(';').collect();
        if fields.len() != 4 || fields[0].len() < 2 || !fields[0].starts_with("k=") {
            return Err(format!("规范串字段不全（期望 k/p/r/m 四段）：`{text}`"));
        }
        let bad = |what: &str| format!("规范串 `{text}` 的 {what} 段非法");
        let held: Vec<i64> = if fields[0] == "k=-" {
            Vec::new()
        } else {
            let bits = &fields[0][2..];
            if bits.is_empty() || !bits.bytes().all(|b| b == b'0' || b == b'1') {
                return Err(bad("k（位串）"));
            }
            bits.bytes()
                .enumerate()
                .filter(|(_, b)| *b == b'1')
                .map(|(i, _)| (i + 1) as i64)
                .collect()
        };
        let csv_field = |raw: &str, name: &str| -> Result<Vec<i64>, String> {
            if raw == "-" {
                return Ok(Vec::new());
            }
            raw.split(',')
                .map(|part| {
                    part.parse::<i64>()
                        .map_err(|_| bad(&format!("p/r（{name}）")))
                })
                .collect()
        };
        let pressed = csv_field(fields[1].strip_prefix("p=").ok_or_else(|| bad("p"))?, "p")?;
        let released = csv_field(fields[2].strip_prefix("r=").ok_or_else(|| bad("r"))?, "r")?;
        let mouse_raw = fields[3].strip_prefix("m=").ok_or_else(|| bad("m"))?;
        let mouse: Vec<i64> = mouse_raw
            .split(',')
            .map(|part| part.parse::<i64>().map_err(|_| bad("m")))
            .collect::<Result<Vec<_>, _>>()?;
        if mouse.len() != 3 {
            return Err(bad("m（期望 dx,dy,buttons 三整数）"));
        }
        let input = Self {
            held,
            pressed,
            released,
            mouse_dx: mouse[0],
            mouse_dy: mouse[1],
            mouse_buttons: mouse[2],
        };
        // 重建后再规范化的字节必须与输入一致（规范串是逐字节证据，不许
        // 「解析成功但语义漂移」——例如键序未升序）。
        if input.canonical() != text {
            return Err(format!(
                "规范串非规范序：`{text}`（规范化为 `{}`）",
                input.canonical()
            ));
        }
        Ok(input)
    }
}

/// 每帧被宿主闩锁的 rt 状态（`HostState` 持有的共享 `Rc` 背面）。
///
/// 字段都由宿主（[`crate::rt`] 引擎 / `examples/rt/`）写；Mod 经
/// `ygg.rt.loop.info` / `ygg.rt.input.*` 只读投影。
#[derive(Clone, Debug)]
pub struct RtFrame {
    pub mode: RtMode,
    /// 已完成派发的帧号（1 起；0 = 尚无帧）。
    pub n: u64,
    pub hz: u32,
    pub max_catchup: u32,
    /// 逻辑时钟（毫秒）= 逐帧 dt_ms 累加（`ygg.rt.input.time()` 的取值）。
    pub logical_ms: i64,
    pub input: RtInput,
}

impl Default for RtFrame {
    fn default() -> Self {
        Self {
            mode: RtMode::Run,
            n: 0,
            hz: RT_DEFAULT_HZ,
            max_catchup: RT_DEFAULT_MAX_CATCHUP,
            logical_ms: 0,
            input: RtInput::neutral(),
        }
    }
}

/// Lua 整数简写（规范面全整数：帧号 / 键码 / 毫秒都不过浮点）。
fn rt_int(i: i64) -> LuaValue {
    LuaValue::Integer(i)
}

/// 从配置表读一个有界正整数字段（缺省用 default；类型/越界即可读错误）。
fn bounded_field(cfg: &Table, key: &str, default: u32, max: u32) -> Result<u32, Error> {
    match cfg.raw_get(key)? {
        LuaValue::Nil => Ok(default),
        LuaValue::Integer(v) if (1..=max as i64).contains(&v) => Ok(v as u32),
        other => Err(Error::runtime(format!(
            "{RT_SHAPE}: `ygg.rt.config` 的 `{key}` 必须是 1..={max} 的整数，得到 {}",
            other.type_name()
        ))),
    }
}

/// 装 `ygg.rt.*`（config 装载期、loop.info / input.* 运行期只读）。
pub(super) fn install_rt(lua: &Lua, state: &Rc<RefCell<HostState>>) -> Result<(), Error> {
    let ygg: Table = lua.globals().raw_get("ygg")?;
    let rt = lua.create_table()?;

    // ---- ygg.rt.config(cfg) —— 装载期限定 ----
    {
        let st = Rc::clone(state);
        rt.raw_set(
            "config",
            lua.create_function(move |_, cfg: Table| -> Result<(), Error> {
                if st.borrow().sealed {
                    return Err(Error::runtime(format!(
                        "{RT_SEALED}: `ygg.rt.config` 装载期限定（运行期禁改；帧调度参数逐帧照录，回放不受影响）"
                    )));
                }
                // 形状闭集：未知键拒绝（fail-fast，与 pack 声明同纪律）。
                for pair in cfg.pairs::<LuaValue, LuaValue>() {
                    let (key, _) = pair?;
                    let name = match &key {
                        LuaValue::String(s) => s.to_str().ok().map(|n| n.to_string()),
                        _ => None,
                    };
                    if !matches!(name.as_deref(), Some("hz") | Some("maxCatchup")) {
                        return Err(Error::runtime(format!(
                            "{RT_SHAPE}: `ygg.rt.config` 只接受 `hz` / `maxCatchup` 键"
                        )));
                    }
                }
                let hz = bounded_field(&cfg, "hz", RT_DEFAULT_HZ, RT_HZ_MAX)?;
                let max_catchup =
                    bounded_field(&cfg, "maxCatchup", RT_DEFAULT_MAX_CATCHUP, RT_MAX_CATCHUP_CAP)?;
                let host = st.borrow();
                let mut frame = host.rt.borrow_mut();
                frame.hz = hz;
                frame.max_catchup = max_catchup;
                Ok(())
            })?,
        )?;
    }

    // ---- ygg.rt.loop.info() ----
    let loop_table = lua.create_table()?;
    {
        let st = Rc::clone(state);
        loop_table.raw_set(
            "info",
            lua.create_function(move |lua, _: ()| -> Result<Table, Error> {
                let frame = st.borrow().rt.borrow().clone();
                let out = lua.create_table()?;
                out.raw_set("mode", frame.mode.as_str())?;
                out.raw_set("n", rt_int(frame.n as i64))?;
                out.raw_set("hz", rt_int(i64::from(frame.hz)))?;
                out.raw_set("maxCatchup", rt_int(i64::from(frame.max_catchup)))?;
                out.raw_set("logicalMs", rt_int(frame.logical_ms))?;
                Ok(out)
            })?,
        )?;
    }
    rt.raw_set("loop", loop_table)?;

    // ---- ygg.rt.input.{keys,mouse,time} ----
    let input_table = lua.create_table()?;
    {
        let st = Rc::clone(state);
        input_table.raw_set(
            "keys",
            lua.create_function(move |lua, _: ()| -> Result<Table, Error> {
                let input = st.borrow().rt.borrow().input.clone();
                let out = lua.create_table()?;
                // held 位串：规范串的 k= 段同源（同一份字节，两处投影不漂移）。
                let max_code = input
                    .held
                    .iter()
                    .chain(input.pressed.iter())
                    .chain(input.released.iter())
                    .copied()
                    .max()
                    .unwrap_or(0);
                let held = if max_code == 0 {
                    String::new()
                } else {
                    (1..=max_code)
                        .map(|code| if input.held.contains(&code) { '1' } else { '0' })
                        .collect()
                };
                out.raw_set("held", held)?;
                let pressed = lua.create_table()?;
                for (i, code) in input.pressed.iter().enumerate() {
                    pressed.raw_set(i + 1, rt_int(*code))?;
                }
                out.raw_set("pressed", pressed)?;
                let released = lua.create_table()?;
                for (i, code) in input.released.iter().enumerate() {
                    released.raw_set(i + 1, rt_int(*code))?;
                }
                out.raw_set("released", released)?;
                Ok(out)
            })?,
        )?;
    }
    {
        let st = Rc::clone(state);
        input_table.raw_set(
            "mouse",
            lua.create_function(move |lua, _: ()| -> Result<Table, Error> {
                let input = st.borrow().rt.borrow().input.clone();
                let out = lua.create_table()?;
                out.raw_set("dx", rt_int(input.mouse_dx))?;
                out.raw_set("dy", rt_int(input.mouse_dy))?;
                out.raw_set("buttons", rt_int(input.mouse_buttons))?;
                Ok(out)
            })?,
        )?;
    }
    {
        let st = Rc::clone(state);
        input_table.raw_set(
            "time",
            lua.create_function(move |_, _: ()| -> Result<i64, Error> {
                Ok(st.borrow().rt.borrow().logical_ms)
            })?,
        )?;
    }
    rt.raw_set("input", input_table)?;

    ygg.raw_set("rt", rt)?;
    Ok(())
}

/// seal 时把 `ygg.rt.config` 原位替换为可读报错桩（装载期限定的运行期执行面）。
pub(super) fn seal_rt(lua: &Lua) -> Result<(), Error> {
    let ygg: Table = lua.globals().raw_get("ygg")?;
    let rt: Table = ygg.raw_get("rt")?;
    let stub = lua.create_function(|_, _: Table| -> Result<(), Error> {
        Err(Error::runtime(format!(
            "{RT_SEALED}: `ygg.rt.config` 装载期限定（运行期禁改）"
        )))
    })?;
    rt.raw_set("config", stub)?;
    Ok(())
}
