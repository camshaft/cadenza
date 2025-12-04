#[cfg(test)]
mod tests {
    use crate::parse;

    #[test]
    fn test_record_with_equals() {
        let tests = vec![
            ("{ a = 1 }", "simple equals"),
            ("{ a = 2 + 2 }", "equals with expression"),
            ("{ a = 2\n, b = 3\n, }", "multiline with trailing comma"),
        ];
        
        for (code, desc) in tests {
            println!("\n=== Testing {}: {:?} ===", desc, code);
            let parsed = parse(code);
            println!("Errors: {}", parsed.errors.len());
            for error in &parsed.errors {
                println!("  - {:?}", error);
            }
            println!("AST: {:?}", parsed.ast());
        }
    }
}
