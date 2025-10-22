use std::{io, path::Path};
use zjs::JsRuntime;

/// 异步主函数入口
/// 使用 #[tokio::main] 宏来启动 tokio 异步运行时
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 创建 v8 引擎运行时
    let mut runtime = JsRuntime::new();

    let filename = file!(); // 获取当前文件相对路径（编译期宏）
    let path = Path::new(filename); // 转换为 Path

    let not_found_error_fn = || Box::new(io::Error::new(io::ErrorKind::NotFound, "文件不存在"));
    // 获取目录
    let dirname = path.parent().ok_or_else(not_found_error_fn)?;

    let main_js_path = dirname.join("./js/main.js"); // 构造 JS 文件路径

    runtime.execute(&main_js_path.to_string_lossy()).await;

    Ok(()) // 返回成功
}
