use v8::{FunctionCallback, MapFnTo};

pub mod module_loader;
mod print;

fn inject_global_method(
    scope: &mut v8::HandleScope<'_, ()>,
    global_template: &v8::ObjectTemplate,
    method_name: &str,
    method_func: impl MapFnTo<FunctionCallback>,
) {
    let method_name = v8::String::new(scope, method_name).unwrap();
    let method_func = v8::FunctionTemplate::new(scope, method_func);
    global_template.set(method_name.into(), method_func.into());
}

pub(crate) fn inject_global_values(
    scope: &mut v8::HandleScope<'_, ()>,
    global_template: &v8::ObjectTemplate,
) {
    inject_global_method(scope, global_template, "print", print::print);
}
