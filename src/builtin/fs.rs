use super::async_task; // 异步任务模块
use async_task::{create_async_task_from_scope, AsyncTaskResult, AsyncTaskValue}; // 异步任务工具
use std::os::fd::{FromRawFd, IntoRawFd}; // 文件描述符操作
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt}; // 异步 I/O 特性
use v8::{Global, ObjectTemplate};

/// Rust 文件处理器包装
///
/// 将 tokio::fs::File 包装为可跨线程的对象
struct File {
    file_handler_ptr: *mut tokio::fs::File, // 文件指针
}

unsafe impl Send for File {} // 允许在线程间发送
unsafe impl Sync for File {} // 允许多线程访问

impl File {
    /// 从文件描述符创建 File 对象
    fn new(fd: i32) -> Self {
        let file = unsafe { tokio::fs::File::from_raw_fd(fd) }; // 从 FD 创建 File
        let file_handler_ptr = Box::into_raw(Box::new(file)); // 分配到堆并获取指针
        Self { file_handler_ptr }
    }

    /// 异步读取文件的全部内容
    async fn read_to_end(&self) -> Result<Vec<u8>, std::io::Error> {
        let file = unsafe { &mut *self.file_handler_ptr }; // 解指针
        let mut buf = Vec::new(); // 创建缓冲区
        file.seek(tokio::io::SeekFrom::Start(0)).await?; // 寻址到开始
        file.read_to_end(&mut buf).await?; // 读取到缓冲区
        Ok(buf) // 返回缓冲区
    }

    /// 异步定位文件指针
    async fn seek(&self, pos: u64) -> Result<(), std::io::Error> {
        let file = unsafe { &mut *self.file_handler_ptr }; // 解指针
        file.seek(tokio::io::SeekFrom::Start(pos)).await?; // 寻址到指定位置
        Ok(())
    }

    /// 异步写入数据到文件
    async fn write(&self, data: &[u8]) -> Result<(), std::io::Error> {
        let file = unsafe { &mut *self.file_handler_ptr }; // 解指针
        file.write_all(data).await?; // 写入全部数据
        file.flush().await?; // 刷新缓冲区
        Ok(())
    }

    /// 转换为 V8 External 对象
    ///
    /// 这允许我们将 Rust 指针存储在 V8 值中
    fn to_v8_external<'s>(self, scope: &mut v8::HandleScope<'s>) -> v8::Local<'s, v8::External> {
        let ptr = self.into_raw() as *mut _; // 获取指针
        v8::External::new(scope, ptr) // 创建 V8 External
    }

    /// 转换为原始指针（并放弃所有权）
    fn into_raw(self) -> *mut () {
        self.file_handler_ptr as *mut _
    }

    /// 从原始指针还原 File 对象
    unsafe fn from_raw(ptr: *mut ()) -> Self {
        Self {
            file_handler_ptr: ptr as *mut _,
        }
    }
}

/// 从 V8 External 转换为 Rust 引用
impl From<v8::External> for &mut File {
    fn from(external: v8::External) -> Self {
        let ptr = external.value() as *mut File;
        unsafe { &mut *ptr }
    }
}

/// 从 V8 函数回调中提取内部字段中存储的文件处理器
///
/// 文件处理器存储在 V8 对象的内部字段 0 中作为 External
fn extract_internal_field_file_handler(
    scope: &mut v8::HandleScope<'_>,
    args: &v8::FunctionCallbackArguments, // 函数参数
) -> File {
    let caller = args.this(); // 获取自定义函数 this 对象

    // v8::External: 包装成 JavaScript 可以处理的值，实现跨语言的对象引用。
    let file_handler = caller
        .get_internal_field(scope, 0) // 获取通过 set_internal_field(0, xxx) 存储在 v8 JavaScript 对象的第 0 个内部字段 (File 对象指针)
        .unwrap()
        .cast::<v8::External>();

    // 转换为 Rust 对象
    unsafe { File::from_raw(file_handler.value() as *mut _) }
}

