use std::{
    collections::BTreeMap, // 有序键值对映射
    fs::{self},            // 文件系统操作
    path::{Path, PathBuf}, // 路径操作
};

use v8::CallbackScope;

use crate::builtin::fs::create_fs; // 文件系统模块

/// 模块加载器 - 管理 JS 模块的加载、编译、缓存和依赖解析
pub struct ModuleLoader {
    // 映射模块的唯一标识哈希值到其绝对路径
    // 用于在模块回调中快速查询模块信息
    id_to_path_map: BTreeMap<i32, PathBuf>,

    // 模块缓存 - 根据绝对路径缓存已编译的模块
    // 使用 v8::Global 以便在不同作用域中存储模块
    module_cache: BTreeMap<PathBuf, v8::Global<v8::Module>>,

    // 内置模块存储 - 按名称缓存内置模块（如 "fs"）
    builtin_modules: BTreeMap<String, v8::Global<v8::Module>>,
}

impl ModuleLoader {
    /// 初始化 ModuleLoader，将 ModuleLoader 注入到 V8 隔离区的 1 位置的插槽中
    ///
    /// # 参数
    /// - `isolate`: V8 隔离区
    ///
    /// # 返回
    /// 返回一个静态可变引用（使用不安全代码）
    pub fn init_and_inject(isolate: &mut v8::Isolate) -> &'static mut ModuleLoader {
        // Box::into_raw 获取原始指针，手动管理内存，编译器不会自动管理
        let module_loader = Box::into_raw(Box::new(Self {
            id_to_path_map: BTreeMap::new(),
            module_cache: BTreeMap::new(),
            builtin_modules: BTreeMap::new(),
        }));

        // set_data() 允许你将任意数据与 V8 Isolate 关联起来，这些数据可以在后续的回调函数、JavaScript 执行过程中访问
        isolate.set_data(1, module_loader as *mut _);

