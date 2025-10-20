/// 创建 V8 字符串的宏
///
/// 这是一个便利宏，用于简化 V8 字符串的创建
/// 如果创建失败则 panic
///
/// # 示例
/// ```ignore
/// let my_string = v8_string!(scope, "hello");
/// ```
#[macro_export]
macro_rules! v8_string {
    ($scope:expr, $value:expr) => {
        v8::String::new($scope, $value).unwrap() // 创建 V8 字符串并 unwrap（假定成功）
    };
}