/// 文件定位函数 - 将文件指针移动到指定位置
///
/// 返回一个 Promise，当操作完成时 resolve
fn seek_file_pos(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments, // 自定义函数参数获取
    mut return_value: v8::ReturnValue,   // 返回值
) {
    let file_handler = extract_internal_field_file_handler(scope, &args); // 提取文件处理器
    let pos = args.get(0).to_uint32(scope).map(|v| v.value()).unwrap_or(0); // 获取位置参数

    // 创建异步任务
    let promise = create_async_task_from_scope(scope, async move {
        let result = file_handler.seek(pos as u64).await; // 异步文件寻址
        match result {
            Ok(_) => AsyncTaskResult::Resolve(AsyncTaskValue::Undefined), // 成功返回 undefined
            Err(e) => AsyncTaskResult::Reject(AsyncTaskValue::String(e.to_string().into_bytes())), // 错误
        }
    });

    return_value.set(promise.into()); // 设置返回值为 Promise
}

/// 读取文件内容函数
///
/// 返回一个 Promise，当读取完成时 resolve，value 为文件内容（字节数组）
fn read_file_content(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let file_handler = extract_internal_field_file_handler(scope, &args); // 提取文件处理器

    // 创建异步任务
    let promise = create_async_task_from_scope(scope, async move {
        let result = file_handler.read_to_end().await; // 异步读取文件
        match result {
            Ok(content) => AsyncTaskResult::Resolve(AsyncTaskValue::String(content)), // 返回内容
            Err(e) => AsyncTaskResult::Reject(AsyncTaskValue::String(e.to_string().into_bytes())), // 错误
        }
    });

    return_value.set(promise.into()); // 设置返回值为 Promise
}

/// 写入文件函数
///
/// 返回一个 Promise，当写入完成时 resolve，value 为写入的字节数
fn write_file(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let file_handler = extract_internal_field_file_handler(scope, &args); // 提取文件处理器

    // 获取第一个参数并转换为字符串
    let new_content = args
        .get(0) // 获取参数
        .try_cast::<v8::String>()
        .ok()
        .map(|v| v.to_rust_string_lossy(scope));

    // 如果参数不是字符串则在 JS 端抛出异常
    let Some(new_content) = new_content else {
        let error = v8::String::new(scope, "The \"path\" 参数必须被设置为字符串").unwrap();
        scope.throw_exception(error.into());
        return;
    };

    // 创建异步任务
    let promise = create_async_task_from_scope(scope, async move {
        let new_content = new_content.into_bytes(); // 转换为字节
        let result = file_handler.write(&new_content).await; // 异步写入文件
        match result {
            Ok(_) => AsyncTaskResult::Resolve(AsyncTaskValue::Number(new_content.len() as i32)), // 返回写入字节数
            Err(e) => AsyncTaskResult::Reject(AsyncTaskValue::String(e.to_string().into_bytes())), // 错误
        }
    });

    return_value.set(promise.into()); // 设置返回值为 Promise
}

/// 创建 File 对象模板
///
/// 这个模板定义了 File 对象暴露给 JavaScript 的方法
fn create_file_handler_template(scope: &mut v8::HandleScope<()>) -> v8::Global<ObjectTemplate> {
    let template = v8::ObjectTemplate::new(scope); // 创建 File 对象
    template.set_internal_field_count(1); // 设置内部字段数为 1（存放 File 对象指针）

    // 添加 content 方法（读取文件内容）
    let file_content_fn_name = v8::String::new(scope, "content").unwrap();
    let file_content_fn = v8::FunctionTemplate::new(scope, read_file_content);
    template.set(file_content_fn_name.into(), file_content_fn.into());

    // 添加 write 方法（写入文件）
    let file_write_fn_name = v8::String::new(scope, "write").unwrap();
    let file_write_fn = v8::FunctionTemplate::new(scope, write_file);
    template.set(file_write_fn_name.into(), file_write_fn.into());

    // 添加 seek 方法（文件定位）
    let file_seek_fn_name = v8::String::new(scope, "seek").unwrap();
    let file_seek_fn = v8::FunctionTemplate::new(scope, seek_file_pos);
    template.set(file_seek_fn_name.into(), file_seek_fn.into());

    Global::new(scope, template) // 包装为 Global
}

