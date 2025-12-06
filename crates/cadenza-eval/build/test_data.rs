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
    w!("use insta::{{assert_debug_snapshot as s, assert_snapshot as ss}};");

    // Generate eval, ast, ir, and wat tests for each example
    for Example { name, src } in examples.iter() {
        w!("mod {name} {{");
        w!("    use super::*;");
        w!("    #[test]");
        w!("    fn eval() {{");
        w!("        s!({name:?}, t::eval_all({src:?}), {src:?});");
        w!("    }}");
        w!("    #[test]");
        w!("    fn ast() {{");
        let ast_name = format!("{name}_ast");
        w!("        s!({ast_name:?}, t::ast({src:?}), {src:?});");
        w!("    }}");
        w!("    #[test]");
        w!("    fn ir() {{");
        let ir_name = format!("{name}_ir");
        w!("        ss!({ir_name:?}, t::ir({src:?}));");
        w!("    }}");
        w!("    #[test]");
        w!("    fn wat() {{");
        let wat_name = format!("{name}_wat");
        w!("        ss!({wat_name:?}, t::wat({src:?}));");
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
