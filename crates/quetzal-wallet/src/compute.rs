//! Fixed, owned CPU workers. This pool must only run side-effect-free preparation.
//!
//! The owner is created/joined by bootstrap, outside the HTTP executor. Clients
//! reserve item and byte budgets before queueing. Reservations cover queued work,
//! running work and retained results. Callers must bound input capture before
//! admission and accurately reserve peak input plus result bytes; this module
//! does not make an arbitrary allocating closure memory-safe by itself.
use std::{
    collections::VecDeque,
    ops::Deref,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use tokio::sync::{oneshot, OwnedSemaphorePermit, Semaphore};

type Job = Box<dyn FnOnce() + Send + 'static>;

#[derive(Debug, thiserror::Error)]
pub enum ComputeError {
    #[error("compute capacity was exhausted before the admission deadline")]
    Busy,
    #[error("compute service is unavailable")]
    Unavailable,
    #[error("job needs an exclusive or streamed path beyond the compute byte budget")]
    RequiresStreaming,
    #[error("compute preparation panicked")]
    Panicked,
    #[error("cannot start compute worker: {0}")]
    Start(#[source] std::io::Error),
    #[error("compute worker did not join successfully")]
    Join,
    #[error("compute worker failed outside job preparation")]
    WorkerFailed,
    #[error("compute limits must be positive and fit semaphore capacities")]
    InvalidLimits,
}

/// The budget stays attached until the consumer drops the result.
#[must_use]
pub struct Computed<T> {
    value: T,
    _item: OwnedSemaphorePermit,
    _bytes: OwnedSemaphorePermit,
}

impl<T> Deref for Computed<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

struct Queue {
    accepting: bool,
    jobs: VecDeque<Job>,
}

struct Inner {
    queue: Mutex<Queue>,
    ready: Condvar,
    items: Arc<Semaphore>,
    bytes: Arc<Semaphore>,
    byte_limit: u32,
    admission_timeout: Duration,
    failed: AtomicBool,
}

impl Inner {
    fn close(&self) {
        let mut queue = match self.queue.lock() {
            Ok(queue) => queue,
            Err(poisoned) => poisoned.into_inner(),
        };
        queue.accepting = false;
        self.items.close();
        self.bytes.close();
        self.ready.notify_all();
    }

    fn fail(&self) {
        self.failed.store(true, Ordering::Release);
        self.close();
        // If the last worker fails, queued response senders must be dropped;
        // otherwise their callers would wait forever on an abandoned queue.
        let abandoned = {
            let mut queue = match self.queue.lock() {
                Ok(queue) => queue,
                Err(poisoned) => poisoned.into_inner(),
            };
            std::mem::take(&mut queue.jobs)
        };
        drop(abandoned);
    }
}

/// Cloneable admission client. Dropping a response cancels only pure preparation;
/// state-changing work belongs to the storage command service instead.
#[derive(Clone)]
pub struct ComputeClient {
    inner: Arc<Inner>,
}

impl ComputeClient {
    pub async fn execute<T, F>(&self, peak_bytes: u32, work: F) -> Result<Computed<T>, ComputeError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        if peak_bytes == 0 || peak_bytes > self.inner.byte_limit {
            return Err(ComputeError::RequiresStreaming);
        }
        let (item, bytes) = tokio::time::timeout(self.inner.admission_timeout, async {
            let item = self
                .inner
                .items
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| ComputeError::Unavailable)?;
            let bytes = self
                .inner
                .bytes
                .clone()
                .acquire_many_owned(peak_bytes)
                .await
                .map_err(|_| ComputeError::Unavailable)?;
            Ok::<_, ComputeError>((item, bytes))
        })
        .await
        .map_err(|_| ComputeError::Busy)??;
        let (reply, result) = oneshot::channel();
        let job = Box::new(move || {
            if reply.is_closed() {
                return;
            }
            let outcome = catch_unwind(AssertUnwindSafe(work))
                .map(|value| Computed {
                    value,
                    _item: item,
                    _bytes: bytes,
                })
                .map_err(|_| ComputeError::Panicked);
            let _ = reply.send(outcome);
        });
        {
            let mut queue = self
                .inner
                .queue
                .lock()
                .map_err(|_| ComputeError::Unavailable)?;
            if !queue.accepting {
                return Err(ComputeError::Unavailable);
            }
            queue.jobs.push_back(job);
            self.inner.ready.notify_one();
        }
        result.await.map_err(|_| ComputeError::Unavailable)?
    }
}

