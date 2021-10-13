use std::io::Read;

pub fn in_docker() -> bool {
    do_in_docker().unwrap_or(false)
}

fn do_in_docker() -> std::io::Result<bool> {
    let mut f = std::fs::File::open("/proc/1/cgroup")?;
    let mut contents = String::new();
    f.read_to_string(&mut contents)?;

    Ok(contents.contains("docker"))
}
