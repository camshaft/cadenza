use crate::testing as t;
use insta::assert_debug_snapshot as s;
mod record_simple {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "record_simple",
            t::eval_all("{ a = 1, b = 2 }\n"),
            "{ a = 1, b = 2 }\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "record_simple_ast",
            t::ast("{ a = 1, b = 2 }\n"),
            "{ a = 1, b = 2 }\n"
        );
    }
}
mod cmp_gt {
    use super::*;
    #[test]
    fn eval() {
        s!("cmp_gt", t::eval_all("2 > 1\n"), "2 > 1\n");
    }
    #[test]
    fn ast() {
        s!("cmp_gt_ast", t::ast("2 > 1\n"), "2 > 1\n");
    }
}
mod fn_basic {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "fn_basic",
            t::eval_all("fn add x y = x + y\nadd 3 5\n"),
            "fn add x y = x + y\nadd 3 5\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "fn_basic_ast",
            t::ast("fn add x y = x + y\nadd 3 5\n"),
            "fn add x y = x + y\nadd 3 5\n"
        );
    }
}
mod fn_auto_apply {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "fn_auto_apply",
            t::eval_all("fn add x y = x + y\nadd\n"),
            "fn add x y = x + y\nadd\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "fn_auto_apply_ast",
            t::ast("fn add x y = x + y\nadd\n"),
            "fn add x y = x + y\nadd\n"
        );
    }
}
mod lit_int {
    use super::*;
    #[test]
    fn eval() {
        s!("lit_int", t::eval_all("42\n"), "42\n");
    }
    #[test]
    fn ast() {
        s!("lit_int_ast", t::ast("42\n"), "42\n");
    }
}
mod error_divzero {
    use super::*;
    #[test]
    fn eval() {
        s!("error_divzero", t::eval_all("1 / 0\n"), "1 / 0\n");
    }
    #[test]
    fn ast() {
        s!("error_divzero_ast", t::ast("1 / 0\n"), "1 / 0\n");
    }
}
mod cmp_eq {
    use super::*;
    #[test]
    fn eval() {
        s!("cmp_eq", t::eval_all("1 == 1\n"), "1 == 1\n");
    }
    #[test]
    fn ast() {
        s!("cmp_eq_ast", t::ast("1 == 1\n"), "1 == 1\n");
    }
}
mod measure_incompatible {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "measure_incompatible",
            t::eval_all(
                "measure meter\nmeasure second\nlet distance = meter 100\nlet time = second 10\ndistance + time\n"
            ),
            "measure meter\nmeasure second\nlet distance = meter 100\nlet time = second 10\ndistance + time\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "measure_incompatible_ast",
            t::ast(
                "measure meter\nmeasure second\nlet distance = meter 100\nlet time = second 10\ndistance + time\n"
            ),
            "measure meter\nmeasure second\nlet distance = meter 100\nlet time = second 10\ndistance + time\n"
        );
    }
}
mod example_08_lists {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "example_08_lists",
            t::eval_all(
                "# Lists\n# Collection of values in square brackets\n\n# Empty list\n[]\n\n# Simple list of integers\n[1, 2, 3, 4, 5]\n\n# Assign to variable\nlet numbers = [10, 20, 30]\nnumbers\n\n# List with expressions\nlet x = 5\nlet y = 10\n[x, y, x + y, x * y]\n\n# Nested lists\n[[1, 2], [3, 4], [5, 6]]\n\n# List with computed values\nlet a = 100\nlet b = 200\n[[a, a * 2], [b, b / 2]]\n"
            ),
            "# Lists\n# Collection of values in square brackets\n\n# Empty list\n[]\n\n# Simple list of integers\n[1, 2, 3, 4, 5]\n\n# Assign to variable\nlet numbers = [10, 20, 30]\nnumbers\n\n# List with expressions\nlet x = 5\nlet y = 10\n[x, y, x + y, x * y]\n\n# Nested lists\n[[1, 2], [3, 4], [5, 6]]\n\n# List with computed values\nlet a = 100\nlet b = 200\n[[a, a * 2], [b, b / 2]]\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "example_08_lists_ast",
            t::ast(
                "# Lists\n# Collection of values in square brackets\n\n# Empty list\n[]\n\n# Simple list of integers\n[1, 2, 3, 4, 5]\n\n# Assign to variable\nlet numbers = [10, 20, 30]\nnumbers\n\n# List with expressions\nlet x = 5\nlet y = 10\n[x, y, x + y, x * y]\n\n# Nested lists\n[[1, 2], [3, 4], [5, 6]]\n\n# List with computed values\nlet a = 100\nlet b = 200\n[[a, a * 2], [b, b / 2]]\n"
            ),
            "# Lists\n# Collection of values in square brackets\n\n# Empty list\n[]\n\n# Simple list of integers\n[1, 2, 3, 4, 5]\n\n# Assign to variable\nlet numbers = [10, 20, 30]\nnumbers\n\n# List with expressions\nlet x = 5\nlet y = 10\n[x, y, x + y, x * y]\n\n# Nested lists\n[[1, 2], [3, 4], [5, 6]]\n\n# List with computed values\nlet a = 100\nlet b = 200\n[[a, a * 2], [b, b / 2]]\n"
        );
    }
}
mod lit_float {
    use super::*;
    #[test]
    fn eval() {
        s!("lit_float", t::eval_all("3.14\n"), "3.14\n");
    }
    #[test]
    fn ast() {
        s!("lit_float_ast", t::ast("3.14\n"), "3.14\n");
    }
}
mod example_03_arithmetic {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "example_03_arithmetic",
            t::eval_all(
                "# Arithmetic Operations\n# Basic math with integers and floats\n\n# Addition and subtraction\n1 + 2\n10 - 3\n\n# Multiplication and division\n4 * 5\n20 / 4\n\n# Operator precedence\n2 + 3 * 4\n(2 + 3) * 4\n\n# Floating point\n3.14 * 2.0\n10.5 / 2.0\n"
            ),
            "# Arithmetic Operations\n# Basic math with integers and floats\n\n# Addition and subtraction\n1 + 2\n10 - 3\n\n# Multiplication and division\n4 * 5\n20 / 4\n\n# Operator precedence\n2 + 3 * 4\n(2 + 3) * 4\n\n# Floating point\n3.14 * 2.0\n10.5 / 2.0\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "example_03_arithmetic_ast",
            t::ast(
                "# Arithmetic Operations\n# Basic math with integers and floats\n\n# Addition and subtraction\n1 + 2\n10 - 3\n\n# Multiplication and division\n4 * 5\n20 / 4\n\n# Operator precedence\n2 + 3 * 4\n(2 + 3) * 4\n\n# Floating point\n3.14 * 2.0\n10.5 / 2.0\n"
            ),
            "# Arithmetic Operations\n# Basic math with integers and floats\n\n# Addition and subtraction\n1 + 2\n10 - 3\n\n# Multiplication and division\n4 * 5\n20 / 4\n\n# Operator precedence\n2 + 3 * 4\n(2 + 3) * 4\n\n# Floating point\n3.14 * 2.0\n10.5 / 2.0\n"
        );
    }
}
mod measure_unit_arithmetic {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "measure_unit_arithmetic",
            t::eval_all(
                "measure millimeter\nmeasure meter = millimeter 1000\nlet x = millimeter 500\nlet y = meter 1\nlet sum = x + y\nsum\n"
            ),
            "measure millimeter\nmeasure meter = millimeter 1000\nlet x = millimeter 500\nlet y = meter 1\nlet sum = x + y\nsum\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "measure_unit_arithmetic_ast",
            t::ast(
                "measure millimeter\nmeasure meter = millimeter 1000\nlet x = millimeter 500\nlet y = meter 1\nlet sum = x + y\nsum\n"
            ),
            "measure millimeter\nmeasure meter = millimeter 1000\nlet x = millimeter 500\nlet y = meter 1\nlet sum = x + y\nsum\n"
        );
    }
}
mod error_cmp_type_mismatch_ne {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "error_cmp_type_mismatch_ne",
            t::eval_all("# Test that != errors on type mismatch\n42 != \"world\"\n"),
            "# Test that != errors on type mismatch\n42 != \"world\"\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "error_cmp_type_mismatch_ne_ast",
            t::ast("# Test that != errors on type mismatch\n42 != \"world\"\n"),
            "# Test that != errors on type mismatch\n42 != \"world\"\n"
        );
    }
}
mod arith_mul {
    use super::*;
    #[test]
    fn eval() {
        s!("arith_mul", t::eval_all("4 * 5\n"), "4 * 5\n");
    }
    #[test]
    fn ast() {
        s!("arith_mul_ast", t::ast("4 * 5\n"), "4 * 5\n");
    }
}
mod block_simple {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "block_simple",
            t::eval_all("let foo =\n    let bar = 1\n    let baz = 2\n    bar\nfoo\n"),
            "let foo =\n    let bar = 1\n    let baz = 2\n    bar\nfoo\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "block_simple_ast",
            t::ast("let foo =\n    let bar = 1\n    let baz = 2\n    bar\nfoo\n"),
            "let foo =\n    let bar = 1\n    let baz = 2\n    bar\nfoo\n"
        );
    }
}
mod record_with_variables {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "record_with_variables",
            t::eval_all("let x = 1\nlet y = 2\n{ a = x, b = y }\n"),
            "let x = 1\nlet y = 2\n{ a = x, b = y }\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "record_with_variables_ast",
            t::ast("let x = 1\nlet y = 2\n{ a = x, b = y }\n"),
            "let x = 1\nlet y = 2\n{ a = x, b = y }\n"
        );
    }
}
mod error_let_invalid {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "error_let_invalid",
            t::eval_all("let 42 = 1\n"),
            "let 42 = 1\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "error_let_invalid_ast",
            t::ast("let 42 = 1\n"),
            "let 42 = 1\n"
        );
    }
}
mod measure_quantity {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "measure_quantity",
            t::eval_all("measure meter\nlet x = meter 5\nx\n"),
            "measure meter\nlet x = meter 5\nx\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "measure_quantity_ast",
            t::ast("measure meter\nlet x = meter 5\nx\n"),
            "measure meter\nlet x = meter 5\nx\n"
        );
    }
}
mod block_scope {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "block_scope",
            t::eval_all(
                "let outer = 100\nlet result =\n    let inner = 200\n    inner + outer\nresult\n"
            ),
            "let outer = 100\nlet result =\n    let inner = 200\n    inner + outer\nresult\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "block_scope_ast",
            t::ast(
                "let outer = 100\nlet result =\n    let inner = 200\n    inner + outer\nresult\n"
            ),
            "let outer = 100\nlet result =\n    let inner = 200\n    inner + outer\nresult\n"
        );
    }
}
mod measure_base {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "measure_base",
            t::eval_all("measure meter\n"),
            "measure meter\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "measure_base_ast",
            t::ast("measure meter\n"),
            "measure meter\n"
        );
    }
}
mod error_cmp_type_mismatch_lt {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "error_cmp_type_mismatch_lt",
            t::eval_all("# Test that < errors on non-numeric types\n\"foo\" < 5\n"),
            "# Test that < errors on non-numeric types\n\"foo\" < 5\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "error_cmp_type_mismatch_lt_ast",
            t::ast("# Test that < errors on non-numeric types\n\"foo\" < 5\n"),
            "# Test that < errors on non-numeric types\n\"foo\" < 5\n"
        );
    }
}
mod arith_float_mul {
    use super::*;
    #[test]
    fn eval() {
        s!("arith_float_mul", t::eval_all("3.0 * 2.0\n"), "3.0 * 2.0\n");
    }
    #[test]
    fn ast() {
        s!("arith_float_mul_ast", t::ast("3.0 * 2.0\n"), "3.0 * 2.0\n");
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
    #[test]
    fn ast() {
        s!("arith_precedence_ast", t::ast("2 + 3 * 4\n"), "2 + 3 * 4\n");
    }
}
mod cmp_le {
    use super::*;
    #[test]
    fn eval() {
        s!("cmp_le", t::eval_all("1 <= 1\n"), "1 <= 1\n");
    }
    #[test]
    fn ast() {
        s!("cmp_le_ast", t::ast("1 <= 1\n"), "1 <= 1\n");
    }
}
mod block_function_body {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "block_function_body",
            t::eval_all(
                "fn foo a b =\n    let av = a * 2\n    let bv = b * 3\n    av * bv\nfoo 5 7\n"
            ),
            "fn foo a b =\n    let av = a * 2\n    let bv = b * 3\n    av * bv\nfoo 5 7\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "block_function_body_ast",
            t::ast("fn foo a b =\n    let av = a * 2\n    let bv = b * 3\n    av * bv\nfoo 5 7\n"),
            "fn foo a b =\n    let av = a * 2\n    let bv = b * 3\n    av * bv\nfoo 5 7\n"
        );
    }
}
mod record_shorthand {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "record_shorthand",
            t::eval_all("let x = 1\nlet y = 2\n{ x, y }\n"),
            "let x = 1\nlet y = 2\n{ x, y }\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "record_shorthand_ast",
            t::ast("let x = 1\nlet y = 2\n{ x, y }\n"),
            "let x = 1\nlet y = 2\n{ x, y }\n"
        );
    }
}
mod arith_add {
    use super::*;
    #[test]
    fn eval() {
        s!("arith_add", t::eval_all("1 + 2\n"), "1 + 2\n");
    }
    #[test]
    fn ast() {
        s!("arith_add_ast", t::ast("1 + 2\n"), "1 + 2\n");
    }
}
mod cmp_ge {
    use super::*;
    #[test]
    fn eval() {
        s!("cmp_ge", t::eval_all("1 >= 1\n"), "1 >= 1\n");
    }
    #[test]
    fn ast() {
        s!("cmp_ge_ast", t::ast("1 >= 1\n"), "1 >= 1\n");
    }
}
mod record_empty {
    use super::*;
    #[test]
    fn eval() {
        s!("record_empty", t::eval_all("{}\n"), "{}\n");
    }
    #[test]
    fn ast() {
        s!("record_empty_ast", t::ast("{}\n"), "{}\n");
    }
}
mod example_02_literals {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "example_02_literals",
            t::eval_all(
                "# Literal Values\n# Different types of literals\n\n# Integers\n42\n-17\n0\n\n# Floating point\n3.14159\n-2.5\n1.0\n\n# Strings\n\"hello\"\n\"world\"\n\"hello world\"\n"
            ),
            "# Literal Values\n# Different types of literals\n\n# Integers\n42\n-17\n0\n\n# Floating point\n3.14159\n-2.5\n1.0\n\n# Strings\n\"hello\"\n\"world\"\n\"hello world\"\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "example_02_literals_ast",
            t::ast(
                "# Literal Values\n# Different types of literals\n\n# Integers\n42\n-17\n0\n\n# Floating point\n3.14159\n-2.5\n1.0\n\n# Strings\n\"hello\"\n\"world\"\n\"hello world\"\n"
            ),
            "# Literal Values\n# Different types of literals\n\n# Integers\n42\n-17\n0\n\n# Floating point\n3.14159\n-2.5\n1.0\n\n# Strings\n\"hello\"\n\"world\"\n\"hello world\"\n"
        );
    }
}
mod error_cmp_type_mismatch_gt {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "error_cmp_type_mismatch_gt",
            t::eval_all("# Test that > errors on type mismatch\n100 > \"baz\"\n"),
            "# Test that > errors on type mismatch\n100 > \"baz\"\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "error_cmp_type_mismatch_gt_ast",
            t::ast("# Test that > errors on type mismatch\n100 > \"baz\"\n"),
            "# Test that > errors on type mismatch\n100 > \"baz\"\n"
        );
    }
}
mod arith_left_assoc {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "arith_left_assoc",
            t::eval_all("10 - 5 - 2\n"),
            "10 - 5 - 2\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "arith_left_assoc_ast",
            t::ast("10 - 5 - 2\n"),
            "10 - 5 - 2\n"
        );
    }
}
mod fn_closure {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "fn_closure",
            t::eval_all("let x = 10\nfn capture_fn = x\nlet x = 20\ncapture_fn\n"),
            "let x = 10\nfn capture_fn = x\nlet x = 20\ncapture_fn\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "fn_closure_ast",
            t::ast("let x = 10\nfn capture_fn = x\nlet x = 20\ncapture_fn\n"),
            "let x = 10\nfn capture_fn = x\nlet x = 20\ncapture_fn\n"
        );
    }
}
mod cmp_lt {
    use super::*;
    #[test]
    fn eval() {
        s!("cmp_lt", t::eval_all("1 < 2\n"), "1 < 2\n");
    }
    #[test]
    fn ast() {
        s!("cmp_lt_ast", t::ast("1 < 2\n"), "1 < 2\n");
    }
}
mod arith_mixed_rev {
    use super::*;
    #[test]
    fn eval() {
        s!("arith_mixed_rev", t::eval_all("2.5 + 1\n"), "2.5 + 1\n");
    }
    #[test]
    fn ast() {
        s!("arith_mixed_rev_ast", t::ast("2.5 + 1\n"), "2.5 + 1\n");
    }
}
mod measure_scalar_ops {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "measure_scalar_ops",
            t::eval_all(
                "measure meter\nlet x = meter 10\nlet doubled = x * 2\nlet halved = x / 2\ndoubled\nhalved\n"
            ),
            "measure meter\nlet x = meter 10\nlet doubled = x * 2\nlet halved = x / 2\ndoubled\nhalved\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "measure_scalar_ops_ast",
            t::ast(
                "measure meter\nlet x = meter 10\nlet doubled = x * 2\nlet halved = x / 2\ndoubled\nhalved\n"
            ),
            "measure meter\nlet x = meter 10\nlet doubled = x * 2\nlet halved = x / 2\ndoubled\nhalved\n"
        );
    }
}
mod cmp_ne {
    use super::*;
    #[test]
    fn eval() {
        s!("cmp_ne", t::eval_all("1 != 2\n"), "1 != 2\n");
    }
    #[test]
    fn ast() {
        s!("cmp_ne_ast", t::ast("1 != 2\n"), "1 != 2\n");
    }
}
mod fn_single_param {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "fn_single_param",
            t::eval_all("fn triple x = x * 3\ntriple 7\n"),
            "fn triple x = x * 3\ntriple 7\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "fn_single_param_ast",
            t::ast("fn triple x = x * 3\ntriple 7\n"),
            "fn triple x = x * 3\ntriple 7\n"
        );
    }
}
mod fn_zero_arity {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "fn_zero_arity",
            t::eval_all("fn get_value = 42\nget_value\n"),
            "fn get_value = 42\nget_value\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "fn_zero_arity_ast",
            t::ast("fn get_value = 42\nget_value\n"),
            "fn get_value = 42\nget_value\n"
        );
    }
}
mod example_06_functions {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "example_06_functions",
            t::eval_all(
                "# Functions\n# Define and call functions\n\n# Simple function\nfn double x = x * 2\ndouble 5\n\n# Multi-parameter function\nfn add x y = x + y\nadd 3 7\n\n# Function with closure\nlet outer = 100\nfn capture = outer + 1\ncapture\n"
            ),
            "# Functions\n# Define and call functions\n\n# Simple function\nfn double x = x * 2\ndouble 5\n\n# Multi-parameter function\nfn add x y = x + y\nadd 3 7\n\n# Function with closure\nlet outer = 100\nfn capture = outer + 1\ncapture\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "example_06_functions_ast",
            t::ast(
                "# Functions\n# Define and call functions\n\n# Simple function\nfn double x = x * 2\ndouble 5\n\n# Multi-parameter function\nfn add x y = x + y\nadd 3 7\n\n# Function with closure\nlet outer = 100\nfn capture = outer + 1\ncapture\n"
            ),
            "# Functions\n# Define and call functions\n\n# Simple function\nfn double x = x * 2\ndouble 5\n\n# Multi-parameter function\nfn add x y = x + y\nadd 3 7\n\n# Function with closure\nlet outer = 100\nfn capture = outer + 1\ncapture\n"
        );
    }
}
mod example_05_variables {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "example_05_variables",
            t::eval_all(
                "# Variables with let\n# Define and use variables\n\n# Simple binding\nlet x = 42\nx\n\n# Multiple bindings\nlet a = 1\nlet b = 2\na + b\n\n# Using expressions\nlet result = 10 * 5 + 3\nresult\n\n# Variable reassignment\nlet counter = 0\nlet counter = counter + 1\nlet counter = counter + 1\ncounter\n"
            ),
            "# Variables with let\n# Define and use variables\n\n# Simple binding\nlet x = 42\nx\n\n# Multiple bindings\nlet a = 1\nlet b = 2\na + b\n\n# Using expressions\nlet result = 10 * 5 + 3\nresult\n\n# Variable reassignment\nlet counter = 0\nlet counter = counter + 1\nlet counter = counter + 1\ncounter\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "example_05_variables_ast",
            t::ast(
                "# Variables with let\n# Define and use variables\n\n# Simple binding\nlet x = 42\nx\n\n# Multiple bindings\nlet a = 1\nlet b = 2\na + b\n\n# Using expressions\nlet result = 10 * 5 + 3\nresult\n\n# Variable reassignment\nlet counter = 0\nlet counter = counter + 1\nlet counter = counter + 1\ncounter\n"
            ),
            "# Variables with let\n# Define and use variables\n\n# Simple binding\nlet x = 42\nx\n\n# Multiple bindings\nlet a = 1\nlet b = 2\na + b\n\n# Using expressions\nlet result = 10 * 5 + 3\nresult\n\n# Variable reassignment\nlet counter = 0\nlet counter = counter + 1\nlet counter = counter + 1\ncounter\n"
        );
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
    #[test]
    fn ast() {
        s!(
            "let_simple_ast",
            t::ast("let x = 42\nx\n"),
            "let x = 42\nx\n"
        );
    }
}
mod record_nested {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "record_nested",
            t::eval_all("{ a = { b = 1 } }\n"),
            "{ a = { b = 1 } }\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "record_nested_ast",
            t::ast("{ a = { b = 1 } }\n"),
            "{ a = { b = 1 } }\n"
        );
    }
}
mod measure_velocity {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "measure_velocity",
            t::eval_all(
                "measure meter\nmeasure second\nlet distance = meter 100\nlet time = second 10\nlet velocity = distance / time\nvelocity\n"
            ),
            "measure meter\nmeasure second\nlet distance = meter 100\nlet time = second 10\nlet velocity = distance / time\nvelocity\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "measure_velocity_ast",
            t::ast(
                "measure meter\nmeasure second\nlet distance = meter 100\nlet time = second 10\nlet velocity = distance / time\nvelocity\n"
            ),
            "measure meter\nmeasure second\nlet distance = meter 100\nlet time = second 10\nlet velocity = distance / time\nvelocity\n"
        );
    }
}
mod arith_sub {
    use super::*;
    #[test]
    fn eval() {
        s!("arith_sub", t::eval_all("10 - 3\n"), "10 - 3\n");
    }
    #[test]
    fn ast() {
        s!("arith_sub_ast", t::ast("10 - 3\n"), "10 - 3\n");
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
    #[test]
    fn ast() {
        s!(
            "error_undefined_ast",
            t::ast("undefined_var\n"),
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
    #[test]
    fn ast() {
        s!(
            "lit_string_ast",
            t::ast("\"hello world\"\n"),
            "\"hello world\"\n"
        );
    }
}
mod measure_multiply {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "measure_multiply",
            t::eval_all("measure inch\nmeasure foot = inch 12\n"),
            "measure inch\nmeasure foot = inch 12\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "measure_multiply_ast",
            t::ast("measure inch\nmeasure foot = inch 12\n"),
            "measure inch\nmeasure foot = inch 12\n"
        );
    }
}
mod arith_mixed {
    use super::*;
    #[test]
    fn eval() {
        s!("arith_mixed", t::eval_all("1 + 2.5\n"), "1 + 2.5\n");
    }
    #[test]
    fn ast() {
        s!("arith_mixed_ast", t::ast("1 + 2.5\n"), "1 + 2.5\n");
    }
}
mod measure_conversion {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "measure_conversion",
            t::eval_all(
                "measure millimeter  \nmeasure inch = millimeter 25.4\nlet x = 25.4millimeter\nlet y = 1inch\nx\ny\n"
            ),
            "measure millimeter  \nmeasure inch = millimeter 25.4\nlet x = 25.4millimeter\nlet y = 1inch\nx\ny\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "measure_conversion_ast",
            t::ast(
                "measure millimeter  \nmeasure inch = millimeter 25.4\nlet x = 25.4millimeter\nlet y = 1inch\nx\ny\n"
            ),
            "measure millimeter  \nmeasure inch = millimeter 25.4\nlet x = 25.4millimeter\nlet y = 1inch\nx\ny\n"
        );
    }
}
mod let_reassign {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "let_reassign",
            t::eval_all("let x = 1\nx = 2\nx\n"),
            "let x = 1\nx = 2\nx\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "let_reassign_ast",
            t::ast("let x = 1\nx = 2\nx\n"),
            "let x = 1\nx = 2\nx\n"
        );
    }
}
mod op_assign {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "op_assign",
            t::eval_all("let add_op = +\nadd_op 1 2\n"),
            "let add_op = +\nadd_op 1 2\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "op_assign_ast",
            t::ast("let add_op = +\nadd_op 1 2\n"),
            "let add_op = +\nadd_op 1 2\n"
        );
    }
}
mod example_01_welcome {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "example_01_welcome",
            t::eval_all(
                "# Welcome to Cadenza!\n# A functional language with units of measure\n\n# Try some basic expressions\n42\n3.14159\n1 + 2 * 3\n\n# Define variables\nlet name = \"Cadenza\"\nlet version = 0.1\n\n# Create functions\nfn square x = x * x\nsquare 5\n"
            ),
            "# Welcome to Cadenza!\n# A functional language with units of measure\n\n# Try some basic expressions\n42\n3.14159\n1 + 2 * 3\n\n# Define variables\nlet name = \"Cadenza\"\nlet version = 0.1\n\n# Create functions\nfn square x = x * x\nsquare 5\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "example_01_welcome_ast",
            t::ast(
                "# Welcome to Cadenza!\n# A functional language with units of measure\n\n# Try some basic expressions\n42\n3.14159\n1 + 2 * 3\n\n# Define variables\nlet name = \"Cadenza\"\nlet version = 0.1\n\n# Create functions\nfn square x = x * x\nsquare 5\n"
            ),
            "# Welcome to Cadenza!\n# A functional language with units of measure\n\n# Try some basic expressions\n42\n3.14159\n1 + 2 * 3\n\n# Define variables\nlet name = \"Cadenza\"\nlet version = 0.1\n\n# Create functions\nfn square x = x * x\nsquare 5\n"
        );
    }
}
mod measure_dimension_mismatch {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "measure_dimension_mismatch",
            t::eval_all("measure meter\n1meter * 2meter + 3meter\n"),
            "measure meter\n1meter * 2meter + 3meter\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "measure_dimension_mismatch_ast",
            t::ast("measure meter\n1meter * 2meter + 3meter\n"),
            "measure meter\n1meter * 2meter + 3meter\n"
        );
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
    #[test]
    fn ast() {
        s!(
            "let_multi_ast",
            t::ast("let x = 1\nlet y = 2\nx + y\n"),
            "let x = 1\nlet y = 2\nx + y\n"
        );
    }
}
mod let_expr {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "let_expr",
            t::eval_all("let x = 1 + 2\nx\n"),
            "let x = 1 + 2\nx\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "let_expr_ast",
            t::ast("let x = 1 + 2\nx\n"),
            "let x = 1 + 2\nx\n"
        );
    }
}
mod op_override {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "op_override",
            t::eval_all("let my_buggy_add = *\nlet + = my_buggy_add\n1 + 2\n"),
            "let my_buggy_add = *\nlet + = my_buggy_add\n1 + 2\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "op_override_ast",
            t::ast("let my_buggy_add = *\nlet + = my_buggy_add\n1 + 2\n"),
            "let my_buggy_add = *\nlet + = my_buggy_add\n1 + 2\n"
        );
    }
}
mod arith_float {
    use super::*;
    #[test]
    fn eval() {
        s!("arith_float", t::eval_all("1.5 + 2.5\n"), "1.5 + 2.5\n");
    }
    #[test]
    fn ast() {
        s!("arith_float_ast", t::ast("1.5 + 2.5\n"), "1.5 + 2.5\n");
    }
}
mod arith_div {
    use super::*;
    #[test]
    fn eval() {
        s!("arith_div", t::eval_all("20 / 4\n"), "20 / 4\n");
    }
    #[test]
    fn ast() {
        s!("arith_div_ast", t::ast("20 / 4\n"), "20 / 4\n");
    }
}
mod error_cmp_type_mismatch_gte {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "error_cmp_type_mismatch_gte",
            t::eval_all("# Test that >= errors on type mismatch\n200 >= \"qux\"\n"),
            "# Test that >= errors on type mismatch\n200 >= \"qux\"\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "error_cmp_type_mismatch_gte_ast",
            t::ast("# Test that >= errors on type mismatch\n200 >= \"qux\"\n"),
            "# Test that >= errors on type mismatch\n200 >= \"qux\"\n"
        );
    }
}
mod measure_suffix {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "measure_suffix",
            t::eval_all("measure meter\nlet x = 25.4meter\nx\n"),
            "measure meter\nlet x = 25.4meter\nx\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "measure_suffix_ast",
            t::ast("measure meter\nlet x = 25.4meter\nx\n"),
            "measure meter\nlet x = 25.4meter\nx\n"
        );
    }
}
mod error_cmp_type_mismatch_lte {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "error_cmp_type_mismatch_lte",
            t::eval_all("# Test that <= errors on type mismatch\n\"bar\" <= 10\n"),
            "# Test that <= errors on type mismatch\n\"bar\" <= 10\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "error_cmp_type_mismatch_lte_ast",
            t::ast("# Test that <= errors on type mismatch\n\"bar\" <= 10\n"),
            "# Test that <= errors on type mismatch\n\"bar\" <= 10\n"
        );
    }
}
mod block_nested {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "block_nested",
            t::eval_all(
                "let foo =\n    let bar =\n        let baz =\n            1\n        baz\n    bar\nfoo\n"
            ),
            "let foo =\n    let bar =\n        let baz =\n            1\n        baz\n    bar\nfoo\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "block_nested_ast",
            t::ast(
                "let foo =\n    let bar =\n        let baz =\n            1\n        baz\n    bar\nfoo\n"
            ),
            "let foo =\n    let bar =\n        let baz =\n            1\n        baz\n    bar\nfoo\n"
        );
    }
}
mod fn_hoisting {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "fn_hoisting",
            t::eval_all("add 2 3\nfn add x y = x + y\nadd 2 3\n"),
            "add 2 3\nfn add x y = x + y\nadd 2 3\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "fn_hoisting_ast",
            t::ast("add 2 3\nfn add x y = x + y\nadd 2 3\n"),
            "add 2 3\nfn add x y = x + y\nadd 2 3\n"
        );
    }
}
mod example_07_measures {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "example_07_measures",
            t::eval_all(
                "# Units of Measure\n# Define and use physical units\n\n# Define base units\nmeasure meter\nmeasure second\n\n# Use base units\n10meter\n5second\n\n# Derived units\nmeasure kilometer = meter 1000\n2kilometer\n\n# Convert between units\nlet distance = 5000meter\nlet km = 5kilometer\ndistance\nkm\n\n# Unit arithmetic\nlet speed = 100meter / 10second\nspeed\n"
            ),
            "# Units of Measure\n# Define and use physical units\n\n# Define base units\nmeasure meter\nmeasure second\n\n# Use base units\n10meter\n5second\n\n# Derived units\nmeasure kilometer = meter 1000\n2kilometer\n\n# Convert between units\nlet distance = 5000meter\nlet km = 5kilometer\ndistance\nkm\n\n# Unit arithmetic\nlet speed = 100meter / 10second\nspeed\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "example_07_measures_ast",
            t::ast(
                "# Units of Measure\n# Define and use physical units\n\n# Define base units\nmeasure meter\nmeasure second\n\n# Use base units\n10meter\n5second\n\n# Derived units\nmeasure kilometer = meter 1000\n2kilometer\n\n# Convert between units\nlet distance = 5000meter\nlet km = 5kilometer\ndistance\nkm\n\n# Unit arithmetic\nlet speed = 100meter / 10second\nspeed\n"
            ),
            "# Units of Measure\n# Define and use physical units\n\n# Define base units\nmeasure meter\nmeasure second\n\n# Use base units\n10meter\n5second\n\n# Derived units\nmeasure kilometer = meter 1000\n2kilometer\n\n# Convert between units\nlet distance = 5000meter\nlet km = 5kilometer\ndistance\nkm\n\n# Unit arithmetic\nlet speed = 100meter / 10second\nspeed\n"
        );
    }
}
mod example_04_comparison {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "example_04_comparison",
            t::eval_all(
                "# Comparison Operators\n# Compare numbers with ==, !=, <, >, <=, >=\n\n# Equality\n5 == 5\n5 != 3\n\n# Ordering\n10 > 5\n3 < 7\n5 <= 5\n10 >= 10\n"
            ),
            "# Comparison Operators\n# Compare numbers with ==, !=, <, >, <=, >=\n\n# Equality\n5 == 5\n5 != 3\n\n# Ordering\n10 > 5\n3 < 7\n5 <= 5\n10 >= 10\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "example_04_comparison_ast",
            t::ast(
                "# Comparison Operators\n# Compare numbers with ==, !=, <, >, <=, >=\n\n# Equality\n5 == 5\n5 != 3\n\n# Ordering\n10 > 5\n3 < 7\n5 <= 5\n10 >= 10\n"
            ),
            "# Comparison Operators\n# Compare numbers with ==, !=, <, >, <=, >=\n\n# Equality\n5 == 5\n5 != 3\n\n# Ordering\n10 > 5\n3 < 7\n5 <= 5\n10 >= 10\n"
        );
    }
}
mod multi_expr {
    use super::*;
    #[test]
    fn eval() {
        s!("multi_expr", t::eval_all("1\n2\n3\n"), "1\n2\n3\n");
    }
    #[test]
    fn ast() {
        s!("multi_expr_ast", t::ast("1\n2\n3\n"), "1\n2\n3\n");
    }
}
mod error_cmp_type_mismatch_eq {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "error_cmp_type_mismatch_eq",
            t::eval_all("# Test that == errors on type mismatch\n1 == \"hello\"\n"),
            "# Test that == errors on type mismatch\n1 == \"hello\"\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "error_cmp_type_mismatch_eq_ast",
            t::ast("# Test that == errors on type mismatch\n1 == \"hello\"\n"),
            "# Test that == errors on type mismatch\n1 == \"hello\"\n"
        );
    }
}
