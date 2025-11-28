use crate::testing as t;
use insta::assert_debug_snapshot as s;
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