        unsafe { &mut *module_loader }
    }

    /// 编译脚本代码为 V8 模块
    ///
    /// # 参数
    /// - `scope`: V8 作用域
    /// - `code`: JS 源代码
    /// - `resource_name_str`: 资源名称（用于错误信息和调试）
    ///
    /// # 返回
    /// 返回编译后的模块和其唯一标识哈希值的元组
    fn compile_script_module<'s>(
        scope: &mut v8::HandleScope<'s>,
        code: &str,          // JS 源代码
        resource_path: &str, // 资源路径
    ) -> Option<(v8::Local<'s, v8::Module>, i32)> {
        // 创建源代码字符串
        let source = v8::String::new(scope, code)?;
        // 文件 URL 前缀
        let file_url = v8::String::new(scope, "file://").unwrap();
        // 文件路径
        let resource_path = v8::String::new(scope, resource_path)?.into();

        // 创建主机定义的选项数组, PrimitiveArray: 原始数组, 创建大小为 1 的原始数组
        let defined_meta_options = v8::PrimitiveArray::new(scope, 1);
        // 第 0 个元素设为 "file://"
        defined_meta_options.set(scope, 0, file_url.into());

        // 创建脚本来源信息
        let script_origin = v8::ScriptOrigin::new(
            scope,
            resource_path,                     // 文件路径
            0,                                 // 行偏移
            0,                                 // 列偏移
            false, // 是否是共享代码, 非信任脚本限制跨域访问, true 允许跨域访问此脚本的错误信息和堆栈跟踪
            0, // 脚本 ID, 用于调试和性能分析, 调试器识别: 帮助开发工具识别不同的脚本、性能分析: 在性能分析器中区分脚本、缓存机制: V8 内部可能用于缓存编译结果
            None, // sourcemap URL
            false, // 是否是 WASM
            false, // 是否是 wasm
            true, // 是否是 esm 模块
            Some(defined_meta_options.into()), // 向 V8 传递自定义元数据、帮助模块加载器理解如何解析导入、定义脚本的执行权限、传递宿主环境特定的数据
        );

        let mut source = v8::script_compiler::Source::new(source, Some(&script_origin)); // 创建 js 代码源对象

        // 编译为 ES6 模块
        v8::script_compiler::compile_module(scope, &mut source).map(|module| {
            let hash_id: i32 = module.get_identity_hash().into(); // 获取模块唯一哈希 ID
            (module, hash_id) // 返回模块和 ID
        })
    }

    /// 核心函数：获取或编译模块（使用缓存或按需编译）
    ///
    /// # 参数
    /// - `scope`: V8 作用域
    /// - `absolute_path`: 模块的绝对路径
    ///
    /// # 返回
    /// 返回本地作用域中的模块引用
    fn get_or_compile_module<'s>(
        &mut self,
        scope: &mut v8::HandleScope<'s>,
        absolute_path: &Path, // 绝对路径
    ) -> Option<v8::Local<'s, v8::Module>> {
        let absolute_path_buf = absolute_path.to_path_buf();

        // 检查模块是否已在缓存中
        if let Some(global_module) = self.module_cache.get(&absolute_path_buf) {
            // 从全局缓存返回本地引用, v8::Local::new 用于在不同的 V8 作用域(Scope) 之间传递 JavaScript 值
            return Some(v8::Local::new(scope, global_module));
        }

        // 模块不在缓存中，读取并编译
        let content = match fs::read_to_string(absolute_path) {
            Ok(content) => content,
            Err(e) => {
                eprintln!("Error reading file '{}': {}", absolute_path.display(), e); // 错误日志
                return None;
            }
        };

        let resource_path = absolute_path.to_str().unwrap_or("unknown.js");

        if let Some((module, hash_id)) =
            // 编译模块
            Self::compile_script_module(scope, &content, resource_path)
        {
            // 缓存 ID 到路径的映射（在依赖解析时需要）
            self.id_to_path_map
                .insert(hash_id, absolute_path_buf.clone());

            // 实例化模块（重要步骤）
            if module
                .instantiate_module(scope, resolve_module_callback) // 实例化模块，指定依赖解析函数
                .is_none()
            {
                eprintln!("错误: 实例化模块失败: {}", absolute_path.display());
                return None;
            }

            // v8::Global 用于在 rust 中持有对 JavaScript 对象的持久引用, 以便于在不同作用域中存储模块
            let global_module = v8::Global::new(scope, module);
            // 缓存模块
            self.module_cache.insert(absolute_path_buf, global_module);

            Some(module)
        } else {
            eprintln!("错误: 编译模块失败: {}", absolute_path.display());
            None // 编译失败
        }
    }

    /// 创建入口模块
    ///
    /// # 参数
    /// - `scope`: V8 作用域
    /// - `path_str`: 路径字符串（可以是相对路径或绝对路径）
    pub fn create_first_module<'s>(
        &mut self,
        scope: &mut v8::HandleScope<'s>,
        path_str: &str, // 路径字符串
    ) -> Option<v8::Local<'s, v8::Module>> {
        // 规范化路径为绝对路径
        let path = Path::new(path_str); // 创建路径对象
        let absolute_path = match fs::canonicalize(path) {
            Ok(p) => p, // 成功规范化
            Err(e) => {
                eprintln!("错误: 规范化入口点路径 '{}' 失败: {}", path_str, e);
                return None;
            }
        };

        self.get_or_compile_module(scope, &absolute_path) // 获取或编译模块
    }

    /// 初始化内置 API, 例如 fs
    ///
    /// 创建一个合成的 V8 API，由 Rust 代码实现
    fn init_builtin_module(
        scope: &mut v8::HandleScope<'_>,
        specifier_str: &str, // 模块名称
    ) -> v8::Global<v8::Module> {
        let fs_module_name = v8::String::new(scope, specifier_str).unwrap(); // 模块名称字符串
        let export_names = &[v8::String::new(scope, "default").unwrap()]; // 导出名称

        // 创建模块（由 Rust 代码实现的模块）
        let module = v8::Module::create_synthetic_module(
            scope,
            fs_module_name, // 模块名
            export_names,   // 导出项名
            |context: v8::Local<'_, v8::Context>, module: v8::Local<'_, v8::Module>| {
                // 初始化回调
                let mut scope = unsafe { CallbackScope::new(context) }; // 从上下文创建作用域
                let default_export_name = v8::String::new(&mut scope, "default").unwrap(); // "default" 字符串

                let fs_module = create_fs(&mut scope); // 创建文件系统模块
                let value = fs_module.new_instance(&mut scope).unwrap(); // 创建模块实例

                // 设置 default 导出
                let result = module.set_synthetic_module_export(
                    &mut scope,
                    default_export_name,
                    value.into(),
                );

                result.map(|result| v8::Boolean::new(&mut scope, result).into())
                // 返回布尔值
            },
        );

        v8::Global::new(scope, module) // 包装为 Global
    }

    /// 加载内置模块（如 "fs"）
    ///
    /// 如果模块未缓存则初始化，否则从缓存获取
    pub fn load_builtin_module<'s>(
        &mut self,
        scope: &mut v8::HandleScope<'s>,
        specifier_str: &str, // import 导入的模块名称
    ) -> Option<v8::Local<'s, v8::Module>> {
        let module = self
            .builtin_modules
            .entry(specifier_str.to_string()) // 从字典获取或插入
            .or_insert_with(|| Self::init_builtin_module(scope, specifier_str)); // 不存在则初始化

        Some(v8::Local::new(scope, &*module))
    }
}

