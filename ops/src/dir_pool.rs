use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct DirPool {
    root_dir: PathBuf,
    available_dirs: Mutex<HashMap<String, AvailableDirs>>,
}

#[derive(Eq, PartialEq)]
struct AvailableDirs {
    next_new: u32,
    unused: Vec<u32>,
}

pub struct HeldDir {
    id: u32,
    repo_root: PathBuf,
    dir: PathBuf,
    pool: Arc<DirPool>,
}

pub struct ArcDirPool {
    pool: Arc<DirPool>,
}

impl ArcDirPool {
    pub fn new(root_dir: &str) -> ArcDirPool {
        ArcDirPool {
            pool: Arc::new(DirPool::new(root_dir)),
        }
    }

    pub fn take_directory(&self, host: &str, owner: &str, repo: &str) -> HeldDir {
        DirPool::take_directory(self.pool.clone(), host, owner, repo)
    }
}

impl DirPool {
    pub fn new(root_dir: &str) -> DirPool {
        DirPool {
            root_dir: PathBuf::from(root_dir),
            available_dirs: Mutex::new(HashMap::new()),
        }
    }

    pub fn take_directory(pool: Arc<DirPool>, host: &str, owner: &str, repo: &str) -> HeldDir {
        let repo_root = pool.root_dir.join(host).join(owner).join(repo);
        let key = repo_root.to_string_lossy().into_owned();

        let id;
        {
            let mut dirs = pool.available_dirs.lock().unwrap();
            let entry = dirs.entry(key).or_insert(AvailableDirs::new());
            id = entry.get_id();
        }

        HeldDir::new(id, repo_root, pool)
    }

    fn return_dir(&self, id: u32, repo_root: &PathBuf) {
        let key = repo_root.to_string_lossy().into_owned();
        {
            let mut dirs = self.available_dirs.lock().unwrap();
            let entry = dirs.entry(key).or_insert(AvailableDirs::new());
            entry.return_id(id);
        }
    }
}

impl AvailableDirs {
    pub fn new() -> AvailableDirs {
        AvailableDirs {
            next_new: 0,
            unused: vec![],
        }
    }

    pub fn get_id(&mut self) -> u32 {
        match self.unused.pop() {
            Some(id) => id,
            None => {
                self.next_new += 1;
                self.next_new
            }
        }
    }

    pub fn return_id(&mut self, id: u32) {
        self.unused.push(id);
    }
}

impl HeldDir {
    fn new(id: u32, repo_root: PathBuf, pool: Arc<DirPool>) -> HeldDir {
        HeldDir {
            id,
            dir: repo_root.join(id.to_string()),
            repo_root,
            pool,
        }
    }

    pub fn dir(&self) -> &PathBuf {
        &self.dir
    }
}

impl Drop for HeldDir {
    fn drop(&mut self) {
        self.pool.return_dir(self.id, &self.repo_root)
    }
}

#[cfg(test)]
mod tests {
    use super::AvailableDirs;
    use super::*;

    #[test]
    fn test_available_dirs() {
        let mut dirs = AvailableDirs::new();
        assert_eq!(0, dirs.next_new);
        assert_eq!(0, dirs.unused.len());

        assert_eq!(1, dirs.get_id());
        assert_eq!(2, dirs.get_id());

        dirs.return_id(1);
        assert_eq!(1, dirs.get_id());
        assert_eq!(3, dirs.get_id());
    }

    #[test]
    fn test_dir_pool() {
        let dir_pool = ArcDirPool::new("<root>");

        {
            let dir_a1 = dir_pool.take_directory("h1", "o1", "repo-A");
            assert_eq!("<root>/h1/o1/repo-A/1", dir_a1.dir().to_string_lossy());

            let dir_a2 = dir_pool.take_directory("h1", "o1", "repo-A");
            assert_eq!("<root>/h1/o1/repo-A/2", dir_a2.dir().to_string_lossy());

            // test that different repos should have different counts
            let dir_b1 = dir_pool.take_directory("h1", "o1", "repo-B");
            assert_eq!("<root>/h1/o1/repo-B/1", dir_b1.dir().to_string_lossy());
        }

        // going out of scope should return it to the pool
        let dir_a1_again = dir_pool.take_directory("h1", "o1", "repo-A");
        assert_eq!(
            "<root>/h1/o1/repo-A/1",
            dir_a1_again.dir().to_string_lossy()
        );
    }
}
