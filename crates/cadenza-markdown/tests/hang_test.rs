// Test to reproduce markdown parser hang

use std::time::{Duration, Instant};
use std::thread;

fn test_with_timeout(name: &str, source: &str, timeout_secs: u64) -> bool {
    println!("\n=== Testing: {} ===", name);
    let preview = if source.len() > 60 { &source[..60] } else { source };
    println!("Source: {:?}...", preview);
    
    let source = source.to_string();
    let handle = thread::spawn(move || {
        cadenza_markdown::parse(&source)
    });
    
    let start = Instant::now();
    loop {
        if start.elapsed() > Duration::from_secs(timeout_secs) {
            println!("❌ TIMEOUT after {} seconds - INFINITE LOOP DETECTED!", timeout_secs);
            return false;
        }
        
        if handle.is_finished() {
            match handle.join() {
                Ok(result) => {
                    println!("✓ Completed in {:.3}s ({} errors)", 
                             start.elapsed().as_secs_f64(), result.errors.len());
                    return true;
                }
                Err(_) => {
                    println!("❌ Thread panicked");
                    return false;
                }
            }
        }
        
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn test_no_infinite_loops() {
    println!("Testing markdown parser for infinite loops...\n");
    
    // Test basic cases
    assert!(test_with_timeout("Empty string", "", 2));
    assert!(test_with_timeout("Simple text", "Hello World", 2));
    assert!(test_with_timeout("Heading", "# Hello", 2));
    assert!(test_with_timeout("Emphasis", "*italic*", 2));
    assert!(test_with_timeout("Bold", "**bold**", 2));
    
    // Test example content (Cadenza code parsed as markdown)
    let cadenza_example = r#"# Welcome to Cadenza!
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
    assert!(test_with_timeout("Cadenza example (as markdown)", cadenza_example, 5));
    
    // Test content with multiplication
    assert!(test_with_timeout("Multiplication", "x * x", 2));
    assert!(test_with_timeout("Expression", "1 + 2 * 3", 2));
    assert!(test_with_timeout("Function definition", "fn square x = x * x", 2));
    
    println!("\n=== All tests completed ===");
}
