use std::{io, path::Path};  // 标准库导入
use zjs::JsRuntime;  // 导入我们的运行时

/// 异步主函数入口
/// 使用 #[tokio::main] 宏来启动 tokio 异步运行时
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = JsRuntime::new();  // 创建 JS 运行时

    let filename = file!();  // 获取当前文件名（编译期宏）

    let path = Path::new(filename);  // 转换为 Path

    // 错误工厂函数
    let not_found_error = || Box::new(io::Error::new(io::ErrorKind::NotFound, "File not found"));
    let dirname = path.parent().ok_or_else(not_found_error)?;  // 获取目录，如果不存在则返回错误

    let example_main_js_filepath = dirname.join("./js/main.js");  // 构造 JS 文件路径

    // 执行 JS 文件
    runtime
        .execute(&example_main_js_filepath.to_string_lossy())
        .await;

    Ok(())  // 返回成功
}
