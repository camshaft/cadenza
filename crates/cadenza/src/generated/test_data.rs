use crate::testing as t;
use insta::assert_snapshot as s;
mod arithmetic_basic {
    use super::*;
    #[test]
    fn repl() {
        s!("arithmetic_basic", t::repl("1 + 1\n2 * 3\n10 / 2\n5 - 3\n"));
    }
}
mod errors {
    use super::*;
    #[test]
    fn repl() {
        s!("errors", t::repl("1 + \"string\"\nundefined_var\n"));
    }
}
mod function_basic {
    use super::*;
    #[test]
    fn repl() {
        s!("function_basic", t::repl("fn add a b = a + b\nadd 3 4\nadd 10 20\n"));
    }
}
mod variables {
    use super::*;
    #[test]
    fn repl() {
        s!("variables", t::repl("let x = 42\nx\nlet y = 10\nx + y\n"));
    }
}
