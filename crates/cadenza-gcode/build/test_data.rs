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
    w!("use crate::testing::verify_cst_coverage;");
    w!("use insta::assert_debug_snapshot as s;");
    w!("");

    // Generate CST and AST snapshot tests for each example
    for Example { name, src } in examples.iter() {
        w!("mod {name} {{");
        w!("    use super::*;");

        // CST test to verify all bytes are attributed to tokens
        w!("    #[test]");
        w!("    fn cst() {{");
        w!("        let gcode = {src:?};");
        w!("        let parse = parse(gcode);");
        w!("        let cst = parse.syntax();");
        w!("");
        w!("        // Verify CST span coverage and token text accuracy");
        w!("        verify_cst_coverage(gcode);");
        w!("");
        let snap_name_cst = format!("{name}_cst");
        w!("        s!({snap_name_cst:?}, &cst, {src:?});");
        w!("    }}");

        // AST test
        w!("    #[test]");
        w!("    fn ast() {{");
        w!("        let gcode = {src:?};");
        w!("        let parse = parse(gcode);");
        w!("        let root = parse.ast();");
        let snap_name_ast = format!("{name}_ast");
        w!("        s!({snap_name_ast:?}, root, {src:?});");
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
