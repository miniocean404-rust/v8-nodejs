use std::{io, path::Path};
use zjs::JsRuntime;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = JsRuntime::new();

    let filename = file!();

    let path = Path::new(filename);

    let not_found_error = || Box::new(io::Error::new(io::ErrorKind::NotFound, "File not found"));
    let dirname = path.parent().ok_or_else(not_found_error)?;

    let example_main_js_filepath = dirname.join("./js/main.js");

    runtime
        .execute(&example_main_js_filepath.to_string_lossy())
        .await;

    Ok(())
}
