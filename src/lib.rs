mod builtin; // 导入内置模块（异步任务、文件系统）
mod global; // 导入全局模块（全局函数、模块加载）
mod helper; // 导入辅助宏

use builtin::async_task::{AsyncTaskDispatcher, TokioAsyncTaskManager}; // 异步任务管理
use global::inject_global_values; // 注入全局值的函数
use global::module_loader::{host_initialize_import_meta_object_callback, ModuleLoader}; // 模块加载
use v8::{self, ContextOptions, Local, OwnedIsolate, Value}; // V8 类型

/// JS 运行时结构体，支持泛型异步管理器
///
/// 参数 D: 异步任务调度器（默认为 TokioAsyncTaskManager）
pub struct JsRuntime<D: AsyncTaskDispatcher = TokioAsyncTaskManager> {
    isolate: v8::OwnedIsolate, // V8 隔离区（独立的 JS 执行环境）
    task_dispatcher: D,        // 异步任务调度器
}

impl<D: AsyncTaskDispatcher> Default for JsRuntime<D> {
    fn default() -> Self {
        // 初始化 V8 引擎
        let platform = v8::new_default_platform(0, false).make_shared(); // 创建 V8 平台，参数 0 表示线程数，false 表示不启用调试
        v8::V8::initialize_platform(platform); // 初始化 V8 平台
        v8::V8::initialize(); // 初始化 V8 引擎

        // 创建一个新的 Isolate
        let isolate = v8::Isolate::new(Default::default()); // 创建 V8 隔离区（隔离的 JS 执行环境）

        Self {
            isolate,
            task_dispatcher: D::default(), // 创建默认的异步任务管理器
        }
    }
}

// TODO - 待完成的动态 import() 处理
/// 处理动态 import() 的回调函数
///
/// # 参数
/// - `scope`: V8 作用域，用于 GC 跟踪
/// - `host_defined_options`: 主机定义的选项
/// - `resource_name`: 资源名称（通常是文件名）
/// - `specifier`: 模块标识符（import() 中的字符串）
/// - `import_assertions`: import 断言（ES2023 功能）
fn host_import_module_dynamically_callback_example<'s>(
    scope: &mut v8::HandleScope<'s>,
    host_defined_options: v8::Local<'s, v8::Data>,
    resource_name: v8::Local<'s, v8::Value>,
    specifier: v8::Local<'s, v8::String>,
    import_assertions: v8::Local<'s, v8::FixedArray>,
) -> Option<v8::Local<'s, v8::Promise>> {
    todo!() // 标记为未实现
}

impl JsRuntime {
    /// 创建新的 JsRuntime 实例
    pub fn new() -> Self {
        Self::default()
    }

    /// 异步执行 JS 脚本
    ///
    /// # 参数
    /// - `entry_script_path`: JS 脚本文件路径
    ///
    /// # 返回
    /// 返回 main() 函数的执行结果
    pub async fn execute(&mut self, entry_script_path: &str) -> Local<'_, Value> {
        let isolate_ptr = &mut self.isolate as *mut OwnedIsolate; // 获取 isolate 的可变指针（用于 unsafe 操作）

        let scope = &mut v8::HandleScope::new(unsafe { &mut *isolate_ptr }); // 创建作用域

        // 在 isolate 中存储异步任务管理器的指针（slot 0）
        self.isolate
            .set_data(0, &self.task_dispatcher as *const _ as *mut _);

        let module_loader = ModuleLoader::inject_into_isolate(&mut self.isolate); // 创建并注入模块加载器

        let global_template = v8::ObjectTemplate::new(scope); // 创建全局对象模板
        inject_global_values(scope, &global_template); // 注入全局值（如 print 函数）

        // 创建 V8 执行上下文
        let context = v8::Context::new(
            scope,
            ContextOptions {
                global_template: global_template.into(), // 使用自定义全局模板
                ..Default::default()                     // 其他选项使用默认值
            },
        );

        // 设置动态 import() 的处理函数
        self.isolate.set_host_import_module_dynamically_callback(
            host_import_module_dynamically_callback_example,
        );

        // 设置 import.meta 初始化函数
        self.isolate
            .set_host_initialize_import_meta_object_callback(
                host_initialize_import_meta_object_callback,
            );

        let scope = &mut v8::ContextScope::new(scope, context); // 在新上下文中创建作用域

        // 加载并编译入口模块
        let module = module_loader
            .create_first_module(scope, entry_script_path)
            .unwrap();
        module.evaluate(scope).unwrap(); // 执行模块（顶级代码）

        let module_namespace = module.get_module_namespace(); // 获取模块导出的命名空间

        let main_fn_name = v8::String::new(scope, "main").unwrap(); // 创建字符串 "main"

        // 获取 main 函数
        let main_fn = module_namespace
            .to_object(scope)
            .unwrap()
            .get(scope, main_fn_name.into())
            .unwrap();

        // 检查是否确实是函数
        if !main_fn.is_function() {
            panic!("main function not found"); // 如果不是函数则崩溃
        }

        let undefined = v8::undefined(scope); // 创建 undefined 值
                                              // 调用 main 函数（在 undefined 上下文中调用，无参数）
        let result = main_fn
            .cast::<v8::Function>()
            .call(scope, undefined.into(), &[])
            .unwrap();

        // 运行事件循环，处理所有异步任务
        self.task_dispatcher
            .run_event_loop(unsafe { &mut *isolate_ptr }, scope)
            .await;

        result // 返回 main 函数的执行结果
    }
}
