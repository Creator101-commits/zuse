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

/// AppKit exits from applicationWillTerminate rather than returning from run.
/// Dropping a quit observer must therefore drain before termination resumes.
pub struct TerminationDrain {
    shutdown: DesktopShutdown,
    runtime: Handle,
    timeout: Duration,
}

impl Drop for TerminationDrain {
    fn drop(&mut self) {
        self.shutdown.wait(&self.runtime, self.timeout);
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Complete,
    TimedOut,
    Failed,
}

impl DesktopShutdown {
    pub fn before_termination(&self, runtime: &Handle, timeout: Duration) -> TerminationDrain {
        TerminationDrain {
            shutdown: self.clone(),
            runtime: runtime.clone(),
            timeout,
        }
    }
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

    /// Wait on the UI/main thread, outside a Tokio task, while workers drain.
    pub fn wait(&self, runtime: &Handle, drain_timeout: Duration) -> Outcome {
        let task = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        match task {
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
        }
    }

    /// Called after the GUI event loop returns. Never use Runtime's unbounded
    /// drop: blocking provider/file workers can otherwise strand a closed app.
    pub fn finish(
        self,
        runtime: Runtime,
        drain_timeout: Duration,
        worker_timeout: Duration,
    ) -> Outcome {
        let outcome = self.wait(runtime.handle(), drain_timeout);
        runtime.shutdown_timeout(worker_timeout);
        outcome
    }
}
