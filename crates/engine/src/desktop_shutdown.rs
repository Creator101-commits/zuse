//! Own the desktop drain beyond the UI framework's short quit-hook deadline.
use std::{
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::{
    runtime::{Handle, Runtime},
    sync::oneshot,
    task::JoinHandle,
};

#[derive(Clone, Default)]
pub struct DesktopShutdown(Arc<Mutex<Option<JoinHandle<()>>>>);

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Complete,
    TimedOut,
    Failed,
}

impl DesktopShutdown {
    /// Dropping the UI's completion receiver never cancels the owned drain.
    pub fn start(
        &self,
        runtime: &Handle,
        drain: impl Future<Output = ()> + Send + 'static,
    ) -> oneshot::Receiver<()> {
        let (done, receiver) = oneshot::channel();
        let mut task = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if task.is_none() {
            *task = Some(runtime.spawn(async move {
                drain.await;
                let _ = done.send(());
            }));
        }
        receiver
    }

    /// Called after the GUI event loop returns. Never use Runtime's unbounded
    /// drop: blocking provider/file workers can otherwise strand a closed app.
    pub fn finish(
        self,
        runtime: Runtime,
        drain_timeout: Duration,
        worker_timeout: Duration,
    ) -> Outcome {
        let task = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let outcome = match task {
            None => Outcome::Complete,
            Some(mut task) => runtime.block_on(async {
                match tokio::time::timeout(drain_timeout, &mut task).await {
                    Ok(Ok(())) => Outcome::Complete,
                    Ok(Err(error)) => {
                        tracing::error!(%error, "desktop engine drain failed");
                        Outcome::Failed
                    }
                    Err(_) => {
                        tracing::error!("desktop engine drain exceeded its shutdown deadline");
                        task.abort();
                        Outcome::TimedOut
                    }
                }
            }),
        };
        runtime.shutdown_timeout(worker_timeout);
        outcome
    }
}
