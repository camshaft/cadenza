use crate::testing as t;
use insta::assert_snapshot as s;
mod arithmetic_basic {
    use super::*;
    #[test]
    fn repl() {
        s!("arithmetic_basic", t::repl("1 + 1\n2 * 3\n10 / 2\n5 - 3\n"));
    }
}
mod closure {
    use super::*;
    #[test]
    fn repl() {
        s!("closure", t::repl("fn double x = x * 2\ndouble 5\ndouble 10\n"));
    }
}
mod comments {
    use super::*;
    #[test]
    fn repl() {
        s!("comments", t::repl("1 + 1 # Inline comment\n2 + 2\n"));
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
mod strings {
    use super::*;
    #[test]
    fn repl() {
        s!("strings", t::repl("\"Hello, World!\"\n\"Multiple\\nlines\"\n\"String with \\\"quotes\\\"\"\n"));
    }
}
mod variables {
    use super::*;
    #[test]
    fn repl() {
        s!("variables", t::repl("let x = 42\nx\nlet y = 10\nx + y\n"));
    }
}
