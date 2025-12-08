#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_syntax_node_to_cst_with_markdown() {
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
        
        println!("Parsing markdown...");
        let parsed = cadenza_markdown::parse(source);
        println!("Parse completed with {} errors", parsed.errors.len());
        
        println!("Converting to CST...");
        let _cst = syntax_node_to_cst(&parsed.syntax());
        println!("CST conversion completed!");
    }
}
