pub fn tests() -> String {
    let valid_examples = Example::load("test-data");
    let invalid_parse_examples = Example::load("test-data/invalid-parse");
    let mut out = String::new();
    macro_rules! w {
        ($($tt:tt)*) => {
            out.push_str(&format!($($tt)*));
            out.push('\n');
        };
    }

    let types = ["lex", "cst", "ast"];

    w!("use crate::testing as t;");
    w!("use insta::assert_debug_snapshot as s;");

    // Generate valid tests
    for Example { name, src } in valid_examples.iter() {
        w!("mod {name} {{");
        w!("    use super::*;");
        w!("    static SRC: &str = {src:?};");

        for ty in types {
            let name = format!("{name}_{ty}");
            w!("    #[test]");
            w!("    fn {ty}() {{");
            w!("        s!({name:?}, t::{ty}(SRC), SRC);");
            w!("    }}");
        }
        w!("}}");
    }

    // Generate invalid parse tests - these should emit errors
    if !invalid_parse_examples.is_empty() {
        w!("mod invalid_parse {{");
        w!("    use super::*;");

        for Example { name, src } in invalid_parse_examples.iter() {
            w!("    mod {name} {{");
            w!("        use super::*;");
            w!("        static SRC: &str = {src:?};");
            // Use no_assert versions for CST and AST
            let snap_name_cst = format!("invalid_parse_{name}_cst");
            w!("        #[test]");
            w!("        fn cst() {{");
            w!("            s!({snap_name_cst:?}, t::cst_no_assert(SRC), SRC);");
            w!("        }}");
            let snap_name_ast = format!("invalid_parse_{name}_ast");
            w!("        #[test]");
            w!("        fn ast() {{");
            w!("            s!({snap_name_ast:?}, t::ast_no_assert(SRC), SRC);");
            w!("        }}");
            // Lex can use the regular function
            let snap_name_lex = format!("invalid_parse_{name}_lex");
            w!("        #[test]");
            w!("        fn lex() {{");
            w!("            s!({snap_name_lex:?}, t::lex(SRC), SRC);");
            w!("        }}");
            // Add a test that asserts errors are emitted
            let snap_name = format!("invalid_parse_{name}_errors");
            w!("        #[test]");
            w!("        fn errors() {{");
            w!("            let errors = t::parse_errors(SRC);");
            w!(
                "            assert!(!errors.is_empty(), \"expected parse errors for invalid input\");"
            );
            w!("            s!({snap_name:?}, errors, SRC);");
            w!("        }}");
            w!("    }}");
        }
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
            if path.extension().is_none_or(|ext| ext != "cdz") {
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
        examples.into()
    }
}
