use std::os::fd::{FromRawFd, IntoRawFd};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use v8::{Global, ObjectTemplate};

use crate::async_task::{create_async_task_from_scope, AsyncTaskResult, AsyncTaskValue};

fn extract_internal_field_file_handler(
    scope: &mut v8::HandleScope<'_>,
    args: &v8::FunctionCallbackArguments,
) -> File {
    let caller = args.this();
    let file_handler = caller
        .get_internal_field(scope, 0)
        .unwrap()
        .cast::<v8::External>();

    unsafe { File::from_raw(file_handler.value() as *mut _) }
}

fn seek_file_pos(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let file_handler = extract_internal_field_file_handler(scope, &args);
    let pos = args.get(0).to_uint32(scope).map(|v| v.value()).unwrap_or(0);

    let promise = create_async_task_from_scope(scope, async move {
        let result = file_handler.seek(pos as u64).await;
        match result {
            Ok(_) => AsyncTaskResult::Resolve(AsyncTaskValue::Undefined),
            Err(e) => AsyncTaskResult::Reject(AsyncTaskValue::String(e.to_string().into_bytes())),
        }
    });

    return_value.set(promise.into());
}

fn read_file_content(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let file_handler = extract_internal_field_file_handler(scope, &args);

    let promise = create_async_task_from_scope(scope, async move {
        let result = file_handler.read_to_end().await;
        match result {
            Ok(content) => AsyncTaskResult::Resolve(AsyncTaskValue::String(content)),
            Err(e) => AsyncTaskResult::Reject(AsyncTaskValue::String(e.to_string().into_bytes())),
        }
    });

    return_value.set(promise.into());
}

fn write_file(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let file_handler = extract_internal_field_file_handler(scope, &args);

    let new_content = args
        .get(0)
        .try_cast::<v8::String>()
        .ok()
        .map(|v| v.to_rust_string_lossy(scope));

    let Some(new_content) = new_content else {
        let error = v8::String::new(scope, "The \"path\" argument must be string").unwrap();
        scope.throw_exception(error.into());
        return;
    };

    let promise = create_async_task_from_scope(scope, async move {
        let new_content = new_content.into_bytes();
        let result = file_handler.write(&new_content).await;
        match result {
            Ok(_) => AsyncTaskResult::Resolve(AsyncTaskValue::Number(new_content.len() as i32)),
            Err(e) => AsyncTaskResult::Reject(AsyncTaskValue::String(e.to_string().into_bytes())),
        }
    });

    return_value.set(promise.into());
}

fn create_file_handler_template(scope: &mut v8::HandleScope<()>) -> v8::Global<ObjectTemplate> {
    let template = v8::ObjectTemplate::new(scope);
    template.set_internal_field_count(1);

    let file_content_fn_name = v8::String::new(scope, "content").unwrap();
    let file_content_fn = v8::FunctionTemplate::new(scope, read_file_content);
    template.set(file_content_fn_name.into(), file_content_fn.into());

    let file_write_fn_name = v8::String::new(scope, "write").unwrap();
    let file_write_fn = v8::FunctionTemplate::new(scope, write_file);

    template.set(file_write_fn_name.into(), file_write_fn.into());

    let file_seek_fn_name = v8::String::new(scope, "seek").unwrap();
    let file_seek_fn = v8::FunctionTemplate::new(scope, seek_file_pos);
    template.set(file_seek_fn_name.into(), file_seek_fn.into());

    Global::new(scope, template)
}

struct File {
    file_handler_ptr: *mut tokio::fs::File,
}
unsafe impl Send for File {}
unsafe impl Sync for File {}

impl File {
    fn new(fd: i32) -> Self {
        let file = unsafe { tokio::fs::File::from_raw_fd(fd) };
        let file_handler_ptr = Box::into_raw(Box::new(file));
        Self { file_handler_ptr }
    }

    async fn read_to_end(&self) -> Result<Vec<u8>, std::io::Error> {
        let file = unsafe { &mut *self.file_handler_ptr };
        let mut buf = Vec::new();
        file.seek(tokio::io::SeekFrom::Start(0)).await?;
        file.read_to_end(&mut buf).await?;
        Ok(buf)
    }

    async fn seek(&self, pos: u64) -> Result<(), std::io::Error> {
        let file = unsafe { &mut *self.file_handler_ptr };
        file.seek(tokio::io::SeekFrom::Start(pos)).await?;
        Ok(())
    }

    async fn write(&self, data: &[u8]) -> Result<(), std::io::Error> {
        let file = unsafe { &mut *self.file_handler_ptr };
        file.write_all(data).await?;
        file.flush().await?;
        Ok(())
    }

    // 转换成  v8::External 对象
    fn to_v8_external<'s>(self, scope: &mut v8::HandleScope<'s>) -> v8::Local<'s, v8::External> {
        let ptr = self.into_raw() as *mut _;
        v8::External::new(scope, ptr)
    }

    fn into_raw(self) -> *mut () {
        self.file_handler_ptr as *mut _
    }

    unsafe fn from_raw(ptr: *mut ()) -> Self {
        Self {
            file_handler_ptr: ptr as *mut _,
        }
    }
}

impl From<v8::External> for &mut File {
    fn from(external: v8::External) -> Self {
        let ptr = external.value() as *mut File;
        unsafe { &mut *ptr }
    }
}

fn extract_external_from_args<'a, T>(args: &v8::FunctionCallbackArguments) -> &'a mut T {
    let external = args.data().cast::<v8::External>();
    unsafe { &mut *(external.value() as *mut T) }
}

fn open_file_handler(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let path = args.get(0);
    let path_str = path.to_rust_string_lossy(scope);

    let instance = extract_external_from_args::<ObjectTemplate>(&args)
        .new_instance(scope)
        .expect("can't instance object");

    fn promise_mapper(
        scope: &mut v8::HandleScope,
        args: v8::FunctionCallbackArguments,
        mut return_value: v8::ReturnValue,
    ) {
        let fd = args.get(0).to_int32(scope).unwrap().value();
        let instance = args.data().cast::<v8::Object>();
        let file_handler = File::new(fd).to_v8_external(scope);
        instance.set_internal_field(0, file_handler.into());
        return_value.set(instance.into());
    }

    let promise_mapper = v8::Function::builder(promise_mapper)
        .data(instance.into())
        .build(scope)
        .unwrap();

    let promise = create_async_task_from_scope(scope, async move {
        let result = tokio::fs::File::options()
            .write(true)
            .read(true)
            .create(true)
            .truncate(false)
            .open(path_str)
            .await;

        match result {
            Ok(file) => {
                let fd = file.into_std().await.into_raw_fd();
                AsyncTaskResult::Resolve(AsyncTaskValue::Number(fd))
            }
            Err(e) => AsyncTaskResult::Reject(AsyncTaskValue::String(e.to_string().into_bytes())),
        }
    });

    let promise = promise.then(scope, promise_mapper).unwrap();
    return_value.set(promise.into());
}

pub fn create_fs<'s>(scope: &mut v8::HandleScope<'s, ()>) -> v8::Local<'s, v8::ObjectTemplate> {
    let fs = v8::ObjectTemplate::new(scope);

    let file_handler_template = create_file_handler_template(scope);
    let file_handler_template_ptr = file_handler_template.into_raw();
    let file_handler_template =
        v8::External::new(scope, file_handler_template_ptr.as_ptr() as *mut _);

    fs.set(
        v8::String::new(scope, "openFile").unwrap().into(),
        v8::FunctionTemplate::builder(open_file_handler)
            .data(file_handler_template.into())
            .build(scope)
            .into(),
    );

    fs
}
