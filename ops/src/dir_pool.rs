use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct DirPool {
    root_dir: PathBuf,
    available_dirs: Mutex<HashMap<String, AvailableDirs>>,
}

#[derive(Eq, PartialEq, Clone)]
struct DirEntry {
    id: u32,
    last_used: Instant,
}

#[derive(Eq, PartialEq)]
struct AvailableDirs {
    next_new: u32,
    unused: Vec<DirEntry>,
}

pub struct HeldDir {
    entry: DirEntry,
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

    pub fn clean(&self, expiration: Duration) {
        self.pool.clean(expiration)
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

        let entry;
        {
            let mut dirs = pool.available_dirs.lock().unwrap();
            let dir = dirs.entry(key).or_insert_with(AvailableDirs::new);
            entry = dir.get_entry();
        }

        HeldDir::new(entry, repo_root, pool)
    }

    fn return_dir(&self, entry: DirEntry, repo_root: &Path) {
        let key = repo_root.to_string_lossy().into_owned();
        {
            let mut dirs = self.available_dirs.lock().unwrap();
            let dir = dirs.entry(key).or_insert_with(AvailableDirs::new);
            dir.return_entry(entry);
        }
    }

    pub fn clean(&self, expiration: Duration) {
        let deadline = Instant::now() - expiration;
        let mut locked_dirs = self.available_dirs.lock().unwrap();
        for (repo_root_str, dirs) in locked_dirs.iter_mut() {
            let repo_root = Path::new(repo_root_str);

            let new_dirs = dirs
                .unused
                .clone()
                .into_iter()
                .filter_map(|dir| {
                    if dir.last_used < deadline {
                        let path = repo_root.join(dir.id.to_string());

                        if let Err(e) = std::fs::remove_dir_all(&path) {
                            log::error!("Failed to remove stale directory {:?}: {}", path, e);
                        }

                        None
                    } else {
                        Some(dir)
                    }
                })
                .collect::<Vec<_>>();
            dirs.unused = new_dirs;
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

    pub fn get_entry(&mut self) -> DirEntry {
        match self.unused.pop() {
            Some(d) => d,
            None => {
                self.next_new += 1;
                DirEntry {
                    id: self.next_new,
                    last_used: Instant::now(),
                }
            }
        }
    }

    pub fn return_entry(&mut self, mut d: DirEntry) {
        d.last_used = Instant::now();
        self.unused.push(d);
    }
}

impl HeldDir {
    fn new(entry: DirEntry, repo_root: PathBuf, pool: Arc<DirPool>) -> HeldDir {
        let dir = repo_root.join(entry.id.to_string());
        HeldDir {
            entry,
            dir,
            repo_root,
            pool,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for HeldDir {
    fn drop(&mut self) {
        let entry = self.entry.clone();
        self.pool.return_dir(entry, &self.repo_root)
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

        let entry1 = dirs.get_entry();
        let entry2 = dirs.get_entry();

        assert_eq!(1, entry1.id);
        assert_eq!(2, entry2.id);

        dirs.return_entry(entry1);
        assert_eq!(1, dirs.get_entry().id);
        assert_eq!(3, dirs.get_entry().id);
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
