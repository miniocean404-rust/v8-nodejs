use dashmap::DashMap; // 线程安全哈希表
use std::{
    future::Future,
    ptr::NonNull,
    sync::atomic::{AtomicU32, Ordering},
};
use v8::{Global, Local, Promise, PromiseResolver};

/// 异步任务调度器的 trait（接口）
pub trait AsyncTaskDispatcher: Default {
    type AsyncTaskResult; // 关联类型：任务结果

    /// 创建异步任务，返回 Promise
    fn create_async_task<'s, F>(
        &self,
        scope: &mut v8::HandleScope<'s>,
        async_block: F, // 异步闭包
    ) -> Local<'s, Promise>
    where
        F: Future<Output = Self::AsyncTaskResult> + Send + 'static; // F 必须是 Send + 'static

    /// 运行事件循环，处理所有完成的异步任务
    fn run_event_loop(
        &mut self,
        isolate: &mut v8::Isolate,
        scope: &mut v8::HandleScope<'_>,
    ) -> impl Future<Output = ()>; // 返回异步操作
}

/// 异步任务完成消息
#[derive(Debug)]
pub struct AsyncTaskMessage {
    pub task_id: TaskID,          // 任务 ID
    pub payload: AsyncTaskResult, // 任务结果
}

/// 任务结果枚举
#[derive(Debug)]
#[repr(u8)]
pub enum AsyncTaskResult {
    Resolve(AsyncTaskValue), // 成功
    Reject(AsyncTaskValue),  // 失败
}

/// 内部异步任务结构
struct AsyncTask {
    promise_resolver: NonNull<PromiseResolver>, // Promise 解析器的非空指针
}

/// 异步任务的值类型
#[derive(Debug)]
pub enum AsyncTaskValue {
    String(Vec<u8>), // 字符串（字节向量）
    Number(i32),     // 数字
    Undefined,       // undefined
}

pub(crate) type TaskID = u32; // 任务 ID 类型别名

/// Tokio 异步任务管理器 - 使用 Tokio 运行时管理异步任务
pub struct TokioAsyncTaskManager {
    tasks: DashMap<TaskID, AsyncTask>, // 任务存储（ID -> 任务）
    channel_sender: tokio::sync::mpsc::Sender<AsyncTaskMessage>, // 通道发送端
    channel_receiver: tokio::sync::mpsc::Receiver<AsyncTaskMessage>, // 通道接收端
}

impl TokioAsyncTaskManager {
    /// 创建新的 TokioAsyncTaskManager
    pub fn new() -> Self {
        let (sender, receiver) = tokio::sync::mpsc::channel(100); // 创建容量为 100 的通道
        TokioAsyncTaskManager {
            tasks: DashMap::new(), // 初始化空 HashMap
            channel_sender: sender,
            channel_receiver: receiver,
        }
    }
}

/// 生成唯一的任务 ID（原子操作）
pub(crate) fn generate_task_id() -> TaskID {
    static NEXT_ID: AtomicU32 = AtomicU32::new(0); // 原子计数器，初始为 0
    NEXT_ID.fetch_add(1, Ordering::Relaxed) // 自增并返回旧值
}

/// 从 V8 作用域创建异步任务
pub(crate) fn create_async_task_from_scope<'s, F>(
    scope: &mut v8::HandleScope<'s>,
    async_block: F,
) -> Local<'s, Promise>
where
    F: Future<Output = AsyncTaskResult> + Send + 'static,
{
    let value_ptr = scope.get_data(0) as *mut TokioAsyncTaskManager; // 从作用域获取管理器指针
    unsafe { &*value_ptr }.create_async_task(scope, async_block) // 调用管理器创建任务
}

impl Default for TokioAsyncTaskManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AsyncTaskDispatcher for TokioAsyncTaskManager {
    type AsyncTaskResult = AsyncTaskResult;

    /// 创建异步任务，将任务加入循环队列，返回 Promise
    fn create_async_task<'s, F>(
        &self,
        scope: &mut v8::HandleScope<'s>,
        async_block: F,
    ) -> Local<'s, Promise>
    where
        F: Future<Output = Self::AsyncTaskResult> + Send + 'static,
    {
        let promise_resolver = v8::PromiseResolver::new(scope).unwrap(); // 创建 Promise 解析器
        let promise = promise_resolver.get_promise(scope); // 从解析器获取 Promise
        let promise_resolver = Global::new(scope, promise_resolver); // 包装成 Global（可跨作用域）

        let task_id = generate_task_id(); // 生成唯一任务 ID

        // 将任务存储到 DashMap
        self.tasks.insert(
            task_id,
            AsyncTask {
                promise_resolver: promise_resolver.into_raw(), // 转换为原始指针
            },
        );

        // 生成 Tokio 异步任务
        tokio::spawn({
            let channel_sender = self.channel_sender.clone(); // 克隆通道发送端
            async move {
                // 等待异步块完成
                let task_value = async_block.await;
                let task_message = AsyncTaskMessage {
                    task_id,
                    payload: task_value,
                };
                channel_sender.send(task_message).await.unwrap(); // 通过通道发送结果
            }
        });

        promise // 返回 Promise
    }

    /// 运行事件循环，监听任务完成并 resolve/reject Promise
    async fn run_event_loop(&mut self, isolate: &mut v8::Isolate, scope: &mut v8::HandleScope<'_>) {
        while let Some(message) = self.channel_receiver.recv().await {
            // 接收任务完成消息, 从存储中移除任务
            if let Some((_, task)) = self.tasks.remove(&message.task_id) {
                // 还原 Promise 解析器
                let promise_resolver = unsafe { Global::from_raw(isolate, task.promise_resolver) };

                // 根据结果类型处理 Promise
                match message.payload {
                    // 成功
                    AsyncTaskResult::Resolve(task_value) => {
                        let v8_value = task_value.into_v8(scope); // 转换为 V8 值
                        promise_resolver.open(scope).resolve(scope, v8_value); // Resolve Promise
                    }
                    // 失败
                    AsyncTaskResult::Reject(task_value) => {
                        let v8_value = task_value.into_v8(scope);
                        promise_resolver.open(scope).reject(scope, v8_value); // Reject Promise
                    }
                }

                // perform_microtask_checkpoint: 强制让 V8 清空微任务队列，立即执行所有 pending 的 then/catch/queueMicrotask 这样相关的回调
                isolate.perform_microtask_checkpoint();
            }
        }
    }
}

impl AsyncTaskValue {
    /// 转换 AsyncTaskValue 为 V8 值
    pub fn into_v8<'s>(self, scope: &mut v8::HandleScope<'s>) -> v8::Local<'s, v8::Value> {
        match self {
            AsyncTaskValue::String(value) => {
                // 如果是字符串
                v8::String::new(scope, std::str::from_utf8(&value).unwrap_or_default()) // 转换为 V8 字符串
                    .unwrap()
                    .into()
            }
            AsyncTaskValue::Number(value) => v8::Number::new(scope, value as f64).into(), // 转换为 V8 数字
            AsyncTaskValue::Undefined => v8::undefined(scope).into(), // 转换为 undefined
        }
    }
}
