use std::fmt::Debug;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};

pub struct ThreadPool<T>
where
    T: Send + Debug + 'static,
{
    workers: Vec<Worker>,
    sender: mpsc::Sender<CallBackJob<T>>,
}

impl<T: Send + Debug + 'static> ThreadPool<T> {
    pub fn new(size: usize) -> Self {
        let (sender, receiver) = mpsc::channel::<CallBackJob<T>>();
        let receiver = Arc::new(Mutex::new(receiver));
        let workers = (0..size)
            .map(|id| Worker::new(id, Arc::clone(&receiver)))
            .collect::<Vec<Worker>>();

        Self { workers, sender }
    }

    pub fn execute<F, C>(&self, f: F, c: C)
    where
        F: FnOnce() -> T + Send + 'static,
        C: Fn(T) + Send + 'static,
    {
        let (result_sender, result_receiver) = mpsc::channel::<CallBackJob<T>>();
        let callback_job: CallBackJob<T> = (Box::new(f), Box::new(c));
        let _res = self.sender.send(callback_job);
    }
}

type Job<T> = Box<dyn FnOnce() -> T + Send + 'static>;
type Callback<T> = Box<dyn Fn(T) + Send + 'static>;
type CallBackJob<T> = (Job<T>, Callback<T>);

struct Worker {
    id: usize,
    join_handle: JoinHandle<()>,
}

impl Worker {
    fn new<T>(id: usize, receiver: Arc<Mutex<mpsc::Receiver<CallBackJob<T>>>>) -> Self
    where
        T: Send + 'static,
    {
        let join_handle = thread::spawn(move || loop {
            let callback_job = receiver.lock().unwrap().recv();
            if let Ok((job, callback)) = callback_job {
                let t = job();
                callback(t);
            }
        });

        Self { id, join_handle }
    }
}

#[cfg(test)]
mod testes {
    use std::{thread, time::Duration};

    use super::*;

    #[test]
    fn test_thread_pool() {
        let pool = ThreadPool::new(5);
        for x in 0..100000 {
            let result1 = pool.execute(
                move || {
                    thread::sleep(Duration::from_secs(1));
                    let y = 1 + 2 + x;
                    println!("{y}");
                    y
                },
                |x| println!("callback {x}"),
            );

            // println!("{x}: {result1}");
            //
            // assert_eq!(result1, 3 + x);
        }
        println!("OK");
        thread::sleep(Duration::from_secs(100));
    }
}
