use zjs::JsRuntime;

#[tokio::main]
async fn main() {
    let mut runtime = JsRuntime::new();
    let current_dir = std::env::current_dir().unwrap();
    let entry_script_path = current_dir.join("examples/example.js");
    runtime.execute(&entry_script_path.to_string_lossy()).await;
}
