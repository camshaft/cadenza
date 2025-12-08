#[test]
fn test_utf8_handling() {
    // Test with various UTF-8 characters
    let sources = vec![
        "# Hello 世界",
        "**bold** with émojis 🎉",
        "Cырилица and العربية",
        "`code with 日本語`",
    ];
    
    for source in sources {
        println!("Testing UTF-8: {}", source);
        let result = cadenza_markdown::parse(source);
        println!("  Errors: {}", result.errors.len());
        assert!(result.errors.len() < 10, "Too many errors for valid UTF-8 markdown");
    }
}
