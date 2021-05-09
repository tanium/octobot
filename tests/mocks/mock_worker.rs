use std::cmp::PartialEq;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use std::thread;

use octobot::worker::Worker;

#[derive(Debug)]
pub struct MockWorker<T: PartialEq + Debug + Send + Sync + 'static> {
    name: String,
    reqs: Mutex<Vec<T>>,
}

pub struct LockedMockWorker<T: PartialEq + Debug + Send + Sync + 'static> {
    worker: Mutex<Arc<MockWorker<T>>>,
}

impl<T: PartialEq + Debug + Send + Sync + 'static> MockWorker<T> {
    pub fn new(name: &str) -> Self {
        MockWorker {
            name: name.into(),
            reqs: Mutex::new(vec![]),
        }
    }

    pub fn from_reqs(name: &str, reqs: Vec<T>) -> Self {
        MockWorker {
            name: name.into(),
            reqs: Mutex::new(reqs),
        }
    }

    pub fn expect_req(&self, req: T) {
        self.reqs.lock().unwrap().push(req);
    }
}

impl<T: PartialEq + Debug + Send + Sync + 'static> LockedMockWorker<T> {
    pub fn new(name: &str) -> Self {
        Self::from(MockWorker::new(name))
    }

    pub fn from_reqs(name: &str, reqs: Vec<T>) -> Self {
        Self::from(MockWorker::from_reqs(name, reqs))
    }

    pub fn from(worker: MockWorker<T>) -> Self {
        LockedMockWorker { worker: Mutex::new(Arc::new(worker)) }
    }

    pub fn expect_req(&self, req: T) {
        self.worker.lock().unwrap().expect_req(req);
    }

    pub fn new_sender(&self) -> Arc<dyn Worker<T>> {
        let worker: &Arc<MockWorker<T>> = &*self.worker.lock().unwrap();
        worker.clone()
    }
}

impl<T: PartialEq + Debug + Send + Sync + 'static> Worker<T> for MockWorker<T> {
    fn send(&self, req: T) -> () {
        let mut reqs = self.reqs.lock().unwrap();
        assert!(reqs.len() > 0, "Unexpected request to worker {}", self.name);
        let next_req = reqs.remove(0);
        assert_eq!(next_req, req);
    }
}


impl<T: PartialEq + Debug + Send + Sync + 'static> Drop for MockWorker<T> {
    fn drop(&mut self) {
        if !thread::panicking() {
            assert!(self.reqs.lock().unwrap().len() == 0, "Unmet requests: {:?}", *self.reqs.lock().unwrap());
        }
    }
}
