//! `ygg.rt.audio.*` 音频面 Lua 键（S31 R3 三键；契约 rt-face.md 音频面节）。
//!
//! - `open` **装载期限定**（seal 后 `YGG-RT-SEALED`，与 `ygg.rt.config` /
//!   `ygg.rt.frame.open` 同一条运行期禁令——面在一局装载时开，运行期只
//!   写不换面）；v1 单混音面（再开 = `YGG-RT-OPENED`），handle 整数 1 起；
//!   ch 固定 1（Saltline bundle 10 条全单声道，mono 下 pan 无定义）。
//! - `play` / `stop` 运行期写键：pcm = s16 小端单声道字符串（assetgen
//!   `sound.*` 直喂，0 < 样本数 ≤ rate×10s），gain 千分比整数 0..=2000
//!   缺省 1000（**入队时烘焙**，后改不追溯 = 确定性）；voice 整数 1 起；
//!   并发 ≤ 64 超限 `YGG-RT-AUDIO-CAP`。
//! - 混音语义在 `crate::audio`（纯 Rust 面）；本文件只做 Lua 取参 + 错误
//!   码映射 + handle 校验，零混音逻辑。

use super::HostState;
use mlua::{Error, Lua, Table, Value as LuaValue};
use std::cell::RefCell;
use std::rc::Rc;

use crate::audio::{AudioError, AUDIO_GAIN_DEFAULT};

/// 严格整数取参（音频面全整数：浮点/其他类型一律 `YGG-RT-SHAPE`）。
fn want_int(name: &str, v: &LuaValue) -> Result<i64, Error> {
    match v {
        LuaValue::Integer(i) => Ok(*i),
        other => Err(shape_err(&format!(
            "`{name}` 必须是整数，得到 {}",
            other.type_name()
        ))),
    }
}

fn shape_err(what: &str) -> Error {
    Error::runtime(format!("YGG-RT-SHAPE: {what}"))
}

fn audio_err(err: AudioError) -> Error {
    Error::runtime(format!("{}: {err}", err.code()))
}

/// 装 `ygg.rt.audio.*`（挂 `ygg.rt` 表下；open 装载期限定，play/stop 运行期）。
pub(super) fn install_audio(lua: &Lua, state: &Rc<RefCell<HostState>>) -> Result<(), Error> {
    let ygg: Table = lua.globals().raw_get("ygg")?;
    let rt: Table = ygg.raw_get("rt")?;
    let audio = lua.create_table()?;

    // ---- ygg.rt.audio.open({rate=?}) —— 装载期限定，v1 单混音面 ----
    {
        let st = Rc::clone(state);
        audio.raw_set(
            "open",
            lua.create_function(move |_, cfg: Table| -> Result<i64, Error> {
                if st.borrow().sealed {
                    return Err(Error::runtime(
                        "YGG-RT-SEALED: `ygg.rt.audio.open` 装载期限定（面在一局装载时开，运行期只写不换面）",
                    ));
                }
                // 形状闭集：只接受 `rate` 键（fail-fast，与 ygg.rt.config 同纪律）。
                for pair in cfg.pairs::<LuaValue, LuaValue>() {
                    let (key, _) = pair?;
                    let name = match &key {
                        LuaValue::String(s) => s.to_str().ok().map(|n| n.to_string()),
                        _ => None,
                    };
                    if !matches!(name.as_deref(), Some("rate")) {
                        return Err(shape_err("`ygg.rt.audio.open` 只接受 `rate` 键"));
                    }
                }
                let rate = match cfg.raw_get("rate")? {
                    LuaValue::Nil => None,
                    value => Some(want_int("rate", &value)?),
                };
                let face = Rc::clone(&st.borrow().audio);
                let opened = face.borrow_mut().open(rate).map_err(audio_err);
                opened
            })?,
        )?;
    }

    // ---- ygg.rt.audio.play(handle, pcm, gain?) → voice ----
    {
        let st = Rc::clone(state);
        audio.raw_set(
            "play",
            lua.create_function(
                move |_,
                      (handle, pcm, gain): (
                    LuaValue,
                    LuaValue,
                    LuaValue,
                )|
                      -> Result<i64, Error> {
                    let handle = want_int("handle", &handle)?;
                    let raw = match &pcm {
                        LuaValue::String(s) => s.as_bytes().to_vec(),
                        other => {
                            return Err(shape_err(&format!(
                                "`pcm` 必须是字符串（s16 小端单声道串），得到 {}",
                                other.type_name()
                            )))
                        }
                    };
                    // gain 缺省 1000（千分比 = 原样）；其余形状交给 Rust 面。
                    let gain = match gain {
                        LuaValue::Nil => AUDIO_GAIN_DEFAULT,
                        ref value => want_int("gain", value)?,
                    };
                    check_handle(&st.borrow(), handle)?;
                    let face = Rc::clone(&st.borrow().audio);
                    let voice = face.borrow_mut().play(&raw, gain).map_err(audio_err);
                    voice
                },
            )?,
        )?;
    }

    // ---- ygg.rt.audio.stop(handle, voice) ----
    {
        let st = Rc::clone(state);
        audio.raw_set(
            "stop",
            lua.create_function(
                move |_, (handle, voice): (LuaValue, LuaValue)| -> Result<(), Error> {
                    let handle = want_int("handle", &handle)?;
                    let voice = want_int("voice", &voice)?;
                    check_handle(&st.borrow(), handle)?;
                    let face = Rc::clone(&st.borrow().audio);
                    face.borrow_mut().stop(voice);
                    Ok(())
                },
            )?,
        )?;
    }

    rt.raw_set("audio", audio)?;
    Ok(())
}

/// handle 校验（v1 单混音面：必须与 open 返回的当前 handle 一致）。
fn check_handle(state: &HostState, handle: i64) -> Result<(), Error> {
    let face = state.audio.borrow();
    if !face.is_open() {
        Err(shape_err("audio 未 open（open 为装载期限定）"))
    } else if face.handle() != handle {
        Err(shape_err(&format!(
            "handle {handle} 不符（当前面 handle = {}）",
            face.handle()
        )))
    } else {
        Ok(())
    }
}
