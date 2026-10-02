//! ============================================================================
//!  mock_launcher.rs - Wine 验证用替身 EXE
//! ============================================================================
//!  被 `pptx_selector` 通过 `CreateProcessW` 启动时，把收到的命令行参数原样写入
//!  一个文本文件，用于断言：
//!  - 命令行拼接与引号转义是否正确；
//!  - Unicode（中文）与带空格路径是否原样传递；
//!  - 决策/降级/兜底最终启动的是哪一个程序。
//!
//!  输出位置：本 EXE 同目录下的 `launched_<EXE 文件名主干>.txt`。
//!  每行格式：`<序号>\t<可读形式>\t<UTF-16 码元十六进制，空格分隔>`
//!  这样中文与空格都可以被精确断言（可读形式经过 lossy 转换，仅作参考）。
//!
//!  本文件位于 `examples/`，不参与正式二进制，也不受"运行路径禁止 unwrap/expect"
//!  的约束（它只是测试替身）。同时保证在 Linux 上也能编译，便于 `cargo clippy
//!  --all-targets` 通过。
//! ============================================================================

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::ffi::OsStr;
use std::path::PathBuf;

/// 取得 OsStr 的 UTF-16 码元序列（Windows 上是原生表示）。
#[cfg(windows)]
fn wide(value: &OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().collect()
}

#[cfg(not(windows))]
fn wide(value: &OsStr) -> Vec<u16> {
    value.to_string_lossy().encode_utf16().collect()
}

fn main() {
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();

    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("mock.exe"));
    let stem = exe
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "mock".to_string());
    let directory = exe.parent().map(PathBuf::from).unwrap_or_default();
    let output = directory.join(format!("launched_{stem}.txt"));

    let mut text = String::new();
    for (index, arg) in args.iter().enumerate() {
        let units = wide(arg);
        let hex: Vec<String> = units.iter().map(|u| format!("{u:04X}")).collect();
        text.push_str(&format!(
            "{index}\t{}\t{}\n",
            arg.to_string_lossy(),
            hex.join(" ")
        ));
    }

    let _ = std::fs::write(&output, text);
    std::process::exit(0);
}