/// 从函数参数中提取 External 数据
///
/// 用于提取传递给函数的数据指针
fn extract_external_from_args<'a, T>(args: &v8::FunctionCallbackArguments) -> &'a mut T {
    let external = args.data().cast::<v8::External>(); // 强制转换
    unsafe { &mut *(external.value() as *mut T) } // 解指针
}

/// openFile 函数
///
/// 返回一个 Promise，当文件打开成功时 resolve 为文件对象实例
fn open_file_handler(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let path = args.get(0); // 获取文件路径参数
    let path_str = path.to_rust_string_lossy(scope); // 转换为字符串

    /// Promise 映射函数 - 在异步任务完成时调用
    ///
    /// 获取文件描述符，创建文件处理器，存储到对象的内部字段
    fn promise_mapper(
        scope: &mut v8::HandleScope,
        args: v8::FunctionCallbackArguments,
        mut return_value: v8::ReturnValue,
    ) {
        let fd = args.get(0).to_int32(scope).unwrap().value(); // 获取文件描述符
        let instance = args.data().cast::<v8::Object>(); // 获取 File 对象实例
        let file_handler = File::new(fd).to_v8_external(scope); // 创建文件处理器
        instance.set_internal_field(0, file_handler.into()); // 存储到内部字段
        return_value.set(instance.into()); // 返回 File 对象
    }

    // 提取 file_handler_template 模板并创建实例
    let instance = extract_external_from_args::<ObjectTemplate>(&args)
        .new_instance(scope)
        .expect("不能实例化对象");

    // 构建 Promise 映射函数
    let promise_mapper = v8::Function::builder(promise_mapper)
        .data(instance.into()) // 传递实例
        .build(scope) // 构建
        .unwrap();

    // 创建异步任务来打开文件
    let promise = create_async_task_from_scope(scope, async move {
        let result = tokio::fs::File::options() // 配置文件打开选项
            .write(true) // 可写
            .read(true) // 可读
            .create(true) // 不存在则创建
            .truncate(false) // 不截断
            .open(path_str) // 打开文件
            .await;

        match result {
            Ok(file) => {
                let fd = file.into_std().await.into_raw_fd(); // 获取文件描述符
                AsyncTaskResult::Resolve(AsyncTaskValue::Number(fd)) // 返回 FD
            }
            Err(e) => AsyncTaskResult::Reject(AsyncTaskValue::String(e.to_string().into_bytes())), // 错误
        }
    });

    // 链接 Promise：第一个 Promise resolve 后调用 mapper 获得文件对象
    let promise = promise.then(scope, promise_mapper).unwrap();
    return_value.set(promise.into()); // 返回最终 Promise
}

/// 创建文件系统模块
///
/// 返回一个对象模板，暴露 openFile 方法给 JavaScript
pub fn create_fs<'s>(scope: &mut v8::HandleScope<'s, ()>) -> v8::Local<'s, v8::ObjectTemplate> {
    let fs: v8::Local<'_, ObjectTemplate> = v8::ObjectTemplate::new(scope); // 创建 fs 对象(是一个模板)

    let file_handler_template = create_file_handler_template(scope); // 创建 File 对象(是一个模板)
    let file_handler_template_ptr = file_handler_template.into_raw(); // 获取原始指针
    let file_handler_template =
        v8::External::new(scope, file_handler_template_ptr.as_ptr() as *mut _); // v8::External 允许将 C++ 对象的指针包装成 JavaScript 值，使其能在 JavaScript 环境中传递和存储。

    // 添加 openFile 方法
    fs.set(
        v8::String::new(scope, "openFile").unwrap().into(), // "openFile" 方法
        v8::FunctionTemplate::builder(open_file_handler) // 创建函数
            .data(file_handler_template.into()) // 将模板作为 data 传入, 可在 buider 回调中使用 args.data() 重新获取
            .build(scope)
            .into(),
    );

    fs
}
