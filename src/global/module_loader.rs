use std::{
    collections::BTreeMap,
    fs::{self},
    path::{Path, PathBuf},
};

use v8::CallbackScope;

use crate::builtin::fs::create_fs;

pub struct ModuleLoader {
    // Maps module identity hash to its absolute path
    id_to_path_map: BTreeMap<i32, PathBuf>,
    // Caches compiled modules by absolute path
    // Use v8::Global to store modules across different scopes
    module_cache: BTreeMap<PathBuf, v8::Global<v8::Module>>,

    builtin_modules: BTreeMap<String, v8::Global<v8::Module>>,
}

impl ModuleLoader {
    pub fn inject_into_isolate(isolate: &mut v8::Isolate) -> &'static mut ModuleLoader {
        let module_loader = Box::into_raw(Box::new(Self {
            id_to_path_map: BTreeMap::new(),
            module_cache: BTreeMap::new(),
            builtin_modules: BTreeMap::new(),
        }));
        isolate.set_data(1, module_loader as *mut _);
        unsafe { &mut *module_loader }
    }

    // Helper function to compile a script into a V8 module and return the module and its ID
    fn compile_script_module<'s>(
        scope: &mut v8::HandleScope<'s>,
        code: &str,
        resource_name_str: &str,
    ) -> Option<(v8::Local<'s, v8::Module>, i32)> {
        let source = v8::String::new(scope, code)?;

        let file_url = v8::String::new(scope, "file://").unwrap();
        let resource_name = v8::String::new(scope, resource_name_str)?.into();

        // Create an Array to hold host-defined options
        let host_defined_options = v8::PrimitiveArray::new(scope, 1);
        host_defined_options.set(scope, 0, file_url.into());
        // We don't need to cast to PrimitiveArray, Array is a subclass of Data
        // let host_defined_options: v8::Local<v8::PrimitiveArray> = host_defined_options_arr.cast();

        let script_origin = v8::ScriptOrigin::new(
            scope,
            resource_name,
            0,
            0,
            false,
            0,
            None,
            false,
            false,
            true,
            Some(host_defined_options.into()), // host_defined_options (now an Array, cast to Data)
        );

        let mut source = v8::script_compiler::Source::new(source, Some(&script_origin));

        v8::script_compiler::compile_module(scope, &mut source).map(|module| {
            let hash_id: i32 = module.get_identity_hash().into();
            (module, hash_id)
        })
    }

    // Core function to get a compiled module, using cache or compiling on demand
    fn get_or_compile_module<'s>(
        &mut self,
        scope: &mut v8::HandleScope<'s>,
        absolute_path: &Path,
    ) -> Option<v8::Local<'s, v8::Module>> {
        let absolute_path_buf = absolute_path.to_path_buf();

        // Check if the module is already cached
        if let Some(global_module) = self.module_cache.get(&absolute_path_buf) {
            // Return the local handle from the global cache
            return Some(v8::Local::new(scope, global_module));
        }

        // Module not in cache, read and compile
        let content = match fs::read_to_string(absolute_path) {
            Ok(content) => content,
            Err(e) => {
                eprintln!("Error reading file '{}': {}", absolute_path.display(), e);
                return None;
            }
        };

        let resource_name_str = absolute_path.to_str().unwrap_or("unknown.js");

        if let Some((module, hash_id)) =
            Self::compile_script_module(scope, &content, resource_name_str)
        {
            // Store the mapping from hash ID to path (needed for resolving)
            self.id_to_path_map
                .insert(hash_id, absolute_path_buf.clone());

            // Instantiate the module before caching (important step)
            // Need to handle potential instantiation errors.
            if module
                .instantiate_module(scope, resolve_module_callback)
                .is_none()
            {
                eprintln!(
                    "Error: Failed to instantiate module: {}",
                    absolute_path.display()
                );
                // Don't cache if instantiation fails
                return None;
            }

            // Create a global handle and store it in the cache
            let global_module = v8::Global::new(scope, module);
            self.module_cache.insert(absolute_path_buf, global_module); // Add to module cache

            Some(module) // Return the local handle for the current scope
        } else {
            eprintln!(
                "Error: Failed to compile module: {}",
                absolute_path.display()
            );
            None // Compilation failed
        }
    }

    pub fn create_first_module<'s>(
        &mut self,
        scope: &mut v8::HandleScope<'s>,
        path_str: &str,
    ) -> Option<v8::Local<'s, v8::Module>> {
        // Canonicalize the path to get an absolute path
        let path = Path::new(path_str);
        let absolute_path = match fs::canonicalize(path) {
            Ok(p) => p,
            Err(e) => {
                eprintln!(
                    "Error canonicalizing entry point path '{}': {}",
                    path_str, e
                );
                return None;
            }
        };

        self.get_or_compile_module(scope, &absolute_path)
    }

    fn init_builtin_module(
        scope: &mut v8::HandleScope<'_>,
        specifier_str: &str,
    ) -> v8::Global<v8::Module> {
        let fs_module_name = v8::String::new(scope, specifier_str).unwrap();

        let export_names = &[v8::String::new(scope, "default").unwrap()];

        let module = v8::Module::create_synthetic_module(
            scope,
            fs_module_name,
            export_names,
            |context: v8::Local<'_, v8::Context>, module: v8::Local<'_, v8::Module>| {
                let mut scope = unsafe { CallbackScope::new(context) };
                let default_export_name = v8::String::new(&mut scope, "default").unwrap();
                let fs_module = create_fs(&mut scope);
                let value = fs_module.new_instance(&mut scope).unwrap();

                let result = module.set_synthetic_module_export(
                    &mut scope,
                    default_export_name,
                    value.into(),
                );

                result.map(|result| v8::Boolean::new(&mut scope, result).into())
            },
        );

        v8::Global::new(scope, module)
    }

    pub fn load_builtin_module<'s>(
        &mut self,
        scope: &mut v8::HandleScope<'s>,
        specifier_str: &str,
    ) -> Option<v8::Local<'s, v8::Module>> {
        let module = self
            .builtin_modules
            .entry(specifier_str.to_string())
            .or_insert_with(|| Self::init_builtin_module(scope, specifier_str));

        Some(v8::Local::new(scope, &*module))
    }
}

