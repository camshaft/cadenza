use cadenza_web::{lex, parse_source, Syntax};

#[test]
fn test_individual_wasm_functions() {
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
    
    println!("Testing individual WASM functions with Cadenza example as Markdown...\n");
    
    println!("1. Testing lex()...");
    let _lex_result = lex(source);
    println!("   ✓ lex completed\n");
    
    println!("2. Testing parse_source()...");
    let syntax_js = serde_wasm_bindgen::to_value(&Syntax::Markdown).unwrap();
    let _parse_result = parse_source(source, syntax_js);
    println!("   ✓ parse_source completed\n");
    
    println!("All tests passed!");
}
