//! 包镜像生成器（M3.4b）：把 `<pack目录>` 打包为 `ygg/1` 无文件系统镜像。
//!
//! 走的是**真实装载口**（[`Loaded::from_dir`]）再经 [`ygg_host::image::to_image`] 导出：
//! 生成物保证已通过 `pack.json` / `manifest.json` / schema / 模块表三方对账，且与
//! [`ygg_host::image::load`] 严格互逆（wasm / C ABI 宿主无需文件系统即可装载）。
//!
//! 用法：
//!
//! ```bash
//! cargo run -q --example image -p ygg-host -- <pack目录> [--out <文件>] [--check]
//! ```
//!
//! - 缺省 `--out`：把镜像 JSON 打印到 stdout（末尾补换行）；
//! - 指定 `--out <文件>`：写出文件；若文件名以 `.js` 结尾（如 `hosts/h5/pack.image.js`），
//!   自动包成零构建 ES module 形态（`export const IMAGE = <json>;\n`），否则写纯 JSON；
//! - `--check`：与 `--out <文件>` 联用，读现有文件与现算内容逐字节比对，不一致则非 0 退出
//!   （不改盘，供门禁验证生成物没有与包源码漂移）。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::exit;

use ygg_host::image::to_image;
use ygg_sandbox::modload::Loaded;

/// ES module 生成物的自动头注：提醒审阅者此文件由本生成器产出，不要手改。
const JS_HEADER: &str =
    "// 生成物：由 `cargo run -q --example image -p ygg-host -- <pack目录> --out <文件>` 产出，勿手改。\n\
     export const IMAGE = ";

fn main() {
    let mut dir_arg: Option<PathBuf> = None;
    let mut out_arg: Option<PathBuf> = None;
    let mut check = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => {
                out_arg = match args.next() {
                    Some(value) => Some(PathBuf::from(value)),
                    None => fail("--out 需要一个输出文件路径".to_owned()),
                };
            }
            "--check" => check = true,
            "-h" | "--help" => {
                print_usage();
                exit(0);
            }
            other if other.starts_with("--") => fail(format!("未知参数：{other}")),
            other => {
                if dir_arg.is_some() {
                    fail(format!("只接受一个 <pack目录> 参数，多出了：{other}"));
                }
                dir_arg = Some(PathBuf::from(other));
            }
        }
    }

    let Some(dir) = dir_arg else {
        print_usage();
        exit(1);
    };
    if !dir.is_dir() {
        fail(format!("目录不存在或不可读：{}", dir.display()));
    }
    if check && out_arg.is_none() {
        fail("--check 必须配合 --out <文件> 指定要比对的现有文件".to_owned());
    }

    // 经真实装载口校验 pack / manifest / schema / 模块表，再由 to_image 序列化。
    let loaded = match Loaded::from_dir(&dir) {
        Ok(loaded) => loaded,
        Err(err) => fail(format!("装载 {} 失败：{err}", dir.display())),
    };
    let image_json = match to_image(&loaded) {
        Ok(json) => json,
        Err(err) => fail(format!("生成 {} 的镜像失败：{err}", dir.display())),
    };

    match out_arg {
        None => {
            println!("{image_json}");
        }
        Some(out_path) => {
            let expected = format_output(&out_path, &image_json);
            if check {
                check_file(&out_path, &expected);
            } else {
                write_file(&out_path, &expected);
            }
        }
    }
}

/// 根据目标扩展名决定输出形态：`.js` / `.mjs` 包成 ES module，其余输出纯 JSON。
fn format_output(out_path: &Path, image_json: &str) -> String {
    let is_js = matches!(
        out_path.extension().and_then(|ext| ext.to_str()),
        Some("js" | "mjs")
    );
    if is_js {
        format!("{JS_HEADER}{image_json};\n")
    } else {
        format!("{image_json}\n")
    }
}

/// `--check`：读现有文件逐字节比对，不写盘；不一致则报错并非 0 退出。
fn check_file(out_path: &Path, expected: &str) {
    let actual = match fs::read_to_string(out_path) {
        Ok(text) => text,
        Err(err) => fail(format!(
            "校验失败：无法读取现有文件 {}：{err}",
            out_path.display()
        )),
    };
    if actual != expected {
        fail(format!(
            "镜像已漂移：{}（{} 字节）与当前包现算镜像（{} 字节）不一致；\
             请重新运行生成器刷新",
            out_path.display(),
            actual.len(),
            expected.len()
        ));
    }
    println!("ok: {} ({} bytes)", out_path.display(), actual.len());
}

/// 写出镜像文件（必要时创建父目录）。
fn write_file(out_path: &Path, content: &str) {
    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        if let Err(err) = fs::create_dir_all(parent) {
            fail(format!("创建目录 {} 失败：{err}", parent.display()));
        }
    }
    if let Err(err) = fs::write(out_path, content) {
        fail(format!("写入 {} 失败：{err}", out_path.display()));
    }
    println!(
        "镜像已生成 → {} ({} bytes)",
        out_path.display(),
        content.len()
    );
}

fn print_usage() {
    eprintln!(
        "用法: cargo run -q --example image -p ygg-host -- <pack目录> [--out <文件>] [--check]"
    );
    eprintln!("  缺省 --out：向 stdout 打印镜像 JSON");
    eprintln!("  --out <文件>：写出文件（.js/.mjs 自动包成 `export const IMAGE = ...;`）");
    eprintln!("  --check：与 --out 联用，比对现有文件与现算镜像，不一致则非 0 退出");
}

fn fail(msg: String) -> ! {
    eprintln!("错误: {msg}");
    exit(1);
}
