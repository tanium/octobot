use std::sync::Mutex;
use std::sync::mpsc::{channel, Sender, SendError};
use std::thread::{self, JoinHandle};

#[derive(Debug)]
pub enum WorkMessage<T> {
    Stop,
    WorkItem(T),
}

pub struct Worker<T: Send + 'static> {
    sender: Mutex<Sender<WorkMessage<T>>>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Clone)]
pub struct WorkSender<T: Send + 'static> {
    sender: Sender<WorkMessage<T>>
}

pub trait Runner<T: Send + 'static> : Send {
    fn handle(&self, req: T);
}

impl<T: Send + 'static> WorkSender<T> {
    pub fn new(sender: Sender<WorkMessage<T>>) -> WorkSender<T> {
        WorkSender {
            sender: sender
        }
    }

    pub fn send(&self, msg: T) -> Result<(), SendError<WorkMessage<T>>> {
        self.sender.send(WorkMessage::WorkItem(msg))
    }

    pub fn stop(&mut self) -> Result<(), SendError<WorkMessage<T>>> {
        self.sender.send(WorkMessage::Stop)
    }
}

impl<T: Send + 'static> Drop for Worker<T> {
    fn drop(&mut self) {
        let mut sender = self.new_sender();
        match sender.stop() {
            Ok(_) => {
                match self.thread.take().unwrap().join() {
                    Ok(_) => (),
                    Err(e) => error!("Error shutting down worker: {:?}", e),
                }
            }
            Err(e) => error!("Error sending stop message: {}", e),
        }
    }
}

impl<T: Send + 'static> Worker<T> {
    pub fn new<R: Runner<T> + 'static>(handler: R) -> Worker<T> {
        let (tx, rx) = channel();

        Worker {
            sender: Mutex::new(tx),
            thread: Some(thread::spawn(move || {
                loop {
                    match rx.recv() {
                        Ok(WorkMessage::Stop) => break,
                        Ok(WorkMessage::WorkItem(req)) => handler.handle(req),
                        Err(e) => error!("Error receiving message: {}", e),
                    };
                }
            })),
        }
    }

    pub fn new_sender(&self) -> WorkSender<T> {
        let sender = self.sender.lock().unwrap();
        WorkSender {
            sender: sender.clone(),
        }
    }
}
