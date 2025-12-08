use std::time::{Duration, Instant};

#[test]
fn test_parse_cadenza_example_as_markdown() {
    let source = r#"# Welcome to Cadenza!
# A functional language with units of measure

# Try some basic expressions
42
3.14159
1 + 2 * 3

# Define variables
let name = "Cadenza"
let version = 0.1

# Create functions
fn square x = x * x
square 5
"#;
    
    let start = Instant::now();
    let result = cadenza_markdown::parse(source);
    let elapsed = start.elapsed();
    
    println!("Parse completed in {:?}", elapsed);
    println!("Errors: {}", result.errors.len());
    
    assert!(elapsed < Duration::from_secs(1), "Parsing took too long: {:?}", elapsed);
}