/// 模块依赖解析回调函数
///
/// 当 JavaScript 模块中包含 import/export 语句时，V8 会调用此函数来解析依赖
pub fn resolve_module_callback<'s>(
    context: v8::Local<'s, v8::Context>,
    specifier: v8::Local<'s, v8::String>, // import 其他导入的模块路径
    _import_assertions: v8::Local<'s, v8::FixedArray>, // import 断言（未使用）
    referrer: v8::Local<'s, v8::Module>,  // 当前的文件引用
) -> Option<v8::Local<'s, v8::Module>> {
    let mut scope = unsafe { v8::CallbackScope::new(context) }; // 创建作用域

    let state_ptr = scope.get_data(1); // 获取 ModuleLoader 指针
    if state_ptr.is_null() {
        eprintln!("错误: 在 resolve_module_callback 中的 ModuleLoader state 为空 ");
        return None;
    }
    let module_loader = unsafe { &mut *(state_ptr as *mut ModuleLoader) }; // 转换为引用
    let specifier_str = specifier.to_rust_string_lossy(&mut scope); // 模块路径字符串

    // 判断是否为内置模块（不含路径分隔符）, 如果是内置模块则加载内置模块
    if !specifier_str.starts_with('.') && !specifier_str.starts_with('/') {
        return module_loader.load_builtin_module(&mut scope, &specifier_str);
    }

    let referrer_id: i32 = referrer.get_identity_hash().into(); // 获取导入模块的 hash
    let referrer_path = module_loader.id_to_path_map.get(&referrer_id)?; // 查询导入者路径

    let referrer_dir = referrer_path.parent().unwrap_or(Path::new("")); // 导入者目录
    let resolved_path_buf = referrer_dir.join(&specifier_str); // 解析路径

    // 支持的文件扩展名
    const EXTENSIONS: [&str; 2] = ["", "js"]; // 尝试原文件名和 .js 扩展

    EXTENSIONS
        .iter()
        .find_map(|extension| {
            // 逐个尝试扩展名
            let mut resolved_path_with_extension = resolved_path_buf.clone();
            resolved_path_with_extension.set_extension(extension); // 添加扩展名
            fs::canonicalize(&resolved_path_with_extension).ok() // 规范化路径
        })
        .and_then(|path| module_loader.get_or_compile_module(&mut scope, &path))
}

/// import.meta 对象初始化回调函数
///
/// 当 JavaScript 代码访问 import.meta 时，V8 会调用此函数来初始化该对象
/// 这里我们将模块的目录名称设为 import.meta.dirname
///
/// extern "C" 是 Rust 中的外部函数接口 (FFI - Foreign Function Interface) 声明，用于与 C 语言 ABI (Application Binary Interface) 兼容、及防止名称被修改导致编译后 FFI 调用无法找到函数
pub extern "C" fn host_initialize_import_meta_object_callback(
    context: v8::Local<'_, v8::Context>, // V8 执行上下文
    module: v8::Local<'_, v8::Module>,   // 当前正在加载的模块
    meta: v8::Local<'_, v8::Object>,     // import.meta 对象的引用
) {
    // 根据上下文创建作用域
    let mut scope = unsafe { v8::CallbackScope::new(context) };

    // 获取 ModuleLoader
    let state_ptr = scope.get_data(1);
    if state_ptr.is_null() {
        eprintln!("错误: 在 resolve_module_callback 中的 ModuleLoader 为空 ");
        return;
    }
    let module_loader = unsafe { &mut *(state_ptr as *mut ModuleLoader) };

    // 模块 hash
    let module_id: i32 = module.get_identity_hash().into();
    // 根据模块 hash 查找文件夹
    let dir_name = module_loader
        .id_to_path_map
        .get(&module_id) // 根据模块 hash 查找文件路径
        .unwrap()
        .parent() // 获取目录
        .unwrap();

    // 在 import.meta 上设置 dirname 属性
    let key = v8::String::new(&mut scope, "dirname").unwrap();
    let dir_name_str = v8::String::new(&mut scope, dir_name.to_str().unwrap()).unwrap();

    // 设置 meta.dirname
    meta.set(&mut scope, key.into(), dir_name_str.into());
}
