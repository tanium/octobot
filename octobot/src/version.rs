use std::sync::LazyLock;

// The short commit hash that build.py writes next to the binary. Builds made
// outside build.py have no version file.
pub static VERSION: LazyLock<Option<String>> = LazyLock::new(|| {
    let mut path = std::env::current_exe().ok()?;
    path.pop();
    path.push("version");
    let version = std::fs::read_to_string(path).ok()?;
    Some(version.trim().to_string())
});
