mod async_task;
mod fs;

use async_task::{AsyncTaskDispatcher, TokioAsyncTaskManager};
use fs::create_fs;
use v8::{self, ContextOptions, Local, OwnedIsolate, Value};

pub struct JsRuntime<D: AsyncTaskDispatcher = TokioAsyncTaskManager> {
    isolate: v8::OwnedIsolate,
    task_dispatcher: D,
}

impl<D: AsyncTaskDispatcher> Default for JsRuntime<D> {
    fn default() -> Self {
        // 初始化 V8
        let platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(platform);
        v8::V8::initialize();

        // 创建一个新的 Isolate
        let isolate = v8::Isolate::new(Default::default());

        Self {
            isolate,
            task_dispatcher: D::default(),
        }
    }
}

fn print(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    _return_value: v8::ReturnValue,
) {
    let value = args.get(0).to_string(scope).unwrap();
    println!("{}", value.to_rust_string_lossy(scope));
}

impl JsRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn execute(&mut self, code: &str) -> Local<'_, Value> {
        let isolate_ptr = &mut self.isolate as *mut OwnedIsolate;

        let scope = &mut v8::HandleScope::new(unsafe { &mut *isolate_ptr });

        self.isolate
            .set_data(0, &self.task_dispatcher as *const _ as *mut _);

        let global_template = v8::ObjectTemplate::new(scope);

        // 添加全局 print 函数
        let print_name = v8::String::new(scope, "print").unwrap();
        let print_func = v8::FunctionTemplate::new(scope, print);
        global_template.set(print_name.into(), print_func.into());

        // 添加 fs 模块
        let fs = create_fs(scope);
        global_template.set(v8::String::new(scope, "fs").unwrap().into(), fs.into());

        let context = v8::Context::new(
            scope,
            ContextOptions {
                global_template: global_template.into(),
                ..Default::default()
            },
        );

        let scope = &mut v8::ContextScope::new(scope, context);

        let source = v8::String::new(scope, code).unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();

        // 运行脚本获取全局对象
        script.run(scope).unwrap();

        // 从全局对象获取 main 函数
        let global = context.global(scope);
        let main_str = v8::String::new(scope, "main").unwrap();
        let main_fn = global.get(scope, main_str.into()).unwrap();

        if !main_fn.is_function() {
            panic!("main function not found");
        }

        // 调用 main 函数
        println!("run main function");
        let undefined = v8::undefined(scope);
        let result = main_fn
            .cast::<v8::Function>()
            .call(scope, undefined.into(), &[])
            .unwrap();

        self.task_dispatcher
            .run_event_loop(unsafe { &mut *isolate_ptr }, scope)
            .await;

        result
    }
}
