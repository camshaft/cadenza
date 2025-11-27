pub fn tests() -> String {
    let examples = Example::load();
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
    for Example { name, src } in examples.iter() {
        w!("mod {name} {{");
        w!("    use super::*;");
        for ty in types {
            let name = format!("{name}_{ty}");
            w!("    #[test]");
            w!("    fn {ty}() {{");
            w!("        s!({name:?}, t::{ty}(&{src:?}), {src:?});");
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
    fn load() -> Box<[Example]> {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/test-data/");
        let mut examples = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().unwrap() != "cdz" {
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
