use std;
use std::io::Read;

pub fn in_docker() -> bool {
    match do_in_docker() {
        Ok(b) => b,
        Err(_) => false,
    }
}

fn do_in_docker() -> std::io::Result<bool> {
    let mut f = std::fs::File::open("/proc/1/cgroup")?;
    let mut contents = String::new();
    f.read_to_string(&mut contents)?;

    Ok(contents.find("docker").is_some())
}
