/// 全局 print 函数的 Rust 实现
///
/// 这个函数允许 JavaScript 代码通过 print() 函数打印信息到标准输出
///
/// # 参数
/// - `scope`: V8 作用域
/// - `args`: 包含了从 JavaScript 调用这个函数时传递的所有信息, 包含：- 所有传递的参数、- 参数数量、- this 对象、- 调用上下文信息
/// - `_return_value`: 返回值（这里未使用）
pub(crate) fn print(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    _return_value: v8::ReturnValue,
) {
    // 获取第一个参数并转换为字符串
    let value = args.get(0).to_string(scope).unwrap();
    // 打印到标准输出
    println!("{}", value.to_rust_string_lossy(scope));
}
