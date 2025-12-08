#[test]
fn test_deeply_nested_inline() {
    // Create deeply nested emphasis
    let mut source = String::new();
    for _ in 0..100 {
        source.push_str("**");
    }
    source.push_str("text");
    for _ in 0..100 {
        source.push_str("**");
    }
    
    println!("Testing deeply nested emphasis...");
    let result = cadenza_markdown::parse(&source);
    println!("Success! Errors: {}", result.errors.len());
}

#[test]  
fn test_many_inline_elements() {
    // Create many inline elements in sequence
    let mut source = String::new();
    for i in 0..1000 {
        source.push_str(&format!("*item{}* ", i));
    }
    
    println!("Testing many inline elements...");
    let result = cadenza_markdown::parse(&source);
    println!("Success! Errors: {}", result.errors.len());
}
