use crate::testing as t;
use insta::assert_debug_snapshot as s;
mod lit_float {
    use super::*;
    #[test]
    fn lex() {
        s!("lit_float_lex", t::lex(&"3.14"), "3.14");
    }
    #[test]
    fn cst() {
        s!("lit_float_cst", t::cst(&"3.14"), "3.14");
    }
    #[test]
    fn ast() {
        s!("lit_float_ast", t::ast(&"3.14"), "3.14");
    }
}
mod ap_simple {
    use super::*;
    #[test]
    fn lex() {
        s!("ap_simple_lex", t::lex(&"foo 1 2 3 baz bar"), "foo 1 2 3 baz bar");
    }
    #[test]
    fn cst() {
        s!("ap_simple_cst", t::cst(&"foo 1 2 3 baz bar"), "foo 1 2 3 baz bar");
    }
    #[test]
    fn ast() {
        s!("ap_simple_ast", t::ast(&"foo 1 2 3 baz bar"), "foo 1 2 3 baz bar");
    }
}
mod op_add {
    use super::*;
    #[test]
    fn lex() {
        s!("op_add_lex", t::lex(&"a + b\n"), "a + b\n");
    }
    #[test]
    fn cst() {
        s!("op_add_cst", t::cst(&"a + b\n"), "a + b\n");
    }
    #[test]
    fn ast() {
        s!("op_add_ast", t::ast(&"a + b\n"), "a + b\n");
    }
}
mod let_op_or {
    use super::*;
    #[test]
    fn lex() {
        s!("let_op_or_lex", t::lex(&"let x = a || b"), "let x = a || b");
    }
    #[test]
    fn cst() {
        s!("let_op_or_cst", t::cst(&"let x = a || b"), "let x = a || b");
    }
    #[test]
    fn ast() {
        s!("let_op_or_ast", t::ast(&"let x = a || b"), "let x = a || b");
    }
}
mod let_decl {
    use super::*;
    #[test]
    fn lex() {
        s!("let_decl_lex", t::lex(&"let x"), "let x");
    }
    #[test]
    fn cst() {
        s!("let_decl_cst", t::cst(&"let x"), "let x");
    }
    #[test]
    fn ast() {
        s!("let_decl_ast", t::ast(&"let x"), "let x");
    }
}
mod ws_paren {
    use super::*;
    #[test]
    fn lex() {
        s!("ws_paren_lex", t::lex(&"foo (\n    a + b\n) c d e + f\n"), "foo (\n    a + b\n) c d e + f\n");
    }
    #[test]
    fn cst() {
        s!("ws_paren_cst", t::cst(&"foo (\n    a + b\n) c d e + f\n"), "foo (\n    a + b\n) c d e + f\n");
    }
    #[test]
    fn ast() {
        s!("ws_paren_ast", t::ast(&"foo (\n    a + b\n) c d e + f\n"), "foo (\n    a + b\n) c d e + f\n");
    }
}
mod op_sub {
    use super::*;
    #[test]
    fn lex() {
        s!("op_sub_lex", t::lex(&"a - b"), "a - b");
    }
    #[test]
    fn cst() {
        s!("op_sub_cst", t::cst(&"a - b"), "a - b");
    }
    #[test]
    fn ast() {
        s!("op_sub_ast", t::ast(&"a - b"), "a - b");
    }
}
mod op_add_mul {
    use super::*;
    #[test]
    fn lex() {
        s!("op_add_mul_lex", t::lex(&"a + b * c"), "a + b * c");
    }
    #[test]
    fn cst() {
        s!("op_add_mul_cst", t::cst(&"a + b * c"), "a + b * c");
    }
    #[test]
    fn ast() {
        s!("op_add_mul_ast", t::ast(&"a + b * c"), "a + b * c");
    }
}
mod lit_multi_line {
    use super::*;
    #[test]
    fn lex() {
        s!("lit_multi_line_lex", t::lex(&"let a = 1\nlet b = 2"), "let a = 1\nlet b = 2");
    }
    #[test]
    fn cst() {
        s!("lit_multi_line_cst", t::cst(&"let a = 1\nlet b = 2"), "let a = 1\nlet b = 2");
    }
    #[test]
    fn ast() {
        s!("lit_multi_line_ast", t::ast(&"let a = 1\nlet b = 2"), "let a = 1\nlet b = 2");
    }
}
mod lit_empty_string {
    use super::*;
    #[test]
    fn lex() {
        s!("lit_empty_string_lex", t::lex(&"\"\""), "\"\"");
    }
    #[test]
    fn cst() {
        s!("lit_empty_string_cst", t::cst(&"\"\""), "\"\"");
    }
    #[test]
    fn ast() {
        s!("lit_empty_string_ast", t::ast(&"\"\""), "\"\"");
    }
}
mod ap_single {
    use super::*;
    #[test]
    fn lex() {
        s!("ap_single_lex", t::lex(&"foo bar"), "foo bar");
    }
    #[test]
    fn cst() {
        s!("ap_single_cst", t::cst(&"foo bar"), "foo bar");
    }
    #[test]
    fn ast() {
        s!("ap_single_ast", t::ast(&"foo bar"), "foo bar");
    }
}
mod lit_multiline_string {
    use super::*;
    #[test]
    fn lex() {
        s!("lit_multiline_string_lex", t::lex(&"\"\nhello\nworld!\n\""), "\"\nhello\nworld!\n\"");
    }
    #[test]
    fn cst() {
        s!("lit_multiline_string_cst", t::cst(&"\"\nhello\nworld!\n\""), "\"\nhello\nworld!\n\"");
    }
    #[test]
    fn ast() {
        s!("lit_multiline_string_ast", t::ast(&"\"\nhello\nworld!\n\""), "\"\nhello\nworld!\n\"");
    }
}
mod let_eq_lit {
    use super::*;
    #[test]
    fn lex() {
        s!("let_eq_lit_lex", t::lex(&"let x = 42"), "let x = 42");
    }
    #[test]
    fn cst() {
        s!("let_eq_lit_cst", t::cst(&"let x = 42"), "let x = 42");
    }
    #[test]
    fn ast() {
        s!("let_eq_lit_ast", t::ast(&"let x = 42"), "let x = 42");
    }
}
mod ws_block {
    use super::*;
    #[test]
    fn lex() {
        s!("ws_block_lex", t::lex(&"let foo = # comment here\n    let bar = 123 # other comment here\n    bar\n"), "let foo = # comment here\n    let bar = 123 # other comment here\n    bar\n");
    }
    #[test]
    fn cst() {
        s!("ws_block_cst", t::cst(&"let foo = # comment here\n    let bar = 123 # other comment here\n    bar\n"), "let foo = # comment here\n    let bar = 123 # other comment here\n    bar\n");
    }
    #[test]
    fn ast() {
        s!("ws_block_ast", t::ast(&"let foo = # comment here\n    let bar = 123 # other comment here\n    bar\n"), "let foo = # comment here\n    let bar = 123 # other comment here\n    bar\n");
    }
}
mod op_mul_add {
    use super::*;
    #[test]
    fn lex() {
        s!("op_mul_add_lex", t::lex(&"a * b + c"), "a * b + c");
    }
    #[test]
    fn cst() {
        s!("op_mul_add_cst", t::cst(&"a * b + c"), "a * b + c");
    }
    #[test]
    fn ast() {
        s!("op_mul_add_ast", t::ast(&"a * b + c"), "a * b + c");
    }
}
mod ap_op_add {
    use super::*;
    #[test]
    fn lex() {
        s!("ap_op_add_lex", t::lex(&"foo 1 + 2"), "foo 1 + 2");
    }
    #[test]
    fn cst() {
        s!("ap_op_add_cst", t::cst(&"foo 1 + 2"), "foo 1 + 2");
    }
    #[test]
    fn ast() {
        s!("ap_op_add_ast", t::ast(&"foo 1 + 2"), "foo 1 + 2");
    }
}
mod lit_string {
    use super::*;
    #[test]
    fn lex() {
        s!("lit_string_lex", t::lex(&"\"hello\""), "\"hello\"");
    }
    #[test]
    fn cst() {
        s!("lit_string_cst", t::cst(&"\"hello\""), "\"hello\"");
    }
    #[test]
    fn ast() {
        s!("lit_string_ast", t::ast(&"\"hello\""), "\"hello\"");
    }
}
mod op_pipe {
    use super::*;
    #[test]
    fn lex() {
        s!("op_pipe_lex", t::lex(&"foo 1 + 2\n|> bar 3\n|> baz 4 * 5\n"), "foo 1 + 2\n|> bar 3\n|> baz 4 * 5\n");
    }
    #[test]
    fn cst() {
        s!("op_pipe_cst", t::cst(&"foo 1 + 2\n|> bar 3\n|> baz 4 * 5\n"), "foo 1 + 2\n|> bar 3\n|> baz 4 * 5\n");
    }
    #[test]
    fn ast() {
        s!("op_pipe_ast", t::ast(&"foo 1 + 2\n|> bar 3\n|> baz 4 * 5\n"), "foo 1 + 2\n|> bar 3\n|> baz 4 * 5\n");
    }
}
mod ap_nested {
    use super::*;
    #[test]
    fn lex() {
        s!("ap_nested_lex", t::lex(&"foo 1 (bar 2) 3 4"), "foo 1 (bar 2) 3 4");
    }
    #[test]
    fn cst() {
        s!("ap_nested_cst", t::cst(&"foo 1 (bar 2) 3 4"), "foo 1 (bar 2) 3 4");
    }
    #[test]
    fn ast() {
        s!("ap_nested_ast", t::ast(&"foo 1 (bar 2) 3 4"), "foo 1 (bar 2) 3 4");
    }
}
mod lit_string_with_escape {
    use super::*;
    #[test]
    fn lex() {
        s!("lit_string_with_escape_lex", t::lex(&"\"hello\\nworld\\\"\""), "\"hello\\nworld\\\"\"");
    }
    #[test]
    fn cst() {
        s!("lit_string_with_escape_cst", t::cst(&"\"hello\\nworld\\\"\""), "\"hello\\nworld\\\"\"");
    }
    #[test]
    fn ast() {
        s!("lit_string_with_escape_ast", t::ast(&"\"hello\\nworld\\\"\""), "\"hello\\nworld\\\"\"");
    }
}
mod op_add_sub {
    use super::*;
    #[test]
    fn lex() {
        s!("op_add_sub_lex", t::lex(&"a + b - c"), "a + b - c");
    }
    #[test]
    fn cst() {
        s!("op_add_sub_cst", t::cst(&"a + b - c"), "a + b - c");
    }
    #[test]
    fn ast() {
        s!("op_add_sub_ast", t::ast(&"a + b - c"), "a + b - c");
    }
}
mod op_multi_add {
    use super::*;
    #[test]
    fn lex() {
        s!("op_multi_add_lex", t::lex(&"a + b + c"), "a + b + c");
    }
    #[test]
    fn cst() {
        s!("op_multi_add_cst", t::cst(&"a + b + c"), "a + b + c");
    }
    #[test]
    fn ast() {
        s!("op_multi_add_ast", t::ast(&"a + b + c"), "a + b + c");
    }
}
mod lit_int {
    use super::*;
    #[test]
    fn lex() {
        s!("lit_int_lex", t::lex(&"1234"), "1234");
    }
    #[test]
    fn cst() {
        s!("lit_int_cst", t::cst(&"1234"), "1234");
    }
    #[test]
    fn ast() {
        s!("lit_int_ast", t::ast(&"1234"), "1234");
    }
}
mod ap_op_or {
    use super::*;
    #[test]
    fn lex() {
        s!("ap_op_or_lex", t::lex(&"foo a || b"), "foo a || b");
    }
    #[test]
    fn cst() {
        s!("ap_op_or_cst", t::cst(&"foo a || b"), "foo a || b");
    }
    #[test]
    fn ast() {
        s!("ap_op_or_ast", t::ast(&"foo a || b"), "foo a || b");
    }
}