/// Owns all compute threads. Explicit shutdown drains accepted pure jobs and
/// joins the workers; bootstrap must call it after cancelling producers.
pub struct ComputePool {
    client: ComputeClient,
    workers: Vec<JoinHandle<()>>,
}

impl ComputePool {
    pub fn new(threads: usize, items: usize, bytes: u32) -> Result<Self, ComputeError> {
        Self::with_timeout(threads, items, bytes, Duration::from_millis(250))
    }

    fn with_timeout(
        threads: usize,
        items: usize,
        bytes: u32,
        admission_timeout: Duration,
    ) -> Result<Self, ComputeError> {
        if threads == 0
            || items == 0
            || bytes == 0
            || items > Semaphore::MAX_PERMITS
            || bytes as usize > Semaphore::MAX_PERMITS
        {
            return Err(ComputeError::InvalidLimits);
        }
        let inner = Arc::new(Inner {
            queue: Mutex::new(Queue {
                accepting: true,
                jobs: VecDeque::new(),
            }),
            ready: Condvar::new(),
            items: Arc::new(Semaphore::new(items)),
            bytes: Arc::new(Semaphore::new(bytes as usize)),
            byte_limit: bytes,
            admission_timeout,
            failed: AtomicBool::new(false),
        });
        let mut pool = Self {
            client: ComputeClient {
                inner: inner.clone(),
            },
            workers: Vec::new(),
        };
        for number in 0..threads {
            let shared = inner.clone();
            let worker = thread::Builder::new()
                .name(format!("quetzal-compute-{number}"))
                .spawn(move || worker_loop(&shared));
            match worker {
                Ok(worker) => pool.workers.push(worker),
                Err(error) => {
                    pool.shutdown()?;
                    return Err(ComputeError::Start(error));
                }
            }
        }
        Ok(pool)
    }

    pub fn client(&self) -> ComputeClient {
        self.client.clone()
    }

    pub fn shutdown(&mut self) -> Result<(), ComputeError> {
        self.client.inner.close();
        let mut failed = false;
        for worker in self.workers.drain(..) {
            if worker.join().is_err() {
                failed = true;
            }
        }
        if failed {
            Err(ComputeError::Join)
        } else if self.client.inner.failed.load(Ordering::Acquire) {
            Err(ComputeError::WorkerFailed)
        } else {
            Ok(())
        }
    }
}

impl Drop for ComputePool {
    fn drop(&mut self) {
        if let Err(error) = self.shutdown() {
            tracing::error!(%error,"Compute worker shutdown failed");
        }
    }
}

