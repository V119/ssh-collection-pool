use std::fmt::Debug;
use std::sync::Arc;

use tokio::{
    sync::{mpsc, oneshot, Mutex},
    task::JoinHandle,
};

pub struct ThreadPool {
    workers: Vec<Worker>,
    sender: mpsc::Sender<Job>,
}

impl ThreadPool {
    pub fn new(size: usize) -> Self {
        let (sender, receiver) = mpsc::channel::<Job>(size);
        let receiver = Arc::new(Mutex::new(receiver));
        let workers = (0..size)
            .map(|id| Worker::new(id, Arc::clone(&receiver)))
            .collect::<Vec<Worker>>();

        Self { workers, sender }
    }

    pub async fn execute<F, T>(&self, f: F)
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + Debug + 'static,
    {
        let (result_sender, result_receiver) = oneshot::channel::<T>();
        let _res = self
            .sender
            .send(Box::new(move || {
                let reult = f();
                result_sender.send(reult).unwrap();
            }))
            .await;

        // result_receiver.await.unwrap()
    }
}

type Job = Box<dyn FnOnce() + Send + 'static>;

struct Worker {
    id: usize,
    join_handle: JoinHandle<()>,
}

impl Worker {
    fn new(id: usize, receiver: Arc<Mutex<mpsc::Receiver<Job>>>) -> Self {
        let join_handle = tokio::task::spawn(async move {
            loop {
                let job = receiver.lock().await.recv().await;
                if let Some(job) = job {
                    tokio::task::spawn_blocking(move || job()).await.unwrap();
                }
            }
        });

        Self { id, join_handle }
    }
}

#[cfg(test)]
mod testes {
    use std::{thread, time::Duration};

    use super::*;

    #[tokio::test]
    async fn test_thread_pool() {
        let pool = ThreadPool::new(5);
        for x in 0..10000 {
            let result1 = pool
                .execute(move || {
                    thread::sleep(Duration::from_secs(1));
                    let y = 1 + 2 + x;
                    println!("{y}");
                })
                .await;

            // println!("{x}: {result1}");
            //
            // assert_eq!(result1, 3 + x);
        }
        println!("OK");
    }
}
