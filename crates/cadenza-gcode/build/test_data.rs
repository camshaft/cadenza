pub fn tests() -> String {
    let examples = Example::load("test-data");
    let mut out = String::new();
    macro_rules! w {
        ($($tt:tt)*) => {
            out.push_str(&format!($($tt)*));
            out.push('\n');
        };
    }

    w!("use crate::{{parse_gcode, transpile_to_cadenza}};");
    w!("use insta::assert_snapshot as s;");

    // Generate transpile tests for each example
    for Example { name, src } in examples.iter() {
        w!("mod {name} {{");
        w!("    use super::*;");
        w!("    #[test]");
        w!("    fn transpile() {{");
        w!("        let gcode = {src:?};");
        w!("        let program = parse_gcode(gcode).expect(\"Failed to parse GCode\");");
        w!("        let cadenza = transpile_to_cadenza(&program).expect(\"Failed to transpile\");");
        let snap_name = format!("{name}_transpile");
        w!("        s!({snap_name:?}, cadenza);");
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
