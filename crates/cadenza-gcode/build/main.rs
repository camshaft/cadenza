use std::{
    fs::write,
    io::Write,
    process::{Command, Stdio},
};

mod test_data;

pub fn main() {
    let _ = std::fs::create_dir_all("src/generated");

    write("src/generated.rs", GENERATED.trim_start()).unwrap();
    write(
        "src/generated/test_data.rs",
        rustfmt(&test_data::tests()),
    )
    .unwrap();

    println!("cargo:rerun-if-changed=test-data/");
}

fn rustfmt(code: &str) -> String {
    let mut child = Command::new("rustfmt")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("failed to run rustfmt");

    let mut stdin = child.stdin.take().expect("failed to open stdin");
    stdin.write_all(code.as_bytes()).expect("failed to write");
    drop(stdin);

    let output = child
        .wait_with_output()
        .expect("failed to wait on rustfmt");
    if !output.status.success() {
        panic!("rustfmt failed");
    }

    String::from_utf8(output.stdout).expect("rustfmt output was not valid UTF-8")
}

static GENERATED: &str = r#"
#[cfg(test)]
mod test_data;
"#;
