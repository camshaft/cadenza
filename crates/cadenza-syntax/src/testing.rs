use std::sync::OnceLock;

pub fn examples() -> &'static [Example] {
    pub static EXAMPLES: OnceLock<Box<[Example]>> = OnceLock::new();
    EXAMPLES.get_or_init(Example::load)
}

pub struct Example {
    pub name: String,
    pub src: String,
}

impl Example {
    fn load() -> Box<[Example]> {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/");
        let mut examples = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().unwrap() != "cdz" {
                continue;
            }
            let name = path.file_stem().unwrap().to_str().unwrap().to_string();
            let src = std::fs::read_to_string(path).unwrap();
            examples.push(Example { name, src });
        }
        examples.into()
    }
}
