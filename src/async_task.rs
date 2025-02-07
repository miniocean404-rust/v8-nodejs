use std::{
    future::Future,
    ptr::NonNull,
    sync::atomic::{AtomicU32, Ordering},
};

use dashmap::DashMap;
use v8::{Global, Local, Promise, PromiseResolver};

pub trait AsyncTaskDispatcher: Default {
    type AsyncTaskResult;

    fn create_async_task<'s, F>(
        &self,
        scope: &mut v8::HandleScope<'s>,
        async_block: F,
    ) -> Local<'s, Promise>
    where
        F: Future<Output = Self::AsyncTaskResult> + Send + 'static;

    fn run_event_loop(
        &mut self,
        isolate: &mut v8::Isolate,
        scope: &mut v8::HandleScope<'_>,
    ) -> impl Future<Output = ()>;
}

#[derive(Debug)]
pub(crate) struct AsyncTaskMessage {
    pub task_id: TaskID,
    pub payload: AsyncTaskResult,
}

#[derive(Debug)]
#[repr(u8)]
pub enum AsyncTaskResult {
    Resolve(AsyncTaskValue),
    Reject(AsyncTaskValue),
}

struct AsyncTask {
    promise_resolver: NonNull<PromiseResolver>,
}

#[derive(Debug)]
pub enum AsyncTaskValue {
    String(Vec<u8>),
    Number(i32),
    Undefined,
}

pub(crate) type TaskID = u32;

pub struct TokioAsyncTaskManager {
    tasks: DashMap<TaskID, AsyncTask>,
    channel_sender: tokio::sync::mpsc::Sender<AsyncTaskMessage>,
    channel_receiver: tokio::sync::mpsc::Receiver<AsyncTaskMessage>,
}

impl TokioAsyncTaskManager {
    pub fn new() -> Self {
        let (sender, receiver) = tokio::sync::mpsc::channel(100);
        TokioAsyncTaskManager {
            tasks: DashMap::new(),
            channel_sender: sender,
            channel_receiver: receiver,
        }
    }
}
pub(crate) fn generate_task_id() -> TaskID {
    static NEXT_ID: AtomicU32 = AtomicU32::new(0);
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

pub(crate) fn create_async_task_from_scope<'s, F>(
    scope: &mut v8::HandleScope<'s>,
    async_block: F,
) -> Local<'s, Promise>
where
    F: Future<Output = AsyncTaskResult> + Send + 'static,
{
    let value_ptr = scope.get_data(0) as *mut TokioAsyncTaskManager;
    unsafe { &*value_ptr }.create_async_task(scope, async_block)
}

impl Default for TokioAsyncTaskManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AsyncTaskDispatcher for TokioAsyncTaskManager {
    type AsyncTaskResult = AsyncTaskResult;

    fn create_async_task<'s, F>(
        &self,
        scope: &mut v8::HandleScope<'s>,
        async_block: F,
    ) -> Local<'s, Promise>
    where
        F: Future<Output = Self::AsyncTaskResult> + Send + 'static,
    {
        let promise_resolver = v8::PromiseResolver::new(scope).unwrap();
        let promise = promise_resolver.get_promise(scope);
        let promise_resolver = Global::new(scope, promise_resolver);

        let task_id = generate_task_id();

        self.tasks.insert(
            task_id,
            AsyncTask {
                promise_resolver: promise_resolver.into_raw(),
            },
        );

        tokio::spawn({
            let channel_sender = self.channel_sender.clone();
            async move {
                let task_value = async_block.await;
                let task_message = AsyncTaskMessage {
                    task_id,
                    payload: task_value,
                };
                channel_sender.send(task_message).await.unwrap();
            }
        });

        promise
    }

    async fn run_event_loop(&mut self, isolate: &mut v8::Isolate, scope: &mut v8::HandleScope<'_>) {
        while let Some(message) = self.channel_receiver.recv().await {
            if let Some((_, task)) = self.tasks.remove(&message.task_id) {
                let promise_resolver = unsafe { Global::from_raw(isolate, task.promise_resolver) };

                match message.payload {
                    AsyncTaskResult::Resolve(task_value) => {
                        let v8_value = task_value.into_v8(scope);
                        promise_resolver.open(scope).resolve(scope, v8_value);
                    }
                    AsyncTaskResult::Reject(task_value) => {
                        let v8_value = task_value.into_v8(scope);
                        promise_resolver.open(scope).reject(scope, v8_value);
                    }
                }
            }
        }
    }
}

impl AsyncTaskValue {
    pub fn into_v8<'s>(self, scope: &mut v8::HandleScope<'s>) -> v8::Local<'s, v8::Value> {
        match self {
            AsyncTaskValue::String(value) => {
                v8::String::new(scope, std::str::from_utf8(&value).unwrap_or_default())
                    .unwrap()
                    .into()
            }
            AsyncTaskValue::Number(value) => v8::Number::new(scope, value as f64).into(),
            AsyncTaskValue::Undefined => v8::undefined(scope).into(),
        }
    }
}