pub fn resolve_module_callback<'s>(
    context: v8::Local<'s, v8::Context>,
    specifier: v8::Local<'s, v8::String>,
    _import_assertions: v8::Local<'s, v8::FixedArray>,
    referrer: v8::Local<'s, v8::Module>,
) -> Option<v8::Local<'s, v8::Module>> {
    let mut scope = unsafe { v8::CallbackScope::new(context) };
    // Correctly retrieve the ModuleLoader instance stored in the isolate data slot.
    let state_ptr = scope.get_data(1);
    if state_ptr.is_null() {
        eprintln!("Error: ModuleLoader state is null in resolve_module_callback.");
        return None;
    }
    let module_loader = unsafe { &mut *(state_ptr as *mut ModuleLoader) };

    let specifier_str = specifier.to_rust_string_lossy(&mut scope);

    if !specifier_str.starts_with('.') && !specifier_str.starts_with('/') {
        // Handle built-in modules (specifiers without path structure)
        return module_loader.load_builtin_module(&mut scope, &specifier_str);
    }

    let referrer_id: i32 = referrer.get_identity_hash().into();
    let referrer_path = module_loader.id_to_path_map.get(&referrer_id)?;

    let referrer_dir = referrer_path.parent().unwrap_or(Path::new(""));
    let resolved_path_buf = referrer_dir.join(&specifier_str);

    const EXTENSIONS: [&str; 2] = ["", "js"];

    EXTENSIONS
        .iter()
        .find_map(|extension| {
            let mut resolved_path_with_extension = resolved_path_buf.clone();
            resolved_path_with_extension.set_extension(extension);
            fs::canonicalize(&resolved_path_with_extension).ok()
        })
        .and_then(|path| module_loader.get_or_compile_module(&mut scope, &path))
}

pub extern "C" fn host_initialize_import_meta_object_callback(
    context: v8::Local<'_, v8::Context>,
    module: v8::Local<'_, v8::Module>,
    meta: v8::Local<'_, v8::Object>,
) {
    let mut scope = unsafe { v8::CallbackScope::new(context) };
    let state_ptr = scope.get_data(1);
    if state_ptr.is_null() {
        eprintln!("Error: ModuleLoader state is null in resolve_module_callback.");
        return;
    }
    let module_loader = unsafe { &mut *(state_ptr as *mut ModuleLoader) };

    let module_id: i32 = module.get_identity_hash().into();
    let dir_name = module_loader
        .id_to_path_map
        .get(&module_id)
        .unwrap()
        .parent()
        .unwrap();

    let key = v8::String::new(&mut scope, "dirname").unwrap();
    let dir_name_str = v8::String::new(&mut scope, dir_name.to_str().unwrap()).unwrap();
    meta.set(&mut scope, key.into(), dir_name_str.into());
}
