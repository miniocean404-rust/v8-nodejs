use v8::{FunctionCallback, MapFnTo}; // V8 回调函数类型

pub mod module_loader; // 模块加载器模块
mod print; // print 函数模块

/// 注入全局方法到全局对象模板
///
/// 这个函数用于将 Rust 实现的函数暴露给 JavaScript 代码
///
/// # 参数
/// - `scope`: V8 作用域
/// - `global_template`: 全局对象模板
/// - `method_name`: 方法名称（在 JS 中访问的名字）
/// - `method_func`: 函数实现
fn inject_global_method(
    scope: &mut v8::HandleScope<'_, ()>,
    global_template: &v8::ObjectTemplate,
    method_name: &str,                           // 方法名（如 "print"）
    method_func: impl MapFnTo<FunctionCallback>, // Rust 函数实现
) {
    let method_name = v8::String::new(scope, method_name).unwrap(); // 创建方法名字符串
    let method_func = v8::FunctionTemplate::new(scope, method_func); // 创建函数模板
    global_template.set(method_name.into(), method_func.into()); // 添加到全局对象模板
}

/// 注入所有全局值到 V8 上下文
///
/// 这个函数在创建 V8 上下文时调用，用于将 Rust 实现的全局函数暴露给 JavaScript
pub(crate) fn inject_global_values(
    scope: &mut v8::HandleScope<'_, ()>,
    global_template: &v8::ObjectTemplate,
) {
    // 注入 print 函数（允许 JS 代码调用 print() 打印到标准输出）
    inject_global_method(scope, global_template, "print", print::print);
}
