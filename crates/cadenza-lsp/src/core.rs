//! Core LSP utilities shared between native and WASM implementations.

use lsp_types::*;

/// Convert cadenza parse errors to LSP diagnostics.
pub fn parse_to_diagnostics(source: &str) -> Vec<Diagnostic> {
    let parsed = cadenza_syntax::parse::parse(source);
    
    parsed
        .errors
        .iter()
        .map(|error| {
            let start_pos = offset_to_position(source, error.span.start);
            let end_pos = offset_to_position(source, error.span.end);

            Diagnostic {
                range: Range::new(start_pos, end_pos),
                severity: Some(DiagnosticSeverity::ERROR),
                code: None,
                code_description: None,
                source: Some("cadenza".to_string()),
                message: error.message.clone(),
                related_information: None,
                tags: None,
                data: None,
            }
        })
        .collect()
}

/// Convert a byte offset to an LSP Position.
pub fn offset_to_position(text: &str, offset: usize) -> Position {
    let mut line = 0;
    let mut character = 0;
    
    for (i, ch) in text.char_indices() {
        if i >= offset {
            break;
        }
        
        if ch == '\n' {
            line += 1;
            character = 0;
        } else {
            character += 1;
        }
    }
    
    Position::new(line, character)
}

/// Convert an LSP Position to a byte offset.
pub fn position_to_offset(text: &str, position: Position) -> usize {
    let mut current_line = 0;
    let mut current_char = 0;
    
    for (i, ch) in text.char_indices() {
        if current_line == position.line && current_char == position.character {
            return i;
        }
        
        if ch == '\n' {
            current_line += 1;
            current_char = 0;
        } else {
            current_char += 1;
        }
    }
    
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_offset_to_position() {
        let text = "hello\nworld";
        assert_eq!(offset_to_position(text, 0), Position::new(0, 0));
        assert_eq!(offset_to_position(text, 5), Position::new(0, 5));
        assert_eq!(offset_to_position(text, 6), Position::new(1, 0));
        assert_eq!(offset_to_position(text, 11), Position::new(1, 5));
    }

    #[test]
    fn test_position_to_offset() {
        let text = "hello\nworld";
        assert_eq!(position_to_offset(text, Position::new(0, 0)), 0);
        assert_eq!(position_to_offset(text, Position::new(0, 5)), 5);
        assert_eq!(position_to_offset(text, Position::new(1, 0)), 6);
        assert_eq!(position_to_offset(text, Position::new(1, 5)), 11);
    }

    #[test]
    fn test_parse_to_diagnostics() {
        let source = "1 + + 2";
        let diagnostics = parse_to_diagnostics(source);
        assert!(!diagnostics.is_empty());
        assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::ERROR));
    }
}
