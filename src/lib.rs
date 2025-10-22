mod builtin;
mod global;
mod helper;

use builtin::async_task::{AsyncTaskDispatcher, TokioAsyncTaskManager};
use global::inject_global_values;
use global::module_loader::{host_initialize_import_meta_object_callback, ModuleLoader};
use v8::{self, ContextOptions, Local, OwnedIsolate, Value};

pub struct JsRuntime<D: AsyncTaskDispatcher = TokioAsyncTaskManager> {
    // V8 隔离区（独立的独立的堆内存 JS 执行环境）管理 JavaScript 对象的生命周期、堆内存管理、垃圾回收器、全局对象和上下文
    isolate: v8::OwnedIsolate,
    // 异步任务调度器
    task_dispatcher: D,
}

impl<D: AsyncTaskDispatcher> Default for JsRuntime<D> {
    // 初始化 V8 引擎
    fn default() -> Self {
        // 创建 V8 平台，参数 0 表示线程数，false 表示不启用调试
        let platform = v8::new_default_platform(0, false).make_shared();
        // 初始化 V8 平台
        v8::V8::initialize_platform(platform);
        // 初始化 V8 引擎
        v8::V8::initialize();

        // 创建 V8 隔离区（隔离的 JS 执行环境）
        let isolate = v8::Isolate::new(Default::default());

        Self {
            isolate,
            // 创建默认的异步任务管理器
            task_dispatcher: D::default(),
        }
    }
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
        let scope = &mut v8::HandleScope::new(unsafe { &mut *isolate_ptr }); // 在这个作用域内创建的所有 JavaScript 值都会被追踪, 当 scope 离开作用域时，自动清理未被引用的对象（临时的"工作台"，管理当前正在使用的 JavaScript 值的句柄）

        let task_dispatcher_ptr = &self.task_dispatcher as *const _ as *mut _;
        self.isolate.set_data(0, task_dispatcher_ptr); // 在 isolate 中存储异步任务管理器的指针, 以便后续 run_event_loop 时使用
        let module_loader = ModuleLoader::init_and_inject(&mut self.isolate); // 在隔离上下文中注入 module_loader 来管理路径、模块、文件之间的关联

        let global_api_template = v8::ObjectTemplate::new(scope); // 创建对象模板, v8::ObjectTemplate 允许你在 Rust 中预定义 JavaScript 对象的结构，包括属性、方法和访问器，然后基于这个模板快速创建多个相似的对象。
        inject_global_values(scope, &global_api_template); // 注入 Global API, 目前有 print 函数

        // 创建 V8 执行上下文, 注入 Global API 方法
        let context = v8::Context::new(
            scope,
            ContextOptions {
                global_template: global_api_template.into(), // 使用自定义全局模板
                ..Default::default()                         // 其他选项使用默认值
            },
        );

        // TODO: 设置动态 import() 的处理函数
        self.isolate.set_host_import_module_dynamically_callback(
            host_import_module_dynamically_callback_example,
        );

        // 设置 import.meta 初始化函数, 为 import.meta.dirname 设置值
        self.isolate
            .set_host_initialize_import_meta_object_callback(
                host_initialize_import_meta_object_callback,
            );

        let scope = &mut v8::ContextScope::new(scope, context); // 在新上下文中创建作用域

        // 加载并编译入口模块
        let module = module_loader
            .create_first_module(scope, entry_script_path)
            .unwrap();

        // 执行模块（顶级代码）
        module.evaluate(scope).unwrap();

        let module_namespace = module.get_module_namespace(); // 获取 js 模块导出的命名空间
        let main_fn_name = v8::String::new(scope, "main").unwrap();

        // 获取 main 函数
        let main_fn = module_namespace
            .to_object(scope)
            .unwrap()
            .get(scope, main_fn_name.into())
            .unwrap();

        // 检查是否确实是函数
        if !main_fn.is_function() {
            panic!("main 函数不存在");
        }

        let undefined = v8::undefined(scope); // 创建 undefined 值

        // 调用 main 函数（绑定 undefined 为函数的 this 参数，&[] 为参数列表）
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
    _scope: &mut v8::HandleScope<'s>,
    _host_defined_options: v8::Local<'s, v8::Data>,
    _resource_name: v8::Local<'s, v8::Value>,
    _specifier: v8::Local<'s, v8::String>,
    _import_assertions: v8::Local<'s, v8::FixedArray>,
) -> Option<v8::Local<'s, v8::Promise>> {
    todo!() // 标记为未实现
}
