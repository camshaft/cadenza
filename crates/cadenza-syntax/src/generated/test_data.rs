use crate::testing as t;
use insta::assert_debug_snapshot as s;
mod array_trailing_comma {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "array_trailing_comma_lex",
            t::lex("[1, 2, 3,]\n"),
            "[1, 2, 3,]\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "array_trailing_comma_cst",
            t::cst("[1, 2, 3,]\n"),
            "[1, 2, 3,]\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "array_trailing_comma_ast",
            t::ast("[1, 2, 3,]\n"),
            "[1, 2, 3,]\n"
        );
    }
}
mod ap_op_add {
    use super::*;
    #[test]
    fn lex() {
        s!("ap_op_add_lex", t::lex("foo 1 + 2"), "foo 1 + 2");
    }
    #[test]
    fn cst() {
        s!("ap_op_add_cst", t::cst("foo 1 + 2"), "foo 1 + 2");
    }
    #[test]
    fn ast() {
        s!("ap_op_add_ast", t::ast("foo 1 + 2"), "foo 1 + 2");
    }
}
mod ident_underscore_only {
    use super::*;
    #[test]
    fn lex() {
        s!("ident_underscore_only_lex", t::lex("_\n"), "_\n");
    }
    #[test]
    fn cst() {
        s!("ident_underscore_only_cst", t::cst("_\n"), "_\n");
    }
    #[test]
    fn ast() {
        s!("ident_underscore_only_ast", t::ast("_\n"), "_\n");
    }
}
mod ap_simple {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "ap_simple_lex",
            t::lex("foo 1 2 3 baz bar"),
            "foo 1 2 3 baz bar"
        );
    }
    #[test]
    fn cst() {
        s!(
            "ap_simple_cst",
            t::cst("foo 1 2 3 baz bar"),
            "foo 1 2 3 baz bar"
        );
    }
    #[test]
    fn ast() {
        s!(
            "ap_simple_ast",
            t::ast("foo 1 2 3 baz bar"),
            "foo 1 2 3 baz bar"
        );
    }
}
mod op_try_chain {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "op_try_chain_lex",
            t::lex("foo x? y? z?\n"),
            "foo x? y? z?\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "op_try_chain_cst",
            t::cst("foo x? y? z?\n"),
            "foo x? y? z?\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "op_try_chain_ast",
            t::ast("foo x? y? z?\n"),
            "foo x? y? z?\n"
        );
    }
}
mod op_path_with_ap {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "op_path_with_ap_lex",
            t::lex("std::io::println x\n"),
            "std::io::println x\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "op_path_with_ap_cst",
            t::cst("std::io::println x\n"),
            "std::io::println x\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "op_path_with_ap_ast",
            t::ast("std::io::println x\n"),
            "std::io::println x\n"
        );
    }
}
mod record_in_application {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "record_in_application_lex",
            t::lex("foo 1 {\n  a = 1\n} 2\nbar\n"),
            "foo 1 {\n  a = 1\n} 2\nbar\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "record_in_application_cst",
            t::cst("foo 1 {\n  a = 1\n} 2\nbar\n"),
            "foo 1 {\n  a = 1\n} 2\nbar\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "record_in_application_ast",
            t::ast("foo 1 {\n  a = 1\n} 2\nbar\n"),
            "foo 1 {\n  a = 1\n} 2\nbar\n"
        );
    }
}
mod op_path {
    use super::*;
    #[test]
    fn lex() {
        s!("op_path_lex", t::lex("std::io::Read\n"), "std::io::Read\n");
    }
    #[test]
    fn cst() {
        s!("op_path_cst", t::cst("std::io::Read\n"), "std::io::Read\n");
    }
    #[test]
    fn ast() {
        s!("op_path_ast", t::ast("std::io::Read\n"), "std::io::Read\n");
    }
}
mod ident_leading_underscore_numbers {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "ident_leading_underscore_numbers_lex",
            t::lex("_123\n"),
            "_123\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "ident_leading_underscore_numbers_cst",
            t::cst("_123\n"),
            "_123\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "ident_leading_underscore_numbers_ast",
            t::ast("_123\n"),
            "_123\n"
        );
    }
}
mod unit_suffix_float {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "unit_suffix_float_lex",
            t::lex("25.4meter\n"),
            "25.4meter\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "unit_suffix_float_cst",
            t::cst("25.4meter\n"),
            "25.4meter\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "unit_suffix_float_ast",
            t::ast("25.4meter\n"),
            "25.4meter\n"
        );
    }
}
mod lit_empty_string {
    use super::*;
    #[test]
    fn lex() {
        s!("lit_empty_string_lex", t::lex("\"\""), "\"\"");
    }
    #[test]
    fn cst() {
        s!("lit_empty_string_cst", t::cst("\"\""), "\"\"");
    }
    #[test]
    fn ast() {
        s!("lit_empty_string_ast", t::ast("\"\""), "\"\"");
    }
}
mod lit_int {
    use super::*;
    #[test]
    fn lex() {
        s!("lit_int_lex", t::lex("1234"), "1234");
    }
    #[test]
    fn cst() {
        s!("lit_int_cst", t::cst("1234"), "1234");
    }
    #[test]
    fn ast() {
        s!("lit_int_ast", t::ast("1234"), "1234");
    }
}
mod ident_emoji_mixed {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "ident_emoji_mixed_lex",
            t::lex("hello🎉world\n"),
            "hello🎉world\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "ident_emoji_mixed_cst",
            t::cst("hello🎉world\n"),
            "hello🎉world\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "ident_emoji_mixed_ast",
            t::ast("hello🎉world\n"),
            "hello🎉world\n"
        );
    }
}
mod op_sub {
    use super::*;
    #[test]
    fn lex() {
        s!("op_sub_lex", t::lex("a - b"), "a - b");
    }
    #[test]
    fn cst() {
        s!("op_sub_cst", t::cst("a - b"), "a - b");
    }
    #[test]
    fn ast() {
        s!("op_sub_ast", t::ast("a - b"), "a - b");
    }
}
mod unit_suffix_int {
    use super::*;
    #[test]
    fn lex() {
        s!("unit_suffix_int_lex", t::lex("25inch\n"), "25inch\n");
    }
    #[test]
    fn cst() {
        s!("unit_suffix_int_cst", t::cst("25inch\n"), "25inch\n");
    }
    #[test]
    fn ast() {
        s!("unit_suffix_int_ast", t::ast("25inch\n"), "25inch\n");
    }
}
mod unit_suffix_multiple {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "unit_suffix_multiple_lex",
            t::lex("10meter 5inch 3.14foot\n"),
            "10meter 5inch 3.14foot\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "unit_suffix_multiple_cst",
            t::cst("10meter 5inch 3.14foot\n"),
            "10meter 5inch 3.14foot\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "unit_suffix_multiple_ast",
            t::ast("10meter 5inch 3.14foot\n"),
            "10meter 5inch 3.14foot\n"
        );
    }
}
mod record_field_indented {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "record_field_indented_lex",
            t::lex("{\n  a = 1,\n  b = 2,\n}\n"),
            "{\n  a = 1,\n  b = 2,\n}\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "record_field_indented_cst",
            t::cst("{\n  a = 1,\n  b = 2,\n}\n"),
            "{\n  a = 1,\n  b = 2,\n}\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "record_field_indented_ast",
            t::ast("{\n  a = 1,\n  b = 2,\n}\n"),
            "{\n  a = 1,\n  b = 2,\n}\n"
        );
    }
}
mod lit_multiline_string {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "lit_multiline_string_lex",
            t::lex("\"\nhello\nworld!\n\""),
            "\"\nhello\nworld!\n\""
        );
    }
    #[test]
    fn cst() {
        s!(
            "lit_multiline_string_cst",
            t::cst("\"\nhello\nworld!\n\""),
            "\"\nhello\nworld!\n\""
        );
    }
    #[test]
    fn ast() {
        s!(
            "lit_multiline_string_ast",
            t::ast("\"\nhello\nworld!\n\""),
            "\"\nhello\nworld!\n\""
        );
    }
}
mod let_decl {
    use super::*;
    #[test]
    fn lex() {
        s!("let_decl_lex", t::lex("let x"), "let x");
    }
    #[test]
    fn cst() {
        s!("let_decl_cst", t::cst("let x"), "let x");
    }
    #[test]
    fn ast() {
        s!("let_decl_ast", t::ast("let x"), "let x");
    }
}
mod op_spread_in_record {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "op_spread_in_record_lex",
            t::lex("{ ...a, b = 1 }\n"),
            "{ ...a, b = 1 }\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "op_spread_in_record_cst",
            t::cst("{ ...a, b = 1 }\n"),
            "{ ...a, b = 1 }\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "op_spread_in_record_ast",
            t::ast("{ ...a, b = 1 }\n"),
            "{ ...a, b = 1 }\n"
        );
    }
}
mod let_eq_lit {
    use super::*;
    #[test]
    fn lex() {
        s!("let_eq_lit_lex", t::lex("let x = 42"), "let x = 42");
    }
    #[test]
    fn cst() {
        s!("let_eq_lit_cst", t::cst("let x = 42"), "let x = 42");
    }
    #[test]
    fn ast() {
        s!("let_eq_lit_ast", t::ast("let x = 42"), "let x = 42");
    }
}
mod ident_underscore_trailing {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "ident_underscore_trailing_lex",
            t::lex("hello_\n"),
            "hello_\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "ident_underscore_trailing_cst",
            t::cst("hello_\n"),
            "hello_\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "ident_underscore_trailing_ast",
            t::ast("hello_\n"),
            "hello_\n"
        );
    }
}
mod array_multiline {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "array_multiline_lex",
            t::lex("[\n    1,\n    2,\n    3\n]\n"),
            "[\n    1,\n    2,\n    3\n]\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "array_multiline_cst",
            t::cst("[\n    1,\n    2,\n    3\n]\n"),
            "[\n    1,\n    2,\n    3\n]\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "array_multiline_ast",
            t::ast("[\n    1,\n    2,\n    3\n]\n"),
            "[\n    1,\n    2,\n    3\n]\n"
        );
    }
}
mod lit_float {
    use super::*;
    #[test]
    fn lex() {
        s!("lit_float_lex", t::lex("3.14"), "3.14");
    }
    #[test]
    fn cst() {
        s!("lit_float_cst", t::cst("3.14"), "3.14");
    }
    #[test]
    fn ast() {
        s!("lit_float_ast", t::ast("3.14"), "3.14");
    }
}
mod ap_nested {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "ap_nested_lex",
            t::lex("foo 1 (bar 2) 3 4"),
            "foo 1 (bar 2) 3 4"
        );
    }
    #[test]
    fn cst() {
        s!(
            "ap_nested_cst",
            t::cst("foo 1 (bar 2) 3 4"),
            "foo 1 (bar 2) 3 4"
        );
    }
    #[test]
    fn ast() {
        s!(
            "ap_nested_ast",
            t::ast("foo 1 (bar 2) 3 4"),
            "foo 1 (bar 2) 3 4"
        );
    }
}
mod record_field_nested {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "record_field_nested_lex",
            t::lex("{ a = { b = 1 } }\n"),
            "{ a = { b = 1 } }\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "record_field_nested_cst",
            t::cst("{ a = { b = 1 } }\n"),
            "{ a = { b = 1 } }\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "record_field_nested_ast",
            t::ast("{ a = { b = 1 } }\n"),
            "{ a = { b = 1 } }\n"
        );
    }
}
mod lit_string_with_escape {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "lit_string_with_escape_lex",
            t::lex("\"hello\\nworld\\\"\""),
            "\"hello\\nworld\\\"\""
        );
    }
    #[test]
    fn cst() {
        s!(
            "lit_string_with_escape_cst",
            t::cst("\"hello\\nworld\\\"\""),
            "\"hello\\nworld\\\"\""
        );
    }
    #[test]
    fn ast() {
        s!(
            "lit_string_with_escape_ast",
            t::ast("\"hello\\nworld\\\"\""),
            "\"hello\\nworld\\\"\""
        );
    }
}
mod array_comma_first {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "array_comma_first_lex",
            t::lex("[ 1\n, 2\n, 3]\n"),
            "[ 1\n, 2\n, 3]\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "array_comma_first_cst",
            t::cst("[ 1\n, 2\n, 3]\n"),
            "[ 1\n, 2\n, 3]\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "array_comma_first_ast",
            t::ast("[ 1\n, 2\n, 3]\n"),
            "[ 1\n, 2\n, 3]\n"
        );
    }
}
mod op_try_then_pipe {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "op_try_then_pipe_lex",
            t::lex("foo 1 |? |> bar\n"),
            "foo 1 |? |> bar\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "op_try_then_pipe_cst",
            t::cst("foo 1 |? |> bar\n"),
            "foo 1 |? |> bar\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "op_try_then_pipe_ast",
            t::ast("foo 1 |? |> bar\n"),
            "foo 1 |? |> bar\n"
        );
    }
}
mod block_simple {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "block_simple_lex",
            t::lex("let foo =\n    let bar = 1\n    let baz = 2\n    bar\n"),
            "let foo =\n    let bar = 1\n    let baz = 2\n    bar\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "block_simple_cst",
            t::cst("let foo =\n    let bar = 1\n    let baz = 2\n    bar\n"),
            "let foo =\n    let bar = 1\n    let baz = 2\n    bar\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "block_simple_ast",
            t::ast("let foo =\n    let bar = 1\n    let baz = 2\n    bar\n"),
            "let foo =\n    let bar = 1\n    let baz = 2\n    bar\n"
        );
    }
}
mod record_field_expr {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "record_field_expr_lex",
            t::lex("{ a = 2 + 2 }\n"),
            "{ a = 2 + 2 }\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "record_field_expr_cst",
            t::cst("{ a = 2 + 2 }\n"),
            "{ a = 2 + 2 }\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "record_field_expr_ast",
            t::ast("{ a = 2 + 2 }\n"),
            "{ a = 2 + 2 }\n"
        );
    }
}
mod let_op_or {
    use super::*;
    #[test]
    fn lex() {
        s!("let_op_or_lex", t::lex("let x = a || b"), "let x = a || b");
    }
    #[test]
    fn cst() {
        s!("let_op_or_cst", t::cst("let x = a || b"), "let x = a || b");
    }
    #[test]
    fn ast() {
        s!("let_op_or_ast", t::ast("let x = a || b"), "let x = a || b");
    }
}
mod op_add_sub {
    use super::*;
    #[test]
    fn lex() {
        s!("op_add_sub_lex", t::lex("a + b - c"), "a + b - c");
    }
    #[test]
    fn cst() {
        s!("op_add_sub_cst", t::cst("a + b - c"), "a + b - c");
    }
    #[test]
    fn ast() {
        s!("op_add_sub_ast", t::ast("a + b - c"), "a + b - c");
    }
}
mod lit_multi_line {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "lit_multi_line_lex",
            t::lex("let a = 1\nlet b = 2"),
            "let a = 1\nlet b = 2"
        );
    }
    #[test]
    fn cst() {
        s!(
            "lit_multi_line_cst",
            t::cst("let a = 1\nlet b = 2"),
            "let a = 1\nlet b = 2"
        );
    }
    #[test]
    fn ast() {
        s!(
            "lit_multi_line_ast",
            t::ast("let a = 1\nlet b = 2"),
            "let a = 1\nlet b = 2"
        );
    }
}
mod array_empty {
    use super::*;
    #[test]
    fn lex() {
        s!("array_empty_lex", t::lex("[]\n"), "[]\n");
    }
    #[test]
    fn cst() {
        s!("array_empty_cst", t::cst("[]\n"), "[]\n");
    }
    #[test]
    fn ast() {
        s!("array_empty_ast", t::ast("[]\n"), "[]\n");
    }
}
mod op_field_and_path {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "op_field_and_path_lex",
            t::lex("mod::Type.field\n"),
            "mod::Type.field\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "op_field_and_path_cst",
            t::cst("mod::Type.field\n"),
            "mod::Type.field\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "op_field_and_path_ast",
            t::ast("mod::Type.field\n"),
            "mod::Type.field\n"
        );
    }
}
mod op_pipe_try_chain {
    use super::*;
    #[test]
    fn lex() {
        s!("op_pipe_try_chain_lex", t::lex("x |? |?\n"), "x |? |?\n");
    }
    #[test]
    fn cst() {
        s!("op_pipe_try_chain_cst", t::cst("x |? |?\n"), "x |? |?\n");
    }
    #[test]
    fn ast() {
        s!("op_pipe_try_chain_ast", t::ast("x |? |?\n"), "x |? |?\n");
    }
}
mod record_shorthand {
    use super::*;
    #[test]
    fn lex() {
        s!("record_shorthand_lex", t::lex("{ x, y }\n"), "{ x, y }\n");
    }
    #[test]
    fn cst() {
        s!("record_shorthand_cst", t::cst("{ x, y }\n"), "{ x, y }\n");
    }
    #[test]
    fn ast() {
        s!("record_shorthand_ast", t::ast("{ x, y }\n"), "{ x, y }\n");
    }
}
mod array_with_ap {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "array_with_ap_lex",
            t::lex("foo [\n    a + b\n] c d e + f\n"),
            "foo [\n    a + b\n] c d e + f\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "array_with_ap_cst",
            t::cst("foo [\n    a + b\n] c d e + f\n"),
            "foo [\n    a + b\n] c d e + f\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "array_with_ap_ast",
            t::ast("foo [\n    a + b\n] c d e + f\n"),
            "foo [\n    a + b\n] c d e + f\n"
        );
    }
}
mod ident_emoji_multiple {
    use super::*;
    #[test]
    fn lex() {
        s!("ident_emoji_multiple_lex", t::lex("🎉🎊🎁\n"), "🎉🎊🎁\n");
    }
    #[test]
    fn cst() {
        s!("ident_emoji_multiple_cst", t::cst("🎉🎊🎁\n"), "🎉🎊🎁\n");
    }
    #[test]
    fn ast() {
        s!("ident_emoji_multiple_ast", t::ast("🎉🎊🎁\n"), "🎉🎊🎁\n");
    }
}
mod op_spread_simple {
    use super::*;
    #[test]
    fn lex() {
        s!("op_spread_simple_lex", t::lex("...a\n"), "...a\n");
    }
    #[test]
    fn cst() {
        s!("op_spread_simple_cst", t::cst("...a\n"), "...a\n");
    }
    #[test]
    fn ast() {
        s!("op_spread_simple_ast", t::ast("...a\n"), "...a\n");
    }
}
mod ap_op_or {
    use super::*;
    #[test]
    fn lex() {
        s!("ap_op_or_lex", t::lex("foo a || b"), "foo a || b");
    }
    #[test]
    fn cst() {
        s!("ap_op_or_cst", t::cst("foo a || b"), "foo a || b");
    }
    #[test]
    fn ast() {
        s!("ap_op_or_ast", t::ast("foo a || b"), "foo a || b");
    }
}
mod array_mixed_lines {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "array_mixed_lines_lex",
            t::lex("[1, 2,\n    3, 4,\n    5]\n"),
            "[1, 2,\n    3, 4,\n    5]\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "array_mixed_lines_cst",
            t::cst("[1, 2,\n    3, 4,\n    5]\n"),
            "[1, 2,\n    3, 4,\n    5]\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "array_mixed_lines_ast",
            t::ast("[1, 2,\n    3, 4,\n    5]\n"),
            "[1, 2,\n    3, 4,\n    5]\n"
        );
    }
}
mod op_mul_add {
    use super::*;
    #[test]
    fn lex() {
        s!("op_mul_add_lex", t::lex("a * b + c"), "a * b + c");
    }
    #[test]
    fn cst() {
        s!("op_mul_add_cst", t::cst("a * b + c"), "a * b + c");
    }
    #[test]
    fn ast() {
        s!("op_mul_add_ast", t::ast("a * b + c"), "a * b + c");
    }
}
mod ident_utf8_japanese {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "ident_utf8_japanese_lex",
            t::lex("こんにちは\n"),
            "こんにちは\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "ident_utf8_japanese_cst",
            t::cst("こんにちは\n"),
            "こんにちは\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "ident_utf8_japanese_ast",
            t::ast("こんにちは\n"),
            "こんにちは\n"
        );
    }
}
mod ident_utf8_greek {
    use super::*;
    #[test]
    fn lex() {
        s!("ident_utf8_greek_lex", t::lex("αβγ\n"), "αβγ\n");
    }
    #[test]
    fn cst() {
        s!("ident_utf8_greek_cst", t::cst("αβγ\n"), "αβγ\n");
    }
    #[test]
    fn ast() {
        s!("ident_utf8_greek_ast", t::ast("αβγ\n"), "αβγ\n");
    }
}
mod record_empty {
    use super::*;
    #[test]
    fn lex() {
        s!("record_empty_lex", t::lex("{}\n"), "{}\n");
    }
    #[test]
    fn cst() {
        s!("record_empty_cst", t::cst("{}\n"), "{}\n");
    }
    #[test]
    fn ast() {
        s!("record_empty_ast", t::ast("{}\n"), "{}\n");
    }
}
mod op_spread_in_list {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "op_spread_in_list_lex",
            t::lex("[...a, 1, 2]\n"),
            "[...a, 1, 2]\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "op_spread_in_list_cst",
            t::cst("[...a, 1, 2]\n"),
            "[...a, 1, 2]\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "op_spread_in_list_ast",
            t::ast("[...a, 1, 2]\n"),
            "[...a, 1, 2]\n"
        );
    }
}
mod ident_underscore_leading {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "ident_underscore_leading_lex",
            t::lex("_hello\n"),
            "_hello\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "ident_underscore_leading_cst",
            t::cst("_hello\n"),
            "_hello\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "ident_underscore_leading_ast",
            t::ast("_hello\n"),
            "_hello\n"
        );
    }
}
mod op_field_after_call {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "op_field_after_call_lex",
            t::lex("(get_point x).field\n"),
            "(get_point x).field\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "op_field_after_call_cst",
            t::cst("(get_point x).field\n"),
            "(get_point x).field\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "op_field_after_call_ast",
            t::ast("(get_point x).field\n"),
            "(get_point x).field\n"
        );
    }
}
mod ident_utf8_chinese {
    use super::*;
    #[test]
    fn lex() {
        s!("ident_utf8_chinese_lex", t::lex("你好\n"), "你好\n");
    }
    #[test]
    fn cst() {
        s!("ident_utf8_chinese_cst", t::cst("你好\n"), "你好\n");
    }
    #[test]
    fn ast() {
        s!("ident_utf8_chinese_ast", t::ast("你好\n"), "你好\n");
    }
}
mod array_simple {
    use super::*;
    #[test]
    fn lex() {
        s!("array_simple_lex", t::lex("[1, 2, 3]\n"), "[1, 2, 3]\n");
    }
    #[test]
    fn cst() {
        s!("array_simple_cst", t::cst("[1, 2, 3]\n"), "[1, 2, 3]\n");
    }
    #[test]
    fn ast() {
        s!("array_simple_ast", t::ast("[1, 2, 3]\n"), "[1, 2, 3]\n");
    }
}
mod ident_utf8_underscore {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "ident_utf8_underscore_lex",
            t::lex("hello_世界\n"),
            "hello_世界\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "ident_utf8_underscore_cst",
            t::cst("hello_世界\n"),
            "hello_世界\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "ident_utf8_underscore_ast",
            t::ast("hello_世界\n"),
            "hello_世界\n"
        );
    }
}
mod op_try_simple {
    use super::*;
    #[test]
    fn lex() {
        s!("op_try_simple_lex", t::lex("x?\n"), "x?\n");
    }
    #[test]
    fn cst() {
        s!("op_try_simple_cst", t::cst("x?\n"), "x?\n");
    }
    #[test]
    fn ast() {
        s!("op_try_simple_ast", t::ast("x?\n"), "x?\n");
    }
}
mod op_try_double {
    use super::*;
    #[test]
    fn lex() {
        s!("op_try_double_lex", t::lex("x??\n"), "x??\n");
    }
    #[test]
    fn cst() {
        s!("op_try_double_cst", t::cst("x??\n"), "x??\n");
    }
    #[test]
    fn ast() {
        s!("op_try_double_ast", t::ast("x??\n"), "x??\n");
    }
}
mod ws_block {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "ws_block_lex",
            t::lex("let foo = # comment here\n    let bar = 123 # other comment here\n    bar\n"),
            "let foo = # comment here\n    let bar = 123 # other comment here\n    bar\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "ws_block_cst",
            t::cst("let foo = # comment here\n    let bar = 123 # other comment here\n    bar\n"),
            "let foo = # comment here\n    let bar = 123 # other comment here\n    bar\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "ws_block_ast",
            t::ast("let foo = # comment here\n    let bar = 123 # other comment here\n    bar\n"),
            "let foo = # comment here\n    let bar = 123 # other comment here\n    bar\n"
        );
    }
}
mod ident_underscore_double {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "ident_underscore_double_lex",
            t::lex("hello__world\n"),
            "hello__world\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "ident_underscore_double_cst",
            t::cst("hello__world\n"),
            "hello__world\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "ident_underscore_double_ast",
            t::ast("hello__world\n"),
            "hello__world\n"
        );
    }
}
mod op_field_chained {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "op_field_chained_lex",
            t::lex("obj.field.subfield\n"),
            "obj.field.subfield\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "op_field_chained_cst",
            t::cst("obj.field.subfield\n"),
            "obj.field.subfield\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "op_field_chained_ast",
            t::ast("obj.field.subfield\n"),
            "obj.field.subfield\n"
        );
    }
}
mod record_field_comma_first {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "record_field_comma_first_lex",
            t::lex("{ a = 1\n, b = 2\n}\n"),
            "{ a = 1\n, b = 2\n}\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "record_field_comma_first_cst",
            t::cst("{ a = 1\n, b = 2\n}\n"),
            "{ a = 1\n, b = 2\n}\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "record_field_comma_first_ast",
            t::ast("{ a = 1\n, b = 2\n}\n"),
            "{ a = 1\n, b = 2\n}\n"
        );
    }
}
mod ident_underscore_with_numbers {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "ident_underscore_with_numbers_lex",
            t::lex("hello_world_123\n"),
            "hello_world_123\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "ident_underscore_with_numbers_cst",
            t::cst("hello_world_123\n"),
            "hello_world_123\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "ident_underscore_with_numbers_ast",
            t::ast("hello_world_123\n"),
            "hello_world_123\n"
        );
    }
}
mod ident_underscore_multiple {
    use super::*;
    #[test]
    fn lex() {
        s!("ident_underscore_multiple_lex", t::lex("___\n"), "___\n");
    }
    #[test]
    fn cst() {
        s!("ident_underscore_multiple_cst", t::cst("___\n"), "___\n");
    }
    #[test]
    fn ast() {
        s!("ident_underscore_multiple_ast", t::ast("___\n"), "___\n");
    }
}
mod array_with_exprs {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "array_with_exprs_lex",
            t::lex("[a + b, c * d]\n"),
            "[a + b, c * d]\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "array_with_exprs_cst",
            t::cst("[a + b, c * d]\n"),
            "[a + b, c * d]\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "array_with_exprs_ast",
            t::ast("[a + b, c * d]\n"),
            "[a + b, c * d]\n"
        );
    }
}
mod op_try_with_ap {
    use super::*;
    #[test]
    fn lex() {
        s!("op_try_with_ap_lex", t::lex("foo 1?\n"), "foo 1?\n");
    }
    #[test]
    fn cst() {
        s!("op_try_with_ap_cst", t::cst("foo 1?\n"), "foo 1?\n");
    }
    #[test]
    fn ast() {
        s!("op_try_with_ap_ast", t::ast("foo 1?\n"), "foo 1?\n");
    }
}
mod op_path_in_expr {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "op_path_in_expr_lex",
            t::lex("std::ops::Add + std::ops::Mul\n"),
            "std::ops::Add + std::ops::Mul\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "op_path_in_expr_cst",
            t::cst("std::ops::Add + std::ops::Mul\n"),
            "std::ops::Add + std::ops::Mul\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "op_path_in_expr_ast",
            t::ast("std::ops::Add + std::ops::Mul\n"),
            "std::ops::Add + std::ops::Mul\n"
        );
    }
}
mod lit_string {
    use super::*;
    #[test]
    fn lex() {
        s!("lit_string_lex", t::lex("\"hello\""), "\"hello\"");
    }
    #[test]
    fn cst() {
        s!("lit_string_cst", t::cst("\"hello\""), "\"hello\"");
    }
    #[test]
    fn ast() {
        s!("lit_string_ast", t::ast("\"hello\""), "\"hello\"");
    }
}
mod op_pipe_try_with_add {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "op_pipe_try_with_add_lex",
            t::lex("a + b |?\n"),
            "a + b |?\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "op_pipe_try_with_add_cst",
            t::cst("a + b |?\n"),
            "a + b |?\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "op_pipe_try_with_add_ast",
            t::ast("a + b |?\n"),
            "a + b |?\n"
        );
    }
}
mod op_pipe_then_try {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "op_pipe_then_try_lex",
            t::lex("foo 1 |> bar |?\n"),
            "foo 1 |> bar |?\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "op_pipe_then_try_cst",
            t::cst("foo 1 |> bar |?\n"),
            "foo 1 |> bar |?\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "op_pipe_then_try_ast",
            t::ast("foo 1 |> bar |?\n"),
            "foo 1 |> bar |?\n"
        );
    }
}
mod ident_utf8_mixed_greek {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "ident_utf8_mixed_greek_lex",
            t::lex("helloαworld\n"),
            "helloαworld\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "ident_utf8_mixed_greek_cst",
            t::cst("helloαworld\n"),
            "helloαworld\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "ident_utf8_mixed_greek_ast",
            t::ast("helloαworld\n"),
            "helloαworld\n"
        );
    }
}
mod op_path_with_field {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "op_path_with_field_lex",
            t::lex("module::Type.field\n"),
            "module::Type.field\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "op_path_with_field_cst",
            t::cst("module::Type.field\n"),
            "module::Type.field\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "op_path_with_field_ast",
            t::ast("module::Type.field\n"),
            "module::Type.field\n"
        );
    }
}
mod op_pipe_try_simple {
    use super::*;
    #[test]
    fn lex() {
        s!("op_pipe_try_simple_lex", t::lex("x |?\n"), "x |?\n");
    }
    #[test]
    fn cst() {
        s!("op_pipe_try_simple_cst", t::cst("x |?\n"), "x |?\n");
    }
    #[test]
    fn ast() {
        s!("op_pipe_try_simple_ast", t::ast("x |?\n"), "x |?\n");
    }
}
mod op_pipe {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "op_pipe_lex",
            t::lex("foo 1 + 2\n|> bar 3\n|> baz 4 * 5\n"),
            "foo 1 + 2\n|> bar 3\n|> baz 4 * 5\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "op_pipe_cst",
            t::cst("foo 1 + 2\n|> bar 3\n|> baz 4 * 5\n"),
            "foo 1 + 2\n|> bar 3\n|> baz 4 * 5\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "op_pipe_ast",
            t::ast("foo 1 + 2\n|> bar 3\n|> baz 4 * 5\n"),
            "foo 1 + 2\n|> bar 3\n|> baz 4 * 5\n"
        );
    }
}
mod op_pipe_try_with_ap {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "op_pipe_try_with_ap_lex",
            t::lex("foo 1 2 |?\n"),
            "foo 1 2 |?\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "op_pipe_try_with_ap_cst",
            t::cst("foo 1 2 |?\n"),
            "foo 1 2 |?\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "op_pipe_try_with_ap_ast",
            t::ast("foo 1 2 |?\n"),
            "foo 1 2 |?\n"
        );
    }
}
mod op_field_in_expr {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "op_field_in_expr_lex",
            t::lex("point.x + point.y\n"),
            "point.x + point.y\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "op_field_in_expr_cst",
            t::cst("point.x + point.y\n"),
            "point.x + point.y\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "op_field_in_expr_ast",
            t::ast("point.x + point.y\n"),
            "point.x + point.y\n"
        );
    }
}
mod ws_leading_newline {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "ws_leading_newline_lex",
            t::lex("\nfoo 123 456\n"),
            "\nfoo 123 456\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "ws_leading_newline_cst",
            t::cst("\nfoo 123 456\n"),
            "\nfoo 123 456\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "ws_leading_newline_ast",
            t::ast("\nfoo 123 456\n"),
            "\nfoo 123 456\n"
        );
    }
}
mod op_pipe_try_pipe {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "op_pipe_try_pipe_lex",
            t::lex("foo 1 |> bar 2 |? |> baz 3\n"),
            "foo 1 |> bar 2 |? |> baz 3\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "op_pipe_try_pipe_cst",
            t::cst("foo 1 |> bar 2 |? |> baz 3\n"),
            "foo 1 |> bar 2 |? |> baz 3\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "op_pipe_try_pipe_ast",
            t::ast("foo 1 |> bar 2 |? |> baz 3\n"),
            "foo 1 |> bar 2 |? |> baz 3\n"
        );
    }
}
mod ident_underscore_middle {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "ident_underscore_middle_lex",
            t::lex("hello_world\n"),
            "hello_world\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "ident_underscore_middle_cst",
            t::cst("hello_world\n"),
            "hello_world\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "ident_underscore_middle_ast",
            t::ast("hello_world\n"),
            "hello_world\n"
        );
    }
}
mod record_field_trailing_comma {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "record_field_trailing_comma_lex",
            t::lex("{ a = 1, b = 2, }\n"),
            "{ a = 1, b = 2, }\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "record_field_trailing_comma_cst",
            t::cst("{ a = 1, b = 2, }\n"),
            "{ a = 1, b = 2, }\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "record_field_trailing_comma_ast",
            t::ast("{ a = 1, b = 2, }\n"),
            "{ a = 1, b = 2, }\n"
        );
    }
}
mod op_try_after_add {
    use super::*;
    #[test]
    fn lex() {
        s!("op_try_after_add_lex", t::lex("a + b?\n"), "a + b?\n");
    }
    #[test]
    fn cst() {
        s!("op_try_after_add_cst", t::cst("a + b?\n"), "a + b?\n");
    }
    #[test]
    fn ast() {
        s!("op_try_after_add_ast", t::ast("a + b?\n"), "a + b?\n");
    }
}
mod op_try_with_ap_add {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "op_try_with_ap_add_lex",
            t::lex("let x = foo 1 + 2?\n"),
            "let x = foo 1 + 2?\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "op_try_with_ap_add_cst",
            t::cst("let x = foo 1 + 2?\n"),
            "let x = foo 1 + 2?\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "op_try_with_ap_add_ast",
            t::ast("let x = foo 1 + 2?\n"),
            "let x = foo 1 + 2?\n"
        );
    }
}
mod op_field_access {
    use super::*;
    #[test]
    fn lex() {
        s!("op_field_access_lex", t::lex("point.x\n"), "point.x\n");
    }
    #[test]
    fn cst() {
        s!("op_field_access_cst", t::cst("point.x\n"), "point.x\n");
    }
    #[test]
    fn ast() {
        s!("op_field_access_ast", t::ast("point.x\n"), "point.x\n");
    }
}
mod op_field_long {
    use super::*;
    #[test]
    fn lex() {
        s!("op_field_long_lex", t::lex("a.b.c.d.e\n"), "a.b.c.d.e\n");
    }
    #[test]
    fn cst() {
        s!("op_field_long_cst", t::cst("a.b.c.d.e\n"), "a.b.c.d.e\n");
    }
    #[test]
    fn ast() {
        s!("op_field_long_ast", t::ast("a.b.c.d.e\n"), "a.b.c.d.e\n");
    }
}
mod record_field_simple {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "record_field_simple_lex",
            t::lex("{ a = 1 }\n"),
            "{ a = 1 }\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "record_field_simple_cst",
            t::cst("{ a = 1 }\n"),
            "{ a = 1 }\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "record_field_simple_ast",
            t::ast("{ a = 1 }\n"),
            "{ a = 1 }\n"
        );
    }
}
mod op_path_simple {
    use super::*;
    #[test]
    fn lex() {
        s!("op_path_simple_lex", t::lex("std::io\n"), "std::io\n");
    }
    #[test]
    fn cst() {
        s!("op_path_simple_cst", t::cst("std::io\n"), "std::io\n");
    }
    #[test]
    fn ast() {
        s!("op_path_simple_ast", t::ast("std::io\n"), "std::io\n");
    }
}
mod ident_emoji_underscore {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "ident_emoji_underscore_lex",
            t::lex("hello_🎉_world\n"),
            "hello_🎉_world\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "ident_emoji_underscore_cst",
            t::cst("hello_🎉_world\n"),
            "hello_🎉_world\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "ident_emoji_underscore_ast",
            t::ast("hello_🎉_world\n"),
            "hello_🎉_world\n"
        );
    }
}
mod ident_emoji_single {
    use super::*;
    #[test]
    fn lex() {
        s!("ident_emoji_single_lex", t::lex("🎉\n"), "🎉\n");
    }
    #[test]
    fn cst() {
        s!("ident_emoji_single_cst", t::cst("🎉\n"), "🎉\n");
    }
    #[test]
    fn ast() {
        s!("ident_emoji_single_ast", t::ast("🎉\n"), "🎉\n");
    }
}
mod ap_single {
    use super::*;
    #[test]
    fn lex() {
        s!("ap_single_lex", t::lex("foo bar"), "foo bar");
    }
    #[test]
    fn cst() {
        s!("ap_single_cst", t::cst("foo bar"), "foo bar");
    }
    #[test]
    fn ast() {
        s!("ap_single_ast", t::ast("foo bar"), "foo bar");
    }
}
mod op_add_mul {
    use super::*;
    #[test]
    fn lex() {
        s!("op_add_mul_lex", t::lex("a + b * c"), "a + b * c");
    }
    #[test]
    fn cst() {
        s!("op_add_mul_cst", t::cst("a + b * c"), "a + b * c");
    }
    #[test]
    fn ast() {
        s!("op_add_mul_ast", t::ast("a + b * c"), "a + b * c");
    }
}
mod op_try_with_paren {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "op_try_with_paren_lex",
            t::lex("let x = (foo 1 + 2)?\n"),
            "let x = (foo 1 + 2)?\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "op_try_with_paren_cst",
            t::cst("let x = (foo 1 + 2)?\n"),
            "let x = (foo 1 + 2)?\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "op_try_with_paren_ast",
            t::ast("let x = (foo 1 + 2)?\n"),
            "let x = (foo 1 + 2)?\n"
        );
    }
}
mod op_field_with_ap {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "op_field_with_ap_lex",
            t::lex("list.map fn\n"),
            "list.map fn\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "op_field_with_ap_cst",
            t::cst("list.map fn\n"),
            "list.map fn\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "op_field_with_ap_ast",
            t::ast("list.map fn\n"),
            "list.map fn\n"
        );
    }
}
mod op_field_with_try {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "op_field_with_try_lex",
            t::lex("result.value?\n"),
            "result.value?\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "op_field_with_try_cst",
            t::cst("result.value?\n"),
            "result.value?\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "op_field_with_try_ast",
            t::ast("result.value?\n"),
            "result.value?\n"
        );
    }
}
mod op_multi_add {
    use super::*;
    #[test]
    fn lex() {
        s!("op_multi_add_lex", t::lex("a + b + c"), "a + b + c");
    }
    #[test]
    fn cst() {
        s!("op_multi_add_cst", t::cst("a + b + c"), "a + b + c");
    }
    #[test]
    fn ast() {
        s!("op_multi_add_ast", t::ast("a + b + c"), "a + b + c");
    }
}
mod op_path_long {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "op_path_long_lex",
            t::lex("a::b::c::d::e\n"),
            "a::b::c::d::e\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "op_path_long_cst",
            t::cst("a::b::c::d::e\n"),
            "a::b::c::d::e\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "op_path_long_ast",
            t::ast("a::b::c::d::e\n"),
            "a::b::c::d::e\n"
        );
    }
}
mod array_nested {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "array_nested_lex",
            t::lex("[[1, 2], [3, 4]]\n"),
            "[[1, 2], [3, 4]]\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "array_nested_cst",
            t::cst("[[1, 2], [3, 4]]\n"),
            "[[1, 2], [3, 4]]\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "array_nested_ast",
            t::ast("[[1, 2], [3, 4]]\n"),
            "[[1, 2], [3, 4]]\n"
        );
    }
}
mod ws_paren {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "ws_paren_lex",
            t::lex("foo (\n    a + b\n) c d e + f\n"),
            "foo (\n    a + b\n) c d e + f\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "ws_paren_cst",
            t::cst("foo (\n    a + b\n) c d e + f\n"),
            "foo (\n    a + b\n) c d e + f\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "ws_paren_ast",
            t::ast("foo (\n    a + b\n) c d e + f\n"),
            "foo (\n    a + b\n) c d e + f\n"
        );
    }
}
mod array_single {
    use super::*;
    #[test]
    fn lex() {
        s!("array_single_lex", t::lex("[1]\n"), "[1]\n");
    }
    #[test]
    fn cst() {
        s!("array_single_cst", t::cst("[1]\n"), "[1]\n");
    }
    #[test]
    fn ast() {
        s!("array_single_ast", t::ast("[1]\n"), "[1]\n");
    }
}
mod op_add {
    use super::*;
    #[test]
    fn lex() {
        s!("op_add_lex", t::lex("a + b\n"), "a + b\n");
    }
    #[test]
    fn cst() {
        s!("op_add_cst", t::cst("a + b\n"), "a + b\n");
    }
    #[test]
    fn ast() {
        s!("op_add_ast", t::ast("a + b\n"), "a + b\n");
    }
}
mod measure_with_equals {
    use super::*;
    #[test]
    fn lex() {
        s!(
            "measure_with_equals_lex",
            t::lex("measure foot = inch 12\n"),
            "measure foot = inch 12\n"
        );
    }
    #[test]
    fn cst() {
        s!(
            "measure_with_equals_cst",
            t::cst("measure foot = inch 12\n"),
            "measure foot = inch 12\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "measure_with_equals_ast",
            t::ast("measure foot = inch 12\n"),
            "measure foot = inch 12\n"
        );
    }
}
mod invalid_parse {
    use super::*;
    mod array_sparse_entries {
        use super::*;
        #[test]
        fn cst() {
            s!(
                "invalid_parse_array_sparse_entries_cst",
                t::cst_no_assert("[\n a\n,,c]\n"),
                "[\n a\n,,c]\n"
            );
        }
        #[test]
        fn ast() {
            s!(
                "invalid_parse_array_sparse_entries_ast",
                t::ast_no_assert("[\n a\n,,c]\n"),
                "[\n a\n,,c]\n"
            );
        }
        #[test]
        fn lex() {
            s!(
                "invalid_parse_array_sparse_entries_lex",
                t::lex("[\n a\n,,c]\n"),
                "[\n a\n,,c]\n"
            );
        }
        #[test]
        fn errors() {
            let errors = t::parse_errors("[\n a\n,,c]\n");
            assert!(
                !errors.is_empty(),
                "expected parse errors for invalid input"
            );
            s!(
                "invalid_parse_array_sparse_entries_errors",
                errors,
                "[\n a\n,,c]\n"
            );
        }
    }
    mod record_missing_brace_simple {
        use super::*;
        #[test]
        fn cst() {
            s!(
                "invalid_parse_record_missing_brace_simple_cst",
                t::cst_no_assert("{ a = 1\nfoo\n"),
                "{ a = 1\nfoo\n"
            );
        }
        #[test]
        fn ast() {
            s!(
                "invalid_parse_record_missing_brace_simple_ast",
                t::ast_no_assert("{ a = 1\nfoo\n"),
                "{ a = 1\nfoo\n"
            );
        }
        #[test]
        fn lex() {
            s!(
                "invalid_parse_record_missing_brace_simple_lex",
                t::lex("{ a = 1\nfoo\n"),
                "{ a = 1\nfoo\n"
            );
        }
        #[test]
        fn errors() {
            let errors = t::parse_errors("{ a = 1\nfoo\n");
            assert!(
                !errors.is_empty(),
                "expected parse errors for invalid input"
            );
            s!(
                "invalid_parse_record_missing_brace_simple_errors",
                errors,
                "{ a = 1\nfoo\n"
            );
        }
    }
    mod array_multiple_commas {
        use super::*;
        #[test]
        fn cst() {
            s!(
                "invalid_parse_array_multiple_commas_cst",
                t::cst_no_assert("[,,,]\n"),
                "[,,,]\n"
            );
        }
        #[test]
        fn ast() {
            s!(
                "invalid_parse_array_multiple_commas_ast",
                t::ast_no_assert("[,,,]\n"),
                "[,,,]\n"
            );
        }
        #[test]
        fn lex() {
            s!(
                "invalid_parse_array_multiple_commas_lex",
                t::lex("[,,,]\n"),
                "[,,,]\n"
            );
        }
        #[test]
        fn errors() {
            let errors = t::parse_errors("[,,,]\n");
            assert!(
                !errors.is_empty(),
                "expected parse errors for invalid input"
            );
            s!(
                "invalid_parse_array_multiple_commas_errors",
                errors,
                "[,,,]\n"
            );
        }
    }
    mod array_dedent_recovery {
        use super::*;
        #[test]
        fn cst() {
            s!(
                "invalid_parse_array_dedent_recovery_cst",
                t::cst_no_assert("foo [\nbar\n"),
                "foo [\nbar\n"
            );
        }
        #[test]
        fn ast() {
            s!(
                "invalid_parse_array_dedent_recovery_ast",
                t::ast_no_assert("foo [\nbar\n"),
                "foo [\nbar\n"
            );
        }
        #[test]
        fn lex() {
            s!(
                "invalid_parse_array_dedent_recovery_lex",
                t::lex("foo [\nbar\n"),
                "foo [\nbar\n"
            );
        }
        #[test]
        fn errors() {
            let errors = t::parse_errors("foo [\nbar\n");
            assert!(
                !errors.is_empty(),
                "expected parse errors for invalid input"
            );
            s!(
                "invalid_parse_array_dedent_recovery_errors",
                errors,
                "foo [\nbar\n"
            );
        }
    }
    mod record_double_comma {
        use super::*;
        #[test]
        fn cst() {
            s!(
                "invalid_parse_record_double_comma_cst",
                t::cst_no_assert("{ a = 1,, b = 2 }\n"),
                "{ a = 1,, b = 2 }\n"
            );
        }
        #[test]
        fn ast() {
            s!(
                "invalid_parse_record_double_comma_ast",
                t::ast_no_assert("{ a = 1,, b = 2 }\n"),
                "{ a = 1,, b = 2 }\n"
            );
        }
        #[test]
        fn lex() {
            s!(
                "invalid_parse_record_double_comma_lex",
                t::lex("{ a = 1,, b = 2 }\n"),
                "{ a = 1,, b = 2 }\n"
            );
        }
        #[test]
        fn errors() {
            let errors = t::parse_errors("{ a = 1,, b = 2 }\n");
            assert!(
                !errors.is_empty(),
                "expected parse errors for invalid input"
            );
            s!(
                "invalid_parse_record_double_comma_errors",
                errors,
                "{ a = 1,, b = 2 }\n"
            );
        }
    }
    mod record_missing_brace_comma {
        use super::*;
        #[test]
        fn cst() {
            s!(
                "invalid_parse_record_missing_brace_comma_cst",
                t::cst_no_assert("{ a = 1,\nfoo\n"),
                "{ a = 1,\nfoo\n"
            );
        }
        #[test]
        fn ast() {
            s!(
                "invalid_parse_record_missing_brace_comma_ast",
                t::ast_no_assert("{ a = 1,\nfoo\n"),
                "{ a = 1,\nfoo\n"
            );
        }
        #[test]
        fn lex() {
            s!(
                "invalid_parse_record_missing_brace_comma_lex",
                t::lex("{ a = 1,\nfoo\n"),
                "{ a = 1,\nfoo\n"
            );
        }
        #[test]
        fn errors() {
            let errors = t::parse_errors("{ a = 1,\nfoo\n");
            assert!(
                !errors.is_empty(),
                "expected parse errors for invalid input"
            );
            s!(
                "invalid_parse_record_missing_brace_comma_errors",
                errors,
                "{ a = 1,\nfoo\n"
            );
        }
    }
    mod array_leading_comma {
        use super::*;
        #[test]
        fn cst() {
            s!(
                "invalid_parse_array_leading_comma_cst",
                t::cst_no_assert("[, a, b]\n"),
                "[, a, b]\n"
            );
        }
        #[test]
        fn ast() {
            s!(
                "invalid_parse_array_leading_comma_ast",
                t::ast_no_assert("[, a, b]\n"),
                "[, a, b]\n"
            );
        }
        #[test]
        fn lex() {
            s!(
                "invalid_parse_array_leading_comma_lex",
                t::lex("[, a, b]\n"),
                "[, a, b]\n"
            );
        }
        #[test]
        fn errors() {
            let errors = t::parse_errors("[, a, b]\n");
            assert!(
                !errors.is_empty(),
                "expected parse errors for invalid input"
            );
            s!(
                "invalid_parse_array_leading_comma_errors",
                errors,
                "[, a, b]\n"
            );
        }
    }
    mod record_missing_brace_indented {
        use super::*;
        #[test]
        fn cst() {
            s!(
                "invalid_parse_record_missing_brace_indented_cst",
                t::cst_no_assert("{\n  a = 1\nfoo\n"),
                "{\n  a = 1\nfoo\n"
            );
        }
        #[test]
        fn ast() {
            s!(
                "invalid_parse_record_missing_brace_indented_ast",
                t::ast_no_assert("{\n  a = 1\nfoo\n"),
                "{\n  a = 1\nfoo\n"
            );
        }
        #[test]
        fn lex() {
            s!(
                "invalid_parse_record_missing_brace_indented_lex",
                t::lex("{\n  a = 1\nfoo\n"),
                "{\n  a = 1\nfoo\n"
            );
        }
        #[test]
        fn errors() {
            let errors = t::parse_errors("{\n  a = 1\nfoo\n");
            assert!(
                !errors.is_empty(),
                "expected parse errors for invalid input"
            );
            s!(
                "invalid_parse_record_missing_brace_indented_errors",
                errors,
                "{\n  a = 1\nfoo\n"
            );
        }
    }
}
