use zjs::JsRuntime;

#[tokio::main]
async fn main() {
    let mut runtime = JsRuntime::new();
    let code = include_str!("./example.js");
    runtime.execute(code).await;
}
