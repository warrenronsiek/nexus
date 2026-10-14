// @feature analyst
// @spec docs/features/analyst.md

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

pub fn write_executable(path: &Path, contents: &str) {
    std::fs::write(path, contents).unwrap();
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
}
