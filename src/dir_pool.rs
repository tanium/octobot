use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time;

pub struct DirPool {
    root_dir: PathBuf,
    available: AtomicUsize,
}

pub struct HeldDir<'a> {
    dir: PathBuf,
    pool: &'a DirPool,
}

impl DirPool {
    pub fn new(root_dir: &str, count: usize) -> DirPool {
        DirPool {
            root_dir: PathBuf::from(root_dir),
            available: AtomicUsize::new(count),
        }
    }

    pub fn take_directory(&self, host: &str, owner: &str, repo: &str) -> Result<HeldDir, String> {
        let duration = time::Duration::from_millis(500);
        // give up after 1 minute
        let max_duration = time::Duration::from_secs(60);
        let mut total_slept: time::Duration = time::Duration::from_millis(0);

        let repo_root = self.root_dir.join(host).join(owner).join(repo);

        let mut id: usize;
        loop {
            id = self.available.fetch_sub(1, Ordering::Release);
            if id > 0 {
                break;
            }

            self.return_dir();

            thread::sleep(duration);
            total_slept += duration;
            if total_slept >= max_duration {
                return Err(format!("Could not find open directory after waiting {:?}",
                                   total_slept));
            }
        }

        Ok(HeldDir::new(repo_root.join(id.to_string()), &self))
    }

    fn return_dir(&self) -> () {
        self.available.fetch_add(1, Ordering::Relaxed);
    }
}

impl<'a> HeldDir<'a> {
    fn new(dir: PathBuf, pool: &DirPool) -> HeldDir {
        HeldDir {
            dir: dir,
            pool: pool,
        }
    }

    pub fn dir(&self) -> &PathBuf {
        &self.dir
    }
}

impl<'a> Drop for HeldDir<'a> {
    fn drop(&mut self) {
        self.pool.return_dir()
    }
}
