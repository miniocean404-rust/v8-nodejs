use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

use v8::Module;

pub struct ModuleLoader {
    // Maps module identity hash to its absolute path
    id_to_path_map: BTreeMap<i32, PathBuf>,
    // Caches compiled modules by absolute path
    // Use v8::Global to store modules across different scopes
    module_cache: BTreeMap<PathBuf, v8::Global<v8::Module>>,
}

impl ModuleLoader {
    pub fn inject_into_isolate(isolate: &mut v8::Isolate) -> &'static mut ModuleLoader {
        let module_loader = Box::into_raw(Box::new(Self {
            id_to_path_map: BTreeMap::new(),
            module_cache: BTreeMap::new(),
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

        let resource_name = v8::String::new(scope, resource_name_str)?.into();
        let script_origin = v8::ScriptOrigin::new(
            scope,
            resource_name,
            0,     // line_offset
            0,     // column_offset
            false, // is_cross_origin
            0,     // script_id
            None,  // source_map_url
            false, // is_opaque
            false, // is_wasm
            true,  // is_module
            None,  // host_defined_options
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

    // Handle built-in modules (specifiers without path structure)
    if !specifier_str.starts_with('.') && !specifier_str.starts_with('/') {
        // TODO: Implement built-in module loading logic here
        // For now, return None or handle specific known built-ins
        println!(
            "Attempted to import non-path specifier (built-in?): {}",
            specifier_str
        );
        return None;
    }

    // Get the identity hash of the referrer module
    let referrer_id: i32 = referrer.get_identity_hash().into();

    // Find the absolute path of the referrer module
    let referrer_path = match module_loader.id_to_path_map.get(&referrer_id) {
        Some(path) => path.clone(),
        None => {
            // This might happen if the referrer module itself failed compilation/instantiation
            // or wasn't processed by our get_or_compile_module.
            // We need the referrer's path to resolve relative specifiers.
            eprintln!(
                "Error: Could not find path for referrer module ID: {} (Resource Name: '{}'). Cannot resolve relative path '{}'.",
                referrer_id, specifier_str, specifier_str
            );
            return None; // Cannot resolve relative paths without referrer's path
        }
    };

    // Resolve the specifier relative to the referrer's directory
    let referrer_dir = referrer_path.parent().unwrap_or_else(|| Path::new("")); // Use empty path if no parent (e.g., root file)
    let resolved_path_buf = referrer_dir.join(&specifier_str);

    // Canonicalize the resolved path to handle '..' and ensure it's absolute
    match fs::canonicalize(&resolved_path_buf) {
        Ok(absolute_path) => {
            // Use the core logic to get or compile the module. This will now hit the cache if available.
            module_loader.get_or_compile_module(&mut scope, &absolute_path)
        }
        Err(e) => {
            eprintln!(
                "Error resolving or canonicalizing path '{}' (from '{}') relative to '{}': {}",
                specifier_str,
                resolved_path_buf.display(),
                referrer_path.display(),
                e
            );
            None
        }
    }
}
