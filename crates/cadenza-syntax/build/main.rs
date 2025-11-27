use std::fs::write;

mod test_data;
mod token;

pub fn main() {
    let _ = std::fs::create_dir_all("src/generated");

    write("src/generated.rs", GENERATED.trim_start()).unwrap();
    write("src/generated/token.rs", token::tokens()).unwrap();
    write("src/generated/test_data.rs", test_data::tests()).unwrap();
}

static GENERATED: &str = r#"
pub mod token;

#[cfg(test)]
mod test_data;
"#;
