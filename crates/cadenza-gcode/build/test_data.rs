pub fn tests() -> String {
    let examples = Example::load("test-data");
    let mut out = String::new();
    macro_rules! w {
        ($($tt:tt)*) => {
            out.push_str(&format!($($tt)*));
            out.push('\n');
        };
    }

    w!("use crate::parse;");
    w!("use insta::assert_snapshot as s;");
    w!("");

    // Generate parse and AST snapshot tests for each example
    for Example { name, src } in examples.iter() {
        w!("mod {name} {{");
        w!("    use super::*;");
        w!("    #[test]");
        w!("    fn parse_ast() {{");
        w!("        let gcode = {src:?};");
        w!("        let parse = parse(gcode);");
        w!("        let root = parse.ast();");
        w!("        let ast_debug = format!(\"{{:?}}\", root);");
        let snap_name = format!("{name}_parse_ast");
        w!("        s!({snap_name:?}, ast_debug);");
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
            if path.extension().is_none_or(|ext| ext != "gcode") {
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
        examples.sort_by(|a, b| a.name.cmp(&b.name));
        examples.into()
    }
}
