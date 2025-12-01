use crate::testing as t;
use insta::assert_debug_snapshot as s;
mod lit_int {
    use super::*;
    #[test]
    fn eval() {
        s!("lit_int", t::eval_all("42\n"), "42\n");
    }
}
mod error_divzero {
    use super::*;
    #[test]
    fn eval() {
        s!("error_divzero", t::eval_all("1 / 0\n"), "1 / 0\n");
    }
}
mod cmp_eq {
    use super::*;
    #[test]
    fn eval() {
        s!("cmp_eq", t::eval_all("1 == 1\n"), "1 == 1\n");
    }
}
mod lit_float {
    use super::*;
    #[test]
    fn eval() {
        s!("lit_float", t::eval_all("3.14\n"), "3.14\n");
    }
}
mod arith_mul {
    use super::*;
    #[test]
    fn eval() {
        s!("arith_mul", t::eval_all("4 * 5\n"), "4 * 5\n");
    }
}
mod arith_precedence {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "arith_precedence",
            t::eval_all("2 + 3 * 4\n"),
            "2 + 3 * 4\n"
        );
    }
}
mod arith_add {
    use super::*;
    #[test]
    fn eval() {
        s!("arith_add", t::eval_all("1 + 2\n"), "1 + 2\n");
    }
}
mod cmp_lt {
    use super::*;
    #[test]
    fn eval() {
        s!("cmp_lt", t::eval_all("1 < 2\n"), "1 < 2\n");
    }
}
mod let_simple {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "let_simple",
            t::eval_all("let x = 42\nx\n"),
            "let x = 42\nx\n"
        );
    }
}
mod arith_sub {
    use super::*;
    #[test]
    fn eval() {
        s!("arith_sub", t::eval_all("10 - 3\n"), "10 - 3\n");
    }
}
mod error_undefined {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "error_undefined",
            t::eval_all("undefined_var\n"),
            "undefined_var\n"
        );
    }
}
mod lit_string {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "lit_string",
            t::eval_all("\"hello world\"\n"),
            "\"hello world\"\n"
        );
    }
}
mod arith_mixed {
    use super::*;
    #[test]
    fn eval() {
        s!("arith_mixed", t::eval_all("1 + 2.5\n"), "1 + 2.5\n");
    }
}
mod let_multi {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "let_multi",
            t::eval_all("let x = 1\nlet y = 2\nx + y\n"),
            "let x = 1\nlet y = 2\nx + y\n"
        );
    }
}
mod arith_float {
    use super::*;
    #[test]
    fn eval() {
        s!("arith_float", t::eval_all("1.5 + 2.5\n"), "1.5 + 2.5\n");
    }
}
mod arith_div {
    use super::*;
    #[test]
    fn eval() {
        s!("arith_div", t::eval_all("20 / 4\n"), "20 / 4\n");
    }
}
mod multi_expr {
    use super::*;
    #[test]
    fn eval() {
        s!("multi_expr", t::eval_all("1\n2\n3\n"), "1\n2\n3\n");
    }
}
