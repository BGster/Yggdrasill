//! `ygg.rt.frame.*` 渲染面 Lua 键（S31 R2 五键；契约 rt-face.md 渲染面节）。
//!
//! - `open` **装载期限定**（seal 后 `YGG-RT-SEALED`，与 `ygg.rt.config` 同一条
//!   运行期禁令——面在一局装载时开，运行期只写不换面）；v1 单面（再开 =
//!   `YGG-RT-OPENED`），handle 整数 1 起。
//! - `clear/span/blit/present` 运行期写键：全整数参数（浮点 =
//!   `YGG-RT-SHAPE`），直写零混色，越界钳制（越界部分静默裁掉——射线柱
//!   超屏是常态非错误）。rgba = i64 打包 `0xRRGGBBAA`。
//! - 缓冲语义在 `crate::frame`（纯 Rust 面）；本文件只做 Lua 取参 + 错误码
//!   映射 + rt 帧号读出，零位图逻辑。

use super::HostState;
use mlua::{Error, Lua, Table, Value as LuaValue};
use std::cell::RefCell;
use std::rc::Rc;

use crate::frame::{FrameError, FRAME_MAX_EDGE};

/// rgba 取参：整数 0..=0xFFFFFFFF（0xRRGGBBAA 打包）。
fn want_rgba(v: &LuaValue) -> Result<u32, Error> {
    match v {
        LuaValue::Integer(i) if (0..=0xFFFF_FFFF).contains(i) => Ok(*i as u32),
        other => Err(shape_err(&format!(
            "`rgba` 必须是 0..=0xFFFFFFFF 的整数（0xRRGGBBAA 打包），得到 {other:?}"
        ))),
    }
}

/// 严格整数取参（渲染面全整数：浮点/其他类型一律 `YGG-RT-SHAPE`）。
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

fn frame_err(err: FrameError) -> Error {
    Error::runtime(format!("{}: {err}", err.code()))
}

