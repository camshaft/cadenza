use std::{
    io::Write as _,
    path::Path,
    process::{Command, Stdio},
};

pub fn rustfmt(code: &str) -> String {
    let mut child = Command::new("rustfmt")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("failed to run rustfmt");

    let mut stdin = child.stdin.take().expect("failed to open stdin");
    stdin.write_all(code.as_bytes()).expect("failed to write");
    drop(stdin);

    let output = child.wait_with_output().expect("failed to wait on rustfmt");
    if !output.status.success() {
        panic!("rustfmt failed");
    }

    String::from_utf8(output.stdout).expect("rustfmt output was not valid UTF-8")
}

pub fn emit(path: impl AsRef<Path>, code: &str) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("failed to create parent directory");
    }
    if let Ok(existing) = std::fs::read_to_string(path)
        && existing == code {
            return;
        }
    std::fs::write(path, code)
        .unwrap_or_else(|err| panic!("failed to write to {}: {err}", path.display()));
}
