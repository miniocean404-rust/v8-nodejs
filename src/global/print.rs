pub(crate) fn print(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    _return_value: v8::ReturnValue,
) {
    let value = args.get(0).to_string(scope).unwrap();
    println!("{}", value.to_rust_string_lossy(scope));
}