/// 装 `ygg.rt.frame.*`（挂 `ygg.rt` 表下；open 装载期限定，写键运行期）。
pub(super) fn install_frame(lua: &Lua, state: &Rc<RefCell<HostState>>) -> Result<(), Error> {
    let ygg: Table = lua.globals().raw_get("ygg")?;
    let rt: Table = ygg.raw_get("rt")?;
    let frame = lua.create_table()?;

    // ---- ygg.rt.frame.open({w=,h=}) —— 装载期限定，v1 单面 ----
    {
        let st = Rc::clone(state);
        frame.raw_set(
            "open",
            lua.create_function(move |_, cfg: Table| -> Result<i64, Error> {
                if st.borrow().sealed {
                    return Err(Error::runtime(
                        "YGG-RT-SEALED: `ygg.rt.frame.open` 装载期限定（面在一局装载时开，运行期只写不换面）",
                    ));
                }
                // 形状闭集：只接受 w/h 键（fail-fast，与 ygg.rt.config 同纪律）。
                for pair in cfg.pairs::<LuaValue, LuaValue>() {
                    let (key, _) = pair?;
                    let name = match &key {
                        LuaValue::String(s) => s.to_str().ok().map(|n| n.to_string()),
                        _ => None,
                    };
                    if !matches!(name.as_deref(), Some("w") | Some("h")) {
                        return Err(shape_err(
                            "`ygg.rt.frame.open` 只接受 `w` / `h` 键",
                        ));
                    }
                }
                let w = want_int("w", &cfg.raw_get("w")?)?;
                let h = want_int("h", &cfg.raw_get("h")?)?;
                let face = Rc::clone(&st.borrow().frame);
                let opened = face.borrow_mut().open(w, h).map_err(frame_err);
                opened
            })?,
        )?;
    }

    // ---- ygg.rt.frame.clear(handle, rgba) ----
    {
        let st = Rc::clone(state);
        frame.raw_set(
            "clear",
            lua.create_function(
                move |_, (handle, rgba): (LuaValue, LuaValue)| -> Result<(), Error> {
                    let handle = want_int("handle", &handle)?;
                    let rgba = want_rgba(&rgba)?;
                    check_handle(&st.borrow(), handle)?;
                    let face = Rc::clone(&st.borrow().frame);
                    let done = face.borrow_mut().clear(rgba).map_err(frame_err);
                    done
                },
            )?,
        )?;
    }

    // ---- ygg.rt.frame.span(handle, x, y, w, hh, rgba) ----
    {
        let st = Rc::clone(state);
        frame.raw_set(
            "span",
            lua.create_function(
                move |_,
                      (handle, x, y, w, hh, rgba): (
                    LuaValue,
                    LuaValue,
                    LuaValue,
                    LuaValue,
                    LuaValue,
                    LuaValue,
                )|
                      -> Result<(), Error> {
                    let handle = want_int("handle", &handle)?;
                    let x = want_int("x", &x)?;
                    let y = want_int("y", &y)?;
                    let w = want_int("w", &w)?;
                    let hh = want_int("hh", &hh)?;
                    let rgba = want_rgba(&rgba)?;
                    check_handle(&st.borrow(), handle)?;
                    let face = Rc::clone(&st.borrow().frame);
                    let done = face.borrow_mut().span(x, y, w, hh, rgba).map_err(frame_err);
                    done
                },
            )?,
        )?;
    }

    // ---- ygg.rt.frame.blit(handle, x, y, w, bytes) ----
    {
        let st = Rc::clone(state);
        frame.raw_set(
            "blit",
            lua.create_function(
                move |_,
                      (handle, x, y, w, bytes): (
                    LuaValue,
                    LuaValue,
                    LuaValue,
                    LuaValue,
                    LuaValue,
                )|
                      -> Result<(), Error> {
                    let handle = want_int("handle", &handle)?;
                    let x = want_int("x", &x)?;
                    let y = want_int("y", &y)?;
                    let w = want_int("w", &w)?;
                    let raw = match &bytes {
                        LuaValue::String(s) => s.as_bytes().to_vec(),
                        other => {
                            return Err(shape_err(&format!(
                                "`bytes` 必须是字符串（RGBA8 行串），得到 {}",
                                other.type_name()
                            )))
                        }
                    };
                    check_handle(&st.borrow(), handle)?;
                    let face = Rc::clone(&st.borrow().frame);
                    let done = face.borrow_mut().blit(x, y, w, &raw).map_err(frame_err);
                    done
                },
            )?,
        )?;
    }

    // ---- ygg.rt.frame.present(handle) → {digest, dirty, frame} ----
    {
        let st = Rc::clone(state);
        frame.raw_set(
            "present",
            lua.create_function(move |lua, handle: LuaValue| -> Result<Table, Error> {
                let handle = want_int("handle", &handle)?;
                let frame_n = st.borrow().rt.borrow().n;
                check_handle(&st.borrow(), handle)?;
                let face = Rc::clone(&st.borrow().frame);
                let info = face.borrow_mut().present(frame_n).map_err(frame_err)?;
                let out = lua.create_table()?;
                out.raw_set("digest", info.digest)?;
                if let Some(rect) = info.dirty {
                    let t = lua.create_table()?;
                    t.raw_set("x", rect.x)?;
                    t.raw_set("y", rect.y)?;
                    t.raw_set("w", rect.w)?;
                    t.raw_set("h", rect.h)?;
                    out.raw_set("dirty", t)?;
                }
                out.raw_set("frame", info.frame)?;
                Ok(out)
            })?,
        )?;
    }

    rt.raw_set("frame", frame)?;
    Ok(())
}

/// handle 校验（v1 单面：必须与 open 返回的当前 handle 一致）。
fn check_handle(state: &HostState, handle: i64) -> Result<(), Error> {
    let face = state.frame.borrow();
    if !face.is_open() {
        Err(shape_err("frame 未 open（open 为装载期限定）"))
    } else if face.handle() != handle {
        Err(shape_err(&format!(
            "handle {handle} 不符（当前面 handle = {}）",
            face.handle()
        )))
    } else {
        Ok(())
    }
}

// FRAME_MAX_EDGE 由 crate::frame 消费；此处再导出供文档注释核对面。
const _: () = assert!(FRAME_MAX_EDGE == 4096);
