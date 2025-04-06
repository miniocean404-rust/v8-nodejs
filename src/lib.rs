mod builtin;
mod global;

use builtin::async_task::{AsyncTaskDispatcher, TokioAsyncTaskManager};
use builtin::fs::create_fs;
use global::inject_global_values;
use global::module_loader::ModuleLoader;
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

// TODO
fn host_import_module_dynamically_callback_example<'s>(
    scope: &mut v8::HandleScope<'s>,
    host_defined_options: v8::Local<'s, v8::Data>,
    resource_name: v8::Local<'s, v8::Value>,
    specifier: v8::Local<'s, v8::String>,
    import_assertions: v8::Local<'s, v8::FixedArray>,
) -> Option<v8::Local<'s, v8::Promise>> {
    todo!()
}

impl JsRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn execute(&mut self, entry_script_path: &str) -> Local<'_, Value> {
        let isolate_ptr = &mut self.isolate as *mut OwnedIsolate;

        let scope = &mut v8::HandleScope::new(unsafe { &mut *isolate_ptr });

        self.isolate
            .set_data(0, &self.task_dispatcher as *const _ as *mut _);

        let module_loader = ModuleLoader::inject_into_isolate(&mut self.isolate);

        let global_template = v8::ObjectTemplate::new(scope);
        inject_global_values(scope, &global_template);

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

        self.isolate.set_host_import_module_dynamically_callback(
            host_import_module_dynamically_callback_example,
        );

        let scope = &mut v8::ContextScope::new(scope, context);

        let module = module_loader
            .create_first_module(scope, entry_script_path)
            .unwrap();
        module.evaluate(scope).unwrap();

        let module_namespace = module.get_module_namespace();

        let main_fn_name = v8::String::new(scope, "main").unwrap();

        let main_fn = module_namespace
            .to_object(scope)
            .unwrap()
            .get(scope, main_fn_name.into())
            .unwrap();

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
