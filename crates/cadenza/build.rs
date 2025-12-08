fn main() {
    println!("cargo:rerun-if-changed=test-data");

    // Create src/generated directory if it doesn't exist
    let _ = std::fs::create_dir_all("src/generated");
    let _ = std::fs::create_dir_all("src/generated/snapshots");

    // Generate test code
    let test_code = test_data::tests();
    let dest_path = std::path::Path::new("src/generated/test_data.rs");
    std::fs::write(dest_path, test_code).unwrap();
}

mod test_data {
    pub fn tests() -> String {
        let examples = Example::load("test-data");
        let mut out = String::new();
        macro_rules! w {
            ($($tt:tt)*) => {
                out.push_str(&format!($($tt)*));
                out.push('\n');
            };
        }

        w!("use crate::testing as t;");
        w!("use insta::assert_snapshot as s;");

        // Generate REPL tests for each example
        for Example { name, src } in examples.iter() {
            w!("mod {name} {{");
            w!("    use super::*;");
            w!("    #[test]");
            w!("    fn repl() {{");
            w!("        s!({name:?}, t::repl({src:?}));");
            w!("    }}");
            w!("}}");
        }

        out
    }

    pub struct Example {
        pub name: String,
        pub src: String,
    }

    impl Example {
        fn load(subdir: &str) -> Box<[Example]> {
            let dir = format!("{}/{}/", env!("CARGO_MANIFEST_DIR"), subdir);
            let mut examples = Vec::new();
            let Ok(entries) = std::fs::read_dir(&dir) else {
                return examples.into();
            };
            for entry in entries {
                let entry = entry.unwrap();
                let path = entry.path();
                // Skip directories
                if path.is_dir() {
                    continue;
                }
                if path.extension().is_none_or(|ext| ext != "repl") {
                    continue;
                }
                let name = path
                    .file_stem()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .replace('-', "_");
                let src = std::fs::read_to_string(path).unwrap();
                examples.push(Example { name, src });
            }
            // Sort examples by name for consistent ordering
            examples.sort_by(|a, b| a.name.cmp(&b.name));
            examples.into()
        }
    }
}
