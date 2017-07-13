use std;
use std::io::Read;

pub fn in_docker() -> bool {
    match do_in_docker() {
        Ok(b) => b,
        Err(_) => false,
    }
}

fn do_in_docker() -> std::io::Result<bool> {
    let mut f = try!(std::fs::File::open("/proc/1/cgroup"));
    let mut contents = String::new();
    try!(f.read_to_string(&mut contents));

    Ok(contents.find("docker").is_some())
}