fn worker_loop(inner: &Inner) {
    loop {
        let job = {
            let mut queue = match inner.queue.lock() {
                Ok(queue) => queue,
                Err(poisoned) => {
                    drop(poisoned.into_inner());
                    inner.fail();
                    return;
                }
            };
            loop {
                if let Some(job) = queue.jobs.pop_front() {
                    break Some(job);
                }
                if !queue.accepting {
                    break None;
                }
                queue = match inner.ready.wait(queue) {
                    Ok(queue) => queue,
                    Err(poisoned) => {
                        drop(poisoned.into_inner());
                        inner.fail();
                        return;
                    }
                };
            }
        };
        let Some(job) = job else {
            return;
        };
        // Work panics are reported to their caller. A panic in queue plumbing or
        // a captured destructor is a service failure and stops new admissions.
        if catch_unwind(AssertUnwindSafe(job)).is_err() {
            inner.fail();
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn wait_for_jobs(client: &ComputeClient, count: usize) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if client.inner.queue.lock().unwrap().jobs.len() == count {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn expensive_work_runs_on_owned_threads_and_retained_results_hold_budget() {
        let mut pool = ComputePool::with_timeout(2, 2, 16, Duration::from_millis(20)).unwrap();
        let client = pool.client();
        let value = client
            .execute(16, || thread::current().name().map(str::to_owned))
            .await
            .unwrap();
        assert!(value.as_deref().unwrap().starts_with("quetzal-compute-"));
        assert!(matches!(
            client.execute(1, || 42).await,
            Err(ComputeError::Busy)
        ));
        drop(value);
        assert_eq!(*client.execute(16, || 42).await.unwrap(), 42);
        assert!(matches!(
            client.execute(17, || 42).await,
            Err(ComputeError::RequiresStreaming)
        ));
        pool.shutdown().unwrap();
        assert!(matches!(
            client.execute(1, || 42).await,
            Err(ComputeError::Unavailable)
        ));
        assert!(pool.workers.is_empty());
    }

    #[tokio::test]
    async fn panic_releases_permits_and_worker_can_process_the_next_job() {
        let mut pool = ComputePool::new(1, 1, 16).unwrap();
        let client = pool.client();
        assert!(matches!(
            client.execute(16, || panic!("public test panic")).await,
            Err(ComputeError::Panicked)
        ));
        assert_eq!(*client.execute(16, || 7).await.unwrap(), 7);
        pool.shutdown().unwrap();
    }

    #[test]
    fn invalid_limits_do_not_spawn_workers() {
        assert!(matches!(
            ComputePool::new(0, 1, 1),
            Err(ComputeError::InvalidLimits)
        ));
        assert!(matches!(
            ComputePool::new(1, 0, 1),
            Err(ComputeError::InvalidLimits)
        ));
        assert!(matches!(
            ComputePool::new(1, 1, 0),
            Err(ComputeError::InvalidLimits)
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_drains_accepted_jobs_and_cancellation_releases_queued_budget() {
        let mut pool = ComputePool::new(1, 3, 3).unwrap();
        let client = pool.client();
        let (started, ready) = oneshot::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let first = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .execute(1, move || {
                        let _ = started.send(());
                        blocked.recv().unwrap();
                        1
                    })
                    .await
            }
        });
        ready.await.unwrap();
        let second = tokio::spawn({
            let client = client.clone();
            async move { client.execute(1, || 2).await }
        });
        let third = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .execute(1, || panic!("cancelled job must not execute"))
                    .await
            }
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if client.inner.queue.lock().unwrap().jobs.len() == 2 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        third.abort();
        let _ = third.await;
        client.inner.close();
        release.send(()).unwrap();
        assert_eq!(*first.await.unwrap().unwrap(), 1);
        assert_eq!(*second.await.unwrap().unwrap(), 2);
        pool.shutdown().unwrap();
        assert_eq!(client.inner.bytes.available_permits(), 3);
        assert_eq!(client.inner.items.available_permits(), 3);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn failed_last_worker_rejects_admission_and_completes_abandoned_callers() {
        struct BadDrop;
        impl Drop for BadDrop {
            fn drop(&mut self) {
                panic!("public test destructor failure");
            }
        }
        let mut pool = ComputePool::new(1, 3, 3).unwrap();
        let client = pool.client();
        let (started, ready) = oneshot::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let first = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .execute(1, move || {
                        let _ = started.send(());
                        blocked.recv().unwrap();
                        1
                    })
                    .await
            }
        });
        ready.await.unwrap();
        let second = tokio::spawn({
            let client = client.clone();
            let bad = BadDrop;
            async move {
                client
                    .execute(1, move || {
                        drop(bad);
                        2
                    })
                    .await
            }
        });
        wait_for_jobs(&client, 1).await;
        let third = tokio::spawn({
            let client = client.clone();
            async move { client.execute(1, || 3).await }
        });
        wait_for_jobs(&client, 2).await;
        second.abort();
        let _ = second.await;
        release.send(()).unwrap();
        assert_eq!(*first.await.unwrap().unwrap(), 1);
        let abandoned = tokio::time::timeout(Duration::from_secs(2), third)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(abandoned, Err(ComputeError::Unavailable)));
        assert!(matches!(pool.shutdown(), Err(ComputeError::WorkerFailed)));
        assert!(matches!(
            client.execute(1, || 4).await,
            Err(ComputeError::Unavailable)
        ));
        assert_eq!(client.inner.bytes.available_permits(), 3);
    }
}
