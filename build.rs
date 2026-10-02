//! 构建脚本：为 Windows target 嵌入图标。
//!
//! 已知陷阱：`build.rs` 在 **host** 上执行，`#[cfg(target_os = "windows")]`
//! 在交叉编译时不会按预期生效，必须改用 `CARGO_CFG_TARGET_OS` 环境变量判断。
//!
//! `build.rs` 属于构建脚本，不受"运行路径禁止 unwrap/expect"的约束：
//! 资源编译失败时直接 expect 终止构建，以便尽早暴露工具链问题。

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("resources/icon.ico");
        res.compile().expect("failed to compile Windows resources");
    }
}
