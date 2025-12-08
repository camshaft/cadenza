use crate::testing as t;
use insta::{assert_debug_snapshot as s, assert_snapshot as ss};
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
    #[test]
    fn ir() {
        ss!("record_simple_ir", t::ir("{ a = 1, b = 2 }\n"));
    }
    #[test]
    fn wat() {
        ss!("record_simple_wat", t::wat("{ a = 1, b = 2 }\n"));
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
    #[test]
    fn ir() {
        ss!("cmp_gt_ir", t::ir("2 > 1\n"));
    }
    #[test]
    fn wat() {
        ss!("cmp_gt_wat", t::wat("2 > 1\n"));
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
    #[test]
    fn ir() {
        ss!("fn_basic_ir", t::ir("fn add x y = x + y\nadd 3 5\n"));
    }
    #[test]
    fn wat() {
        ss!("fn_basic_wat", t::wat("fn add x y = x + y\nadd 3 5\n"));
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
    #[test]
    fn ir() {
        ss!("fn_auto_apply_ir", t::ir("fn add x y = x + y\nadd\n"));
    }
    #[test]
    fn wat() {
        ss!("fn_auto_apply_wat", t::wat("fn add x y = x + y\nadd\n"));
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
    #[test]
    fn ir() {
        ss!("lit_int_ir", t::ir("42\n"));
    }
    #[test]
    fn wat() {
        ss!("lit_int_wat", t::wat("42\n"));
    }
}
mod typeof_string {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "typeof_string",
            t::eval_all("let s = \"hello\"\ntypeof s"),
            "let s = \"hello\"\ntypeof s"
        );
    }
    #[test]
    fn ast() {
        s!(
            "typeof_string_ast",
            t::ast("let s = \"hello\"\ntypeof s"),
            "let s = \"hello\"\ntypeof s"
        );
    }
    #[test]
    fn ir() {
        ss!("typeof_string_ir", t::ir("let s = \"hello\"\ntypeof s"));
    }
    #[test]
    fn wat() {
        ss!("typeof_string_wat", t::wat("let s = \"hello\"\ntypeof s"));
    }
}
mod assert_fail {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "assert_fail",
            t::eval_all("let v = 1\nassert v == 2\n"),
            "let v = 1\nassert v == 2\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "assert_fail_ast",
            t::ast("let v = 1\nassert v == 2\n"),
            "let v = 1\nassert v == 2\n"
        );
    }
    #[test]
    fn ir() {
        ss!("assert_fail_ir", t::ir("let v = 1\nassert v == 2\n"));
    }
    #[test]
    fn wat() {
        ss!("assert_fail_wat", t::wat("let v = 1\nassert v == 2\n"));
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
    #[test]
    fn ir() {
        ss!("error_divzero_ir", t::ir("1 / 0\n"));
    }
    #[test]
    fn wat() {
        ss!("error_divzero_wat", t::wat("1 / 0\n"));
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
    #[test]
    fn ir() {
        ss!("cmp_eq_ir", t::ir("1 == 1\n"));
    }
    #[test]
    fn wat() {
        ss!("cmp_eq_wat", t::wat("1 == 1\n"));
    }
}
mod pipeline_multi {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "pipeline_multi",
            t::eval_all(
                "fn add x y = x + y\nfn mul x y = x * y\nfn sub x y = x - y\n\n5 |> add 3 |> mul 2\n10 |> sub 3 |> add 5 |> mul 2\n"
            ),
            "fn add x y = x + y\nfn mul x y = x * y\nfn sub x y = x - y\n\n5 |> add 3 |> mul 2\n10 |> sub 3 |> add 5 |> mul 2\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "pipeline_multi_ast",
            t::ast(
                "fn add x y = x + y\nfn mul x y = x * y\nfn sub x y = x - y\n\n5 |> add 3 |> mul 2\n10 |> sub 3 |> add 5 |> mul 2\n"
            ),
            "fn add x y = x + y\nfn mul x y = x * y\nfn sub x y = x - y\n\n5 |> add 3 |> mul 2\n10 |> sub 3 |> add 5 |> mul 2\n"
        );
    }
    #[test]
    fn ir() {
        ss!(
            "pipeline_multi_ir",
            t::ir(
                "fn add x y = x + y\nfn mul x y = x * y\nfn sub x y = x - y\n\n5 |> add 3 |> mul 2\n10 |> sub 3 |> add 5 |> mul 2\n"
            )
        );
    }
    #[test]
    fn wat() {
        ss!(
            "pipeline_multi_wat",
            t::wat(
                "fn add x y = x + y\nfn mul x y = x * y\nfn sub x y = x - y\n\n5 |> add 3 |> mul 2\n10 |> sub 3 |> add 5 |> mul 2\n"
            )
        );
    }
}
mod field_access_chained {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "field_access_chained",
            t::eval_all("let obj = { a = { b = 42 } }\nobj.a.b\n"),
            "let obj = { a = { b = 42 } }\nobj.a.b\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "field_access_chained_ast",
            t::ast("let obj = { a = { b = 42 } }\nobj.a.b\n"),
            "let obj = { a = { b = 42 } }\nobj.a.b\n"
        );
    }
    #[test]
    fn ir() {
        ss!(
            "field_access_chained_ir",
            t::ir("let obj = { a = { b = 42 } }\nobj.a.b\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "field_access_chained_wat",
            t::wat("let obj = { a = { b = 42 } }\nobj.a.b\n")
        );
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
    #[test]
    fn ir() {
        ss!(
            "measure_incompatible_ir",
            t::ir(
                "measure meter\nmeasure second\nlet distance = meter 100\nlet time = second 10\ndistance + time\n"
            )
        );
    }
    #[test]
    fn wat() {
        ss!(
            "measure_incompatible_wat",
            t::wat(
                "measure meter\nmeasure second\nlet distance = meter 100\nlet time = second 10\ndistance + time\n"
            )
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
    #[test]
    fn ir() {
        ss!(
            "example_08_lists_ir",
            t::ir(
                "# Lists\n# Collection of values in square brackets\n\n# Empty list\n[]\n\n# Simple list of integers\n[1, 2, 3, 4, 5]\n\n# Assign to variable\nlet numbers = [10, 20, 30]\nnumbers\n\n# List with expressions\nlet x = 5\nlet y = 10\n[x, y, x + y, x * y]\n\n# Nested lists\n[[1, 2], [3, 4], [5, 6]]\n\n# List with computed values\nlet a = 100\nlet b = 200\n[[a, a * 2], [b, b / 2]]\n"
            )
        );
    }
    #[test]
    fn wat() {
        ss!(
            "example_08_lists_wat",
            t::wat(
                "# Lists\n# Collection of values in square brackets\n\n# Empty list\n[]\n\n# Simple list of integers\n[1, 2, 3, 4, 5]\n\n# Assign to variable\nlet numbers = [10, 20, 30]\nnumbers\n\n# List with expressions\nlet x = 5\nlet y = 10\n[x, y, x + y, x * y]\n\n# Nested lists\n[[1, 2], [3, 4], [5, 6]]\n\n# List with computed values\nlet a = 100\nlet b = 200\n[[a, a * 2], [b, b / 2]]\n"
            )
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
    #[test]
    fn ir() {
        ss!("lit_float_ir", t::ir("3.14\n"));
    }
    #[test]
    fn wat() {
        ss!("lit_float_wat", t::wat("3.14\n"));
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
    #[test]
    fn ir() {
        ss!(
            "example_03_arithmetic_ir",
            t::ir(
                "# Arithmetic Operations\n# Basic math with integers and floats\n\n# Addition and subtraction\n1 + 2\n10 - 3\n\n# Multiplication and division\n4 * 5\n20 / 4\n\n# Operator precedence\n2 + 3 * 4\n(2 + 3) * 4\n\n# Floating point\n3.14 * 2.0\n10.5 / 2.0\n"
            )
        );
    }
    #[test]
    fn wat() {
        ss!(
            "example_03_arithmetic_wat",
            t::wat(
                "# Arithmetic Operations\n# Basic math with integers and floats\n\n# Addition and subtraction\n1 + 2\n10 - 3\n\n# Multiplication and division\n4 * 5\n20 / 4\n\n# Operator precedence\n2 + 3 * 4\n(2 + 3) * 4\n\n# Floating point\n3.14 * 2.0\n10.5 / 2.0\n"
            )
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
    #[test]
    fn ir() {
        ss!(
            "measure_unit_arithmetic_ir",
            t::ir(
                "measure millimeter\nmeasure meter = millimeter 1000\nlet x = millimeter 500\nlet y = meter 1\nlet sum = x + y\nsum\n"
            )
        );
    }
    #[test]
    fn wat() {
        ss!(
            "measure_unit_arithmetic_wat",
            t::wat(
                "measure millimeter\nmeasure meter = millimeter 1000\nlet x = millimeter 500\nlet y = meter 1\nlet sum = x + y\nsum\n"
            )
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
    #[test]
    fn ir() {
        ss!(
            "error_cmp_type_mismatch_ne_ir",
            t::ir("# Test that != errors on type mismatch\n42 != \"world\"\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "error_cmp_type_mismatch_ne_wat",
            t::wat("# Test that != errors on type mismatch\n42 != \"world\"\n")
        );
    }
}
mod field_assign_simple {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "field_assign_simple",
            t::eval_all("let point = { x = 10, y = 20 }\npoint.x = 30\npoint.x\n"),
            "let point = { x = 10, y = 20 }\npoint.x = 30\npoint.x\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "field_assign_simple_ast",
            t::ast("let point = { x = 10, y = 20 }\npoint.x = 30\npoint.x\n"),
            "let point = { x = 10, y = 20 }\npoint.x = 30\npoint.x\n"
        );
    }
    #[test]
    fn ir() {
        ss!(
            "field_assign_simple_ir",
            t::ir("let point = { x = 10, y = 20 }\npoint.x = 30\npoint.x\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "field_assign_simple_wat",
            t::wat("let point = { x = 10, y = 20 }\npoint.x = 30\npoint.x\n")
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
    #[test]
    fn ir() {
        ss!("arith_mul_ir", t::ir("4 * 5\n"));
    }
    #[test]
    fn wat() {
        ss!("arith_mul_wat", t::wat("4 * 5\n"));
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
    #[test]
    fn ir() {
        ss!(
            "block_simple_ir",
            t::ir("let foo =\n    let bar = 1\n    let baz = 2\n    bar\nfoo\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "block_simple_wat",
            t::wat("let foo =\n    let bar = 1\n    let baz = 2\n    bar\nfoo\n")
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
    #[test]
    fn ir() {
        ss!(
            "record_with_variables_ir",
            t::ir("let x = 1\nlet y = 2\n{ a = x, b = y }\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "record_with_variables_wat",
            t::wat("let x = 1\nlet y = 2\n{ a = x, b = y }\n")
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
    #[test]
    fn ir() {
        ss!("error_let_invalid_ir", t::ir("let 42 = 1\n"));
    }
    #[test]
    fn wat() {
        ss!("error_let_invalid_wat", t::wat("let 42 = 1\n"));
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
    #[test]
    fn ir() {
        ss!(
            "measure_quantity_ir",
            t::ir("measure meter\nlet x = meter 5\nx\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "measure_quantity_wat",
            t::wat("measure meter\nlet x = meter 5\nx\n")
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
    #[test]
    fn ir() {
        ss!(
            "block_scope_ir",
            t::ir(
                "let outer = 100\nlet result =\n    let inner = 200\n    inner + outer\nresult\n"
            )
        );
    }
    #[test]
    fn wat() {
        ss!(
            "block_scope_wat",
            t::wat(
                "let outer = 100\nlet result =\n    let inner = 200\n    inner + outer\nresult\n"
            )
        );
    }
}
mod type_inference_demo {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "type_inference_demo",
            t::eval_all(
                "# Demonstrates type inference improvements in IR generation\n\n# Function with concrete return type\nfn get_answer = 42\n\n# Function with operations on literals\nfn compute = 10 * 5 + 2\n\n# Function using let bindings with literals\nfn with_let =\n    let x = 100\n    let y = 200\n    x\n\n# Call the functions to test\nget_answer\n"
            ),
            "# Demonstrates type inference improvements in IR generation\n\n# Function with concrete return type\nfn get_answer = 42\n\n# Function with operations on literals\nfn compute = 10 * 5 + 2\n\n# Function using let bindings with literals\nfn with_let =\n    let x = 100\n    let y = 200\n    x\n\n# Call the functions to test\nget_answer\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "type_inference_demo_ast",
            t::ast(
                "# Demonstrates type inference improvements in IR generation\n\n# Function with concrete return type\nfn get_answer = 42\n\n# Function with operations on literals\nfn compute = 10 * 5 + 2\n\n# Function using let bindings with literals\nfn with_let =\n    let x = 100\n    let y = 200\n    x\n\n# Call the functions to test\nget_answer\n"
            ),
            "# Demonstrates type inference improvements in IR generation\n\n# Function with concrete return type\nfn get_answer = 42\n\n# Function with operations on literals\nfn compute = 10 * 5 + 2\n\n# Function using let bindings with literals\nfn with_let =\n    let x = 100\n    let y = 200\n    x\n\n# Call the functions to test\nget_answer\n"
        );
    }
    #[test]
    fn ir() {
        ss!(
            "type_inference_demo_ir",
            t::ir(
                "# Demonstrates type inference improvements in IR generation\n\n# Function with concrete return type\nfn get_answer = 42\n\n# Function with operations on literals\nfn compute = 10 * 5 + 2\n\n# Function using let bindings with literals\nfn with_let =\n    let x = 100\n    let y = 200\n    x\n\n# Call the functions to test\nget_answer\n"
            )
        );
    }
    #[test]
    fn wat() {
        ss!(
            "type_inference_demo_wat",
            t::wat(
                "# Demonstrates type inference improvements in IR generation\n\n# Function with concrete return type\nfn get_answer = 42\n\n# Function with operations on literals\nfn compute = 10 * 5 + 2\n\n# Function using let bindings with literals\nfn with_let =\n    let x = 100\n    let y = 200\n    x\n\n# Call the functions to test\nget_answer\n"
            )
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
    #[test]
    fn ir() {
        ss!("measure_base_ir", t::ir("measure meter\n"));
    }
    #[test]
    fn wat() {
        ss!("measure_base_wat", t::wat("measure meter\n"));
    }
}
mod typeof_integer {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "typeof_integer",
            t::eval_all("let x = 42\ntypeof x"),
            "let x = 42\ntypeof x"
        );
    }
    #[test]
    fn ast() {
        s!(
            "typeof_integer_ast",
            t::ast("let x = 42\ntypeof x"),
            "let x = 42\ntypeof x"
        );
    }
    #[test]
    fn ir() {
        ss!("typeof_integer_ir", t::ir("let x = 42\ntypeof x"));
    }
    #[test]
    fn wat() {
        ss!("typeof_integer_wat", t::wat("let x = 42\ntypeof x"));
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
    #[test]
    fn ir() {
        ss!(
            "error_cmp_type_mismatch_lt_ir",
            t::ir("# Test that < errors on non-numeric types\n\"foo\" < 5\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "error_cmp_type_mismatch_lt_wat",
            t::wat("# Test that < errors on non-numeric types\n\"foo\" < 5\n")
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
    #[test]
    fn ir() {
        ss!("arith_float_mul_ir", t::ir("3.0 * 2.0\n"));
    }
    #[test]
    fn wat() {
        ss!("arith_float_mul_wat", t::wat("3.0 * 2.0\n"));
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
    #[test]
    fn ir() {
        ss!("arith_precedence_ir", t::ir("2 + 3 * 4\n"));
    }
    #[test]
    fn wat() {
        ss!("arith_precedence_wat", t::wat("2 + 3 * 4\n"));
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
    #[test]
    fn ir() {
        ss!("cmp_le_ir", t::ir("1 <= 1\n"));
    }
    #[test]
    fn wat() {
        ss!("cmp_le_wat", t::wat("1 <= 1\n"));
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
    #[test]
    fn ir() {
        ss!(
            "block_function_body_ir",
            t::ir("fn foo a b =\n    let av = a * 2\n    let bv = b * 3\n    av * bv\nfoo 5 7\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "block_function_body_wat",
            t::wat("fn foo a b =\n    let av = a * 2\n    let bv = b * 3\n    av * bv\nfoo 5 7\n")
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
    #[test]
    fn ir() {
        ss!(
            "record_shorthand_ir",
            t::ir("let x = 1\nlet y = 2\n{ x, y }\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "record_shorthand_wat",
            t::wat("let x = 1\nlet y = 2\n{ x, y }\n")
        );
    }
}
mod field_access_missing_field {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "field_access_missing_field",
            t::eval_all("let point = { x = 10, y = 20 }\npoint.z\n"),
            "let point = { x = 10, y = 20 }\npoint.z\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "field_access_missing_field_ast",
            t::ast("let point = { x = 10, y = 20 }\npoint.z\n"),
            "let point = { x = 10, y = 20 }\npoint.z\n"
        );
    }
    #[test]
    fn ir() {
        ss!(
            "field_access_missing_field_ir",
            t::ir("let point = { x = 10, y = 20 }\npoint.z\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "field_access_missing_field_wat",
            t::wat("let point = { x = 10, y = 20 }\npoint.z\n")
        );
    }
}
mod match_no_parens {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "match_no_parens",
            t::eval_all(
                "# Test match expression without outer parentheses\n\n# Basic true pattern - single line syntax\nlet result1 = match true (true -> 42) (false -> 0)\nassert result1 == 42\n\n# Basic false pattern\nlet result2 = match false (true -> 42) (false -> 0)\nassert result2 == 0\n\n# Match with comparison\nlet x = 5\nlet result3 = match x > 0 (true -> \"positive\") (false -> \"negative\")\nassert result3 == \"positive\"\n\n# Match with comparison (false case)\nlet y = -3\nlet result4 = match y > 0 (true -> \"positive\") (false -> \"negative\")\nassert result4 == \"negative\"\n\n# Nested match expressions\nlet z = 10\nlet result5 = match z > 5 (true -> match z > 15 (true -> \"very large\") (false -> \"large\")) (false -> \"small\")\nassert result5 == \"large\"\n"
            ),
            "# Test match expression without outer parentheses\n\n# Basic true pattern - single line syntax\nlet result1 = match true (true -> 42) (false -> 0)\nassert result1 == 42\n\n# Basic false pattern\nlet result2 = match false (true -> 42) (false -> 0)\nassert result2 == 0\n\n# Match with comparison\nlet x = 5\nlet result3 = match x > 0 (true -> \"positive\") (false -> \"negative\")\nassert result3 == \"positive\"\n\n# Match with comparison (false case)\nlet y = -3\nlet result4 = match y > 0 (true -> \"positive\") (false -> \"negative\")\nassert result4 == \"negative\"\n\n# Nested match expressions\nlet z = 10\nlet result5 = match z > 5 (true -> match z > 15 (true -> \"very large\") (false -> \"large\")) (false -> \"small\")\nassert result5 == \"large\"\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "match_no_parens_ast",
            t::ast(
                "# Test match expression without outer parentheses\n\n# Basic true pattern - single line syntax\nlet result1 = match true (true -> 42) (false -> 0)\nassert result1 == 42\n\n# Basic false pattern\nlet result2 = match false (true -> 42) (false -> 0)\nassert result2 == 0\n\n# Match with comparison\nlet x = 5\nlet result3 = match x > 0 (true -> \"positive\") (false -> \"negative\")\nassert result3 == \"positive\"\n\n# Match with comparison (false case)\nlet y = -3\nlet result4 = match y > 0 (true -> \"positive\") (false -> \"negative\")\nassert result4 == \"negative\"\n\n# Nested match expressions\nlet z = 10\nlet result5 = match z > 5 (true -> match z > 15 (true -> \"very large\") (false -> \"large\")) (false -> \"small\")\nassert result5 == \"large\"\n"
            ),
            "# Test match expression without outer parentheses\n\n# Basic true pattern - single line syntax\nlet result1 = match true (true -> 42) (false -> 0)\nassert result1 == 42\n\n# Basic false pattern\nlet result2 = match false (true -> 42) (false -> 0)\nassert result2 == 0\n\n# Match with comparison\nlet x = 5\nlet result3 = match x > 0 (true -> \"positive\") (false -> \"negative\")\nassert result3 == \"positive\"\n\n# Match with comparison (false case)\nlet y = -3\nlet result4 = match y > 0 (true -> \"positive\") (false -> \"negative\")\nassert result4 == \"negative\"\n\n# Nested match expressions\nlet z = 10\nlet result5 = match z > 5 (true -> match z > 15 (true -> \"very large\") (false -> \"large\")) (false -> \"small\")\nassert result5 == \"large\"\n"
        );
    }
    #[test]
    fn ir() {
        ss!(
            "match_no_parens_ir",
            t::ir(
                "# Test match expression without outer parentheses\n\n# Basic true pattern - single line syntax\nlet result1 = match true (true -> 42) (false -> 0)\nassert result1 == 42\n\n# Basic false pattern\nlet result2 = match false (true -> 42) (false -> 0)\nassert result2 == 0\n\n# Match with comparison\nlet x = 5\nlet result3 = match x > 0 (true -> \"positive\") (false -> \"negative\")\nassert result3 == \"positive\"\n\n# Match with comparison (false case)\nlet y = -3\nlet result4 = match y > 0 (true -> \"positive\") (false -> \"negative\")\nassert result4 == \"negative\"\n\n# Nested match expressions\nlet z = 10\nlet result5 = match z > 5 (true -> match z > 15 (true -> \"very large\") (false -> \"large\")) (false -> \"small\")\nassert result5 == \"large\"\n"
            )
        );
    }
    #[test]
    fn wat() {
        ss!(
            "match_no_parens_wat",
            t::wat(
                "# Test match expression without outer parentheses\n\n# Basic true pattern - single line syntax\nlet result1 = match true (true -> 42) (false -> 0)\nassert result1 == 42\n\n# Basic false pattern\nlet result2 = match false (true -> 42) (false -> 0)\nassert result2 == 0\n\n# Match with comparison\nlet x = 5\nlet result3 = match x > 0 (true -> \"positive\") (false -> \"negative\")\nassert result3 == \"positive\"\n\n# Match with comparison (false case)\nlet y = -3\nlet result4 = match y > 0 (true -> \"positive\") (false -> \"negative\")\nassert result4 == \"negative\"\n\n# Nested match expressions\nlet z = 10\nlet result5 = match z > 5 (true -> match z > 15 (true -> \"very large\") (false -> \"large\")) (false -> \"small\")\nassert result5 == \"large\"\n"
            )
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
    #[test]
    fn ir() {
        ss!("arith_add_ir", t::ir("1 + 2\n"));
    }
    #[test]
    fn wat() {
        ss!("arith_add_wat", t::wat("1 + 2\n"));
    }
}
mod assert_pass {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "assert_pass",
            t::eval_all("let v = 1\nassert v == 1\n"),
            "let v = 1\nassert v == 1\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "assert_pass_ast",
            t::ast("let v = 1\nassert v == 1\n"),
            "let v = 1\nassert v == 1\n"
        );
    }
    #[test]
    fn ir() {
        ss!("assert_pass_ir", t::ir("let v = 1\nassert v == 1\n"));
    }
    #[test]
    fn wat() {
        ss!("assert_pass_wat", t::wat("let v = 1\nassert v == 1\n"));
    }
}
mod typeof_function {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "typeof_function",
            t::eval_all("fn identity x = x\ntypeof identity"),
            "fn identity x = x\ntypeof identity"
        );
    }
    #[test]
    fn ast() {
        s!(
            "typeof_function_ast",
            t::ast("fn identity x = x\ntypeof identity"),
            "fn identity x = x\ntypeof identity"
        );
    }
    #[test]
    fn ir() {
        ss!(
            "typeof_function_ir",
            t::ir("fn identity x = x\ntypeof identity")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "typeof_function_wat",
            t::wat("fn identity x = x\ntypeof identity")
        );
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
    #[test]
    fn ir() {
        ss!("cmp_ge_ir", t::ir("1 >= 1\n"));
    }
    #[test]
    fn wat() {
        ss!("cmp_ge_wat", t::wat("1 >= 1\n"));
    }
}
mod field_assign_type_mismatch {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "field_assign_type_mismatch",
            t::eval_all("let point = { x = 10, y = 20 }\npoint.x = \"foo\"\n"),
            "let point = { x = 10, y = 20 }\npoint.x = \"foo\"\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "field_assign_type_mismatch_ast",
            t::ast("let point = { x = 10, y = 20 }\npoint.x = \"foo\"\n"),
            "let point = { x = 10, y = 20 }\npoint.x = \"foo\"\n"
        );
    }
    #[test]
    fn ir() {
        ss!(
            "field_assign_type_mismatch_ir",
            t::ir("let point = { x = 10, y = 20 }\npoint.x = \"foo\"\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "field_assign_type_mismatch_wat",
            t::wat("let point = { x = 10, y = 20 }\npoint.x = \"foo\"\n")
        );
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
    #[test]
    fn ir() {
        ss!("record_empty_ir", t::ir("{}\n"));
    }
    #[test]
    fn wat() {
        ss!("record_empty_wat", t::wat("{}\n"));
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
    #[test]
    fn ir() {
        ss!(
            "example_02_literals_ir",
            t::ir(
                "# Literal Values\n# Different types of literals\n\n# Integers\n42\n-17\n0\n\n# Floating point\n3.14159\n-2.5\n1.0\n\n# Strings\n\"hello\"\n\"world\"\n\"hello world\"\n"
            )
        );
    }
    #[test]
    fn wat() {
        ss!(
            "example_02_literals_wat",
            t::wat(
                "# Literal Values\n# Different types of literals\n\n# Integers\n42\n-17\n0\n\n# Floating point\n3.14159\n-2.5\n1.0\n\n# Strings\n\"hello\"\n\"world\"\n\"hello world\"\n"
            )
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
    #[test]
    fn ir() {
        ss!(
            "error_cmp_type_mismatch_gt_ir",
            t::ir("# Test that > errors on type mismatch\n100 > \"baz\"\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "error_cmp_type_mismatch_gt_wat",
            t::wat("# Test that > errors on type mismatch\n100 > \"baz\"\n")
        );
    }
}
mod field_access_on_expr {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "field_access_on_expr",
            t::eval_all("fn make_rec x = { x }\n(make_rec 1).x\n"),
            "fn make_rec x = { x }\n(make_rec 1).x\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "field_access_on_expr_ast",
            t::ast("fn make_rec x = { x }\n(make_rec 1).x\n"),
            "fn make_rec x = { x }\n(make_rec 1).x\n"
        );
    }
    #[test]
    fn ir() {
        ss!(
            "field_access_on_expr_ir",
            t::ir("fn make_rec x = { x }\n(make_rec 1).x\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "field_access_on_expr_wat",
            t::wat("fn make_rec x = { x }\n(make_rec 1).x\n")
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
    #[test]
    fn ir() {
        ss!("arith_left_assoc_ir", t::ir("10 - 5 - 2\n"));
    }
    #[test]
    fn wat() {
        ss!("arith_left_assoc_wat", t::wat("10 - 5 - 2\n"));
    }
}
mod field_access_simple {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "field_access_simple",
            t::eval_all("let point = { x = 10, y = 20 }\npoint.x\n"),
            "let point = { x = 10, y = 20 }\npoint.x\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "field_access_simple_ast",
            t::ast("let point = { x = 10, y = 20 }\npoint.x\n"),
            "let point = { x = 10, y = 20 }\npoint.x\n"
        );
    }
    #[test]
    fn ir() {
        ss!(
            "field_access_simple_ir",
            t::ir("let point = { x = 10, y = 20 }\npoint.x\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "field_access_simple_wat",
            t::wat("let point = { x = 10, y = 20 }\npoint.x\n")
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
    #[test]
    fn ir() {
        ss!(
            "fn_closure_ir",
            t::ir("let x = 10\nfn capture_fn = x\nlet x = 20\ncapture_fn\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "fn_closure_wat",
            t::wat("let x = 10\nfn capture_fn = x\nlet x = 20\ncapture_fn\n")
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
    #[test]
    fn ir() {
        ss!("cmp_lt_ir", t::ir("1 < 2\n"));
    }
    #[test]
    fn wat() {
        ss!("cmp_lt_wat", t::wat("1 < 2\n"));
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
    #[test]
    fn ir() {
        ss!("arith_mixed_rev_ir", t::ir("2.5 + 1\n"));
    }
    #[test]
    fn wat() {
        ss!("arith_mixed_rev_wat", t::wat("2.5 + 1\n"));
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
    #[test]
    fn ir() {
        ss!(
            "measure_scalar_ops_ir",
            t::ir(
                "measure meter\nlet x = meter 10\nlet doubled = x * 2\nlet halved = x / 2\ndoubled\nhalved\n"
            )
        );
    }
    #[test]
    fn wat() {
        ss!(
            "measure_scalar_ops_wat",
            t::wat(
                "measure meter\nlet x = meter 10\nlet doubled = x * 2\nlet halved = x / 2\ndoubled\nhalved\n"
            )
        );
    }
}
mod field_access_in_expr {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "field_access_in_expr",
            t::eval_all("let point = { x = 10, y = 20 }\npoint.x + point.y\n"),
            "let point = { x = 10, y = 20 }\npoint.x + point.y\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "field_access_in_expr_ast",
            t::ast("let point = { x = 10, y = 20 }\npoint.x + point.y\n"),
            "let point = { x = 10, y = 20 }\npoint.x + point.y\n"
        );
    }
    #[test]
    fn ir() {
        ss!(
            "field_access_in_expr_ir",
            t::ir("let point = { x = 10, y = 20 }\npoint.x + point.y\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "field_access_in_expr_wat",
            t::wat("let point = { x = 10, y = 20 }\npoint.x + point.y\n")
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
    #[test]
    fn ir() {
        ss!("cmp_ne_ir", t::ir("1 != 2\n"));
    }
    #[test]
    fn wat() {
        ss!("cmp_ne_wat", t::wat("1 != 2\n"));
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
    #[test]
    fn ir() {
        ss!(
            "fn_single_param_ir",
            t::ir("fn triple x = x * 3\ntriple 7\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "fn_single_param_wat",
            t::wat("fn triple x = x * 3\ntriple 7\n")
        );
    }
}
mod field_access_on_non_record {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "field_access_on_non_record",
            t::eval_all("let x = 42\nx.field\n"),
            "let x = 42\nx.field\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "field_access_on_non_record_ast",
            t::ast("let x = 42\nx.field\n"),
            "let x = 42\nx.field\n"
        );
    }
    #[test]
    fn ir() {
        ss!(
            "field_access_on_non_record_ir",
            t::ir("let x = 42\nx.field\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "field_access_on_non_record_wat",
            t::wat("let x = 42\nx.field\n")
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
    #[test]
    fn ir() {
        ss!("fn_zero_arity_ir", t::ir("fn get_value = 42\nget_value\n"));
    }
    #[test]
    fn wat() {
        ss!(
            "fn_zero_arity_wat",
            t::wat("fn get_value = 42\nget_value\n")
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
    #[test]
    fn ir() {
        ss!(
            "example_06_functions_ir",
            t::ir(
                "# Functions\n# Define and call functions\n\n# Simple function\nfn double x = x * 2\ndouble 5\n\n# Multi-parameter function\nfn add x y = x + y\nadd 3 7\n\n# Function with closure\nlet outer = 100\nfn capture = outer + 1\ncapture\n"
            )
        );
    }
    #[test]
    fn wat() {
        ss!(
            "example_06_functions_wat",
            t::wat(
                "# Functions\n# Define and call functions\n\n# Simple function\nfn double x = x * 2\ndouble 5\n\n# Multi-parameter function\nfn add x y = x + y\nadd 3 7\n\n# Function with closure\nlet outer = 100\nfn capture = outer + 1\ncapture\n"
            )
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
    #[test]
    fn ir() {
        ss!(
            "example_05_variables_ir",
            t::ir(
                "# Variables with let\n# Define and use variables\n\n# Simple binding\nlet x = 42\nx\n\n# Multiple bindings\nlet a = 1\nlet b = 2\na + b\n\n# Using expressions\nlet result = 10 * 5 + 3\nresult\n\n# Variable reassignment\nlet counter = 0\nlet counter = counter + 1\nlet counter = counter + 1\ncounter\n"
            )
        );
    }
    #[test]
    fn wat() {
        ss!(
            "example_05_variables_wat",
            t::wat(
                "# Variables with let\n# Define and use variables\n\n# Simple binding\nlet x = 42\nx\n\n# Multiple bindings\nlet a = 1\nlet b = 2\na + b\n\n# Using expressions\nlet result = 10 * 5 + 3\nresult\n\n# Variable reassignment\nlet counter = 0\nlet counter = counter + 1\nlet counter = counter + 1\ncounter\n"
            )
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
    #[test]
    fn ir() {
        ss!("let_simple_ir", t::ir("let x = 42\nx\n"));
    }
    #[test]
    fn wat() {
        ss!("let_simple_wat", t::wat("let x = 42\nx\n"));
    }
}
mod example_09_assertions {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "example_09_assertions",
            t::eval_all(
                "# Assertions - Runtime Checks\n# \n# The assert macro allows you to verify conditions at runtime\n# and provides detailed error messages when assertions fail.\n\n# Basic assertion - verifies a condition is true\nlet x = 5\nassert x > 0\n\n# Assertion with custom error message\nlet value = 42\nassert value == 42 \"value must be 42\"\n\n# Assertions are useful for validating function inputs and outputs\nfn divide a b =\n    assert b != 0 \"cannot divide by zero\"\n    a / b\n\ndivide 10 2\n\n# Assertions help catch errors early in development\nlet result = divide 10 2\nassert result == 5 \"expected result to be 5\"\n"
            ),
            "# Assertions - Runtime Checks\n# \n# The assert macro allows you to verify conditions at runtime\n# and provides detailed error messages when assertions fail.\n\n# Basic assertion - verifies a condition is true\nlet x = 5\nassert x > 0\n\n# Assertion with custom error message\nlet value = 42\nassert value == 42 \"value must be 42\"\n\n# Assertions are useful for validating function inputs and outputs\nfn divide a b =\n    assert b != 0 \"cannot divide by zero\"\n    a / b\n\ndivide 10 2\n\n# Assertions help catch errors early in development\nlet result = divide 10 2\nassert result == 5 \"expected result to be 5\"\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "example_09_assertions_ast",
            t::ast(
                "# Assertions - Runtime Checks\n# \n# The assert macro allows you to verify conditions at runtime\n# and provides detailed error messages when assertions fail.\n\n# Basic assertion - verifies a condition is true\nlet x = 5\nassert x > 0\n\n# Assertion with custom error message\nlet value = 42\nassert value == 42 \"value must be 42\"\n\n# Assertions are useful for validating function inputs and outputs\nfn divide a b =\n    assert b != 0 \"cannot divide by zero\"\n    a / b\n\ndivide 10 2\n\n# Assertions help catch errors early in development\nlet result = divide 10 2\nassert result == 5 \"expected result to be 5\"\n"
            ),
            "# Assertions - Runtime Checks\n# \n# The assert macro allows you to verify conditions at runtime\n# and provides detailed error messages when assertions fail.\n\n# Basic assertion - verifies a condition is true\nlet x = 5\nassert x > 0\n\n# Assertion with custom error message\nlet value = 42\nassert value == 42 \"value must be 42\"\n\n# Assertions are useful for validating function inputs and outputs\nfn divide a b =\n    assert b != 0 \"cannot divide by zero\"\n    a / b\n\ndivide 10 2\n\n# Assertions help catch errors early in development\nlet result = divide 10 2\nassert result == 5 \"expected result to be 5\"\n"
        );
    }
    #[test]
    fn ir() {
        ss!(
            "example_09_assertions_ir",
            t::ir(
                "# Assertions - Runtime Checks\n# \n# The assert macro allows you to verify conditions at runtime\n# and provides detailed error messages when assertions fail.\n\n# Basic assertion - verifies a condition is true\nlet x = 5\nassert x > 0\n\n# Assertion with custom error message\nlet value = 42\nassert value == 42 \"value must be 42\"\n\n# Assertions are useful for validating function inputs and outputs\nfn divide a b =\n    assert b != 0 \"cannot divide by zero\"\n    a / b\n\ndivide 10 2\n\n# Assertions help catch errors early in development\nlet result = divide 10 2\nassert result == 5 \"expected result to be 5\"\n"
            )
        );
    }
    #[test]
    fn wat() {
        ss!(
            "example_09_assertions_wat",
            t::wat(
                "# Assertions - Runtime Checks\n# \n# The assert macro allows you to verify conditions at runtime\n# and provides detailed error messages when assertions fail.\n\n# Basic assertion - verifies a condition is true\nlet x = 5\nassert x > 0\n\n# Assertion with custom error message\nlet value = 42\nassert value == 42 \"value must be 42\"\n\n# Assertions are useful for validating function inputs and outputs\nfn divide a b =\n    assert b != 0 \"cannot divide by zero\"\n    a / b\n\ndivide 10 2\n\n# Assertions help catch errors early in development\nlet result = divide 10 2\nassert result == 5 \"expected result to be 5\"\n"
            )
        );
    }
}
mod pipeline_basic {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "pipeline_basic",
            t::eval_all("fn add x y = x + y\nfn double x = x * 2\n\n5 |> add 3\n10 |> double\n"),
            "fn add x y = x + y\nfn double x = x * 2\n\n5 |> add 3\n10 |> double\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "pipeline_basic_ast",
            t::ast("fn add x y = x + y\nfn double x = x * 2\n\n5 |> add 3\n10 |> double\n"),
            "fn add x y = x + y\nfn double x = x * 2\n\n5 |> add 3\n10 |> double\n"
        );
    }
    #[test]
    fn ir() {
        ss!(
            "pipeline_basic_ir",
            t::ir("fn add x y = x + y\nfn double x = x * 2\n\n5 |> add 3\n10 |> double\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "pipeline_basic_wat",
            t::wat("fn add x y = x + y\nfn double x = x * 2\n\n5 |> add 3\n10 |> double\n")
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
    #[test]
    fn ir() {
        ss!("record_nested_ir", t::ir("{ a = { b = 1 } }\n"));
    }
    #[test]
    fn wat() {
        ss!("record_nested_wat", t::wat("{ a = { b = 1 } }\n"));
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
    #[test]
    fn ir() {
        ss!(
            "measure_velocity_ir",
            t::ir(
                "measure meter\nmeasure second\nlet distance = meter 100\nlet time = second 10\nlet velocity = distance / time\nvelocity\n"
            )
        );
    }
    #[test]
    fn wat() {
        ss!(
            "measure_velocity_wat",
            t::wat(
                "measure meter\nmeasure second\nlet distance = meter 100\nlet time = second 10\nlet velocity = distance / time\nvelocity\n"
            )
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
    #[test]
    fn ir() {
        ss!("arith_sub_ir", t::ir("10 - 3\n"));
    }
    #[test]
    fn wat() {
        ss!("arith_sub_wat", t::wat("10 - 3\n"));
    }
}
mod pipeline_comprehensive {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "pipeline_comprehensive",
            t::eval_all(
                "fn add x y = x + y\nfn mul x y = x * y\nfn square x = x * x\n\n5 |> square\n10 |> add 5\n2 |> square |> add 3\n1 |> add 2 |> mul 3 |> square\n"
            ),
            "fn add x y = x + y\nfn mul x y = x * y\nfn square x = x * x\n\n5 |> square\n10 |> add 5\n2 |> square |> add 3\n1 |> add 2 |> mul 3 |> square\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "pipeline_comprehensive_ast",
            t::ast(
                "fn add x y = x + y\nfn mul x y = x * y\nfn square x = x * x\n\n5 |> square\n10 |> add 5\n2 |> square |> add 3\n1 |> add 2 |> mul 3 |> square\n"
            ),
            "fn add x y = x + y\nfn mul x y = x * y\nfn square x = x * x\n\n5 |> square\n10 |> add 5\n2 |> square |> add 3\n1 |> add 2 |> mul 3 |> square\n"
        );
    }
    #[test]
    fn ir() {
        ss!(
            "pipeline_comprehensive_ir",
            t::ir(
                "fn add x y = x + y\nfn mul x y = x * y\nfn square x = x * x\n\n5 |> square\n10 |> add 5\n2 |> square |> add 3\n1 |> add 2 |> mul 3 |> square\n"
            )
        );
    }
    #[test]
    fn wat() {
        ss!(
            "pipeline_comprehensive_wat",
            t::wat(
                "fn add x y = x + y\nfn mul x y = x * y\nfn square x = x * x\n\n5 |> square\n10 |> add 5\n2 |> square |> add 3\n1 |> add 2 |> mul 3 |> square\n"
            )
        );
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
    #[test]
    fn ir() {
        ss!("error_undefined_ir", t::ir("undefined_var\n"));
    }
    #[test]
    fn wat() {
        ss!("error_undefined_wat", t::wat("undefined_var\n"));
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
    #[test]
    fn ir() {
        ss!("lit_string_ir", t::ir("\"hello world\"\n"));
    }
    #[test]
    fn wat() {
        ss!("lit_string_wat", t::wat("\"hello world\"\n"));
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
    #[test]
    fn ir() {
        ss!(
            "measure_multiply_ir",
            t::ir("measure inch\nmeasure foot = inch 12\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "measure_multiply_wat",
            t::wat("measure inch\nmeasure foot = inch 12\n")
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
    #[test]
    fn ir() {
        ss!("arith_mixed_ir", t::ir("1 + 2.5\n"));
    }
    #[test]
    fn wat() {
        ss!("arith_mixed_wat", t::wat("1 + 2.5\n"));
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
    #[test]
    fn ir() {
        ss!(
            "measure_conversion_ir",
            t::ir(
                "measure millimeter  \nmeasure inch = millimeter 25.4\nlet x = 25.4millimeter\nlet y = 1inch\nx\ny\n"
            )
        );
    }
    #[test]
    fn wat() {
        ss!(
            "measure_conversion_wat",
            t::wat(
                "measure millimeter  \nmeasure inch = millimeter 25.4\nlet x = 25.4millimeter\nlet y = 1inch\nx\ny\n"
            )
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
    #[test]
    fn ir() {
        ss!("let_reassign_ir", t::ir("let x = 1\nx = 2\nx\n"));
    }
    #[test]
    fn wat() {
        ss!("let_reassign_wat", t::wat("let x = 1\nx = 2\nx\n"));
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
    #[test]
    fn ir() {
        ss!("op_assign_ir", t::ir("let add_op = +\nadd_op 1 2\n"));
    }
    #[test]
    fn wat() {
        ss!("op_assign_wat", t::wat("let add_op = +\nadd_op 1 2\n"));
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
    #[test]
    fn ir() {
        ss!(
            "example_01_welcome_ir",
            t::ir(
                "# Welcome to Cadenza!\n# A functional language with units of measure\n\n# Try some basic expressions\n42\n3.14159\n1 + 2 * 3\n\n# Define variables\nlet name = \"Cadenza\"\nlet version = 0.1\n\n# Create functions\nfn square x = x * x\nsquare 5\n"
            )
        );
    }
    #[test]
    fn wat() {
        ss!(
            "example_01_welcome_wat",
            t::wat(
                "# Welcome to Cadenza!\n# A functional language with units of measure\n\n# Try some basic expressions\n42\n3.14159\n1 + 2 * 3\n\n# Define variables\nlet name = \"Cadenza\"\nlet version = 0.1\n\n# Create functions\nfn square x = x * x\nsquare 5\n"
            )
        );
    }
}
mod assert_fail_with_message {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "assert_fail_with_message",
            t::eval_all("let v = 1\nassert v == 2 \"v should be 2\"\n"),
            "let v = 1\nassert v == 2 \"v should be 2\"\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "assert_fail_with_message_ast",
            t::ast("let v = 1\nassert v == 2 \"v should be 2\"\n"),
            "let v = 1\nassert v == 2 \"v should be 2\"\n"
        );
    }
    #[test]
    fn ir() {
        ss!(
            "assert_fail_with_message_ir",
            t::ir("let v = 1\nassert v == 2 \"v should be 2\"\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "assert_fail_with_message_wat",
            t::wat("let v = 1\nassert v == 2 \"v should be 2\"\n")
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
    #[test]
    fn ir() {
        ss!(
            "measure_dimension_mismatch_ir",
            t::ir("measure meter\n1meter * 2meter + 3meter\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "measure_dimension_mismatch_wat",
            t::wat("measure meter\n1meter * 2meter + 3meter\n")
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
    #[test]
    fn ir() {
        ss!("let_multi_ir", t::ir("let x = 1\nlet y = 2\nx + y\n"));
    }
    #[test]
    fn wat() {
        ss!("let_multi_wat", t::wat("let x = 1\nlet y = 2\nx + y\n"));
    }
}
mod assert_with_message {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "assert_with_message",
            t::eval_all("let v = 1\nassert v == 1 \"expected v to be one\"\n"),
            "let v = 1\nassert v == 1 \"expected v to be one\"\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "assert_with_message_ast",
            t::ast("let v = 1\nassert v == 1 \"expected v to be one\"\n"),
            "let v = 1\nassert v == 1 \"expected v to be one\"\n"
        );
    }
    #[test]
    fn ir() {
        ss!(
            "assert_with_message_ir",
            t::ir("let v = 1\nassert v == 1 \"expected v to be one\"\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "assert_with_message_wat",
            t::wat("let v = 1\nassert v == 1 \"expected v to be one\"\n")
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
    #[test]
    fn ir() {
        ss!("let_expr_ir", t::ir("let x = 1 + 2\nx\n"));
    }
    #[test]
    fn wat() {
        ss!("let_expr_wat", t::wat("let x = 1 + 2\nx\n"));
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
    #[test]
    fn ir() {
        ss!(
            "op_override_ir",
            t::ir("let my_buggy_add = *\nlet + = my_buggy_add\n1 + 2\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "op_override_wat",
            t::wat("let my_buggy_add = *\nlet + = my_buggy_add\n1 + 2\n")
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
    #[test]
    fn ir() {
        ss!("arith_float_ir", t::ir("1.5 + 2.5\n"));
    }
    #[test]
    fn wat() {
        ss!("arith_float_wat", t::wat("1.5 + 2.5\n"));
    }
}
mod if_simple {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "if_simple",
            t::eval_all(
                "# Test match expression\n\n# Basic true pattern - single line syntax\nlet result1 = match true (true -> 42) (false -> 0)\nassert result1 == 42\n\n# Basic false pattern\nlet result2 = match false (true -> 42) (false -> 0)\nassert result2 == 0\n\n# Match with comparison\nlet x = 5\nlet result3 = match x > 0 (true -> \"positive\") (false -> \"negative\")\nassert result3 == \"positive\"\n\n# Match with comparison (false case)\nlet y = -3\nlet result4 = match y > 0 (true -> \"positive\") (false -> \"negative\")\nassert result4 == \"negative\"\n\n# Nested match expressions\nlet z = 10\nlet result5 = match z > 5 (true -> match z > 15 (true -> \"very large\") (false -> \"large\")) (false -> \"small\")\nassert result5 == \"large\"\n"
            ),
            "# Test match expression\n\n# Basic true pattern - single line syntax\nlet result1 = match true (true -> 42) (false -> 0)\nassert result1 == 42\n\n# Basic false pattern\nlet result2 = match false (true -> 42) (false -> 0)\nassert result2 == 0\n\n# Match with comparison\nlet x = 5\nlet result3 = match x > 0 (true -> \"positive\") (false -> \"negative\")\nassert result3 == \"positive\"\n\n# Match with comparison (false case)\nlet y = -3\nlet result4 = match y > 0 (true -> \"positive\") (false -> \"negative\")\nassert result4 == \"negative\"\n\n# Nested match expressions\nlet z = 10\nlet result5 = match z > 5 (true -> match z > 15 (true -> \"very large\") (false -> \"large\")) (false -> \"small\")\nassert result5 == \"large\"\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "if_simple_ast",
            t::ast(
                "# Test match expression\n\n# Basic true pattern - single line syntax\nlet result1 = match true (true -> 42) (false -> 0)\nassert result1 == 42\n\n# Basic false pattern\nlet result2 = match false (true -> 42) (false -> 0)\nassert result2 == 0\n\n# Match with comparison\nlet x = 5\nlet result3 = match x > 0 (true -> \"positive\") (false -> \"negative\")\nassert result3 == \"positive\"\n\n# Match with comparison (false case)\nlet y = -3\nlet result4 = match y > 0 (true -> \"positive\") (false -> \"negative\")\nassert result4 == \"negative\"\n\n# Nested match expressions\nlet z = 10\nlet result5 = match z > 5 (true -> match z > 15 (true -> \"very large\") (false -> \"large\")) (false -> \"small\")\nassert result5 == \"large\"\n"
            ),
            "# Test match expression\n\n# Basic true pattern - single line syntax\nlet result1 = match true (true -> 42) (false -> 0)\nassert result1 == 42\n\n# Basic false pattern\nlet result2 = match false (true -> 42) (false -> 0)\nassert result2 == 0\n\n# Match with comparison\nlet x = 5\nlet result3 = match x > 0 (true -> \"positive\") (false -> \"negative\")\nassert result3 == \"positive\"\n\n# Match with comparison (false case)\nlet y = -3\nlet result4 = match y > 0 (true -> \"positive\") (false -> \"negative\")\nassert result4 == \"negative\"\n\n# Nested match expressions\nlet z = 10\nlet result5 = match z > 5 (true -> match z > 15 (true -> \"very large\") (false -> \"large\")) (false -> \"small\")\nassert result5 == \"large\"\n"
        );
    }
    #[test]
    fn ir() {
        ss!(
            "if_simple_ir",
            t::ir(
                "# Test match expression\n\n# Basic true pattern - single line syntax\nlet result1 = match true (true -> 42) (false -> 0)\nassert result1 == 42\n\n# Basic false pattern\nlet result2 = match false (true -> 42) (false -> 0)\nassert result2 == 0\n\n# Match with comparison\nlet x = 5\nlet result3 = match x > 0 (true -> \"positive\") (false -> \"negative\")\nassert result3 == \"positive\"\n\n# Match with comparison (false case)\nlet y = -3\nlet result4 = match y > 0 (true -> \"positive\") (false -> \"negative\")\nassert result4 == \"negative\"\n\n# Nested match expressions\nlet z = 10\nlet result5 = match z > 5 (true -> match z > 15 (true -> \"very large\") (false -> \"large\")) (false -> \"small\")\nassert result5 == \"large\"\n"
            )
        );
    }
    #[test]
    fn wat() {
        ss!(
            "if_simple_wat",
            t::wat(
                "# Test match expression\n\n# Basic true pattern - single line syntax\nlet result1 = match true (true -> 42) (false -> 0)\nassert result1 == 42\n\n# Basic false pattern\nlet result2 = match false (true -> 42) (false -> 0)\nassert result2 == 0\n\n# Match with comparison\nlet x = 5\nlet result3 = match x > 0 (true -> \"positive\") (false -> \"negative\")\nassert result3 == \"positive\"\n\n# Match with comparison (false case)\nlet y = -3\nlet result4 = match y > 0 (true -> \"positive\") (false -> \"negative\")\nassert result4 == \"negative\"\n\n# Nested match expressions\nlet z = 10\nlet result5 = match z > 5 (true -> match z > 15 (true -> \"very large\") (false -> \"large\")) (false -> \"small\")\nassert result5 == \"large\"\n"
            )
        );
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
    #[test]
    fn ir() {
        ss!("arith_div_ir", t::ir("20 / 4\n"));
    }
    #[test]
    fn wat() {
        ss!("arith_div_wat", t::wat("20 / 4\n"));
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
    #[test]
    fn ir() {
        ss!(
            "error_cmp_type_mismatch_gte_ir",
            t::ir("# Test that >= errors on type mismatch\n200 >= \"qux\"\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "error_cmp_type_mismatch_gte_wat",
            t::wat("# Test that >= errors on type mismatch\n200 >= \"qux\"\n")
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
    #[test]
    fn ir() {
        ss!(
            "measure_suffix_ir",
            t::ir("measure meter\nlet x = 25.4meter\nx\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "measure_suffix_wat",
            t::wat("measure meter\nlet x = 25.4meter\nx\n")
        );
    }
}
mod field_assign_with_expr {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "field_assign_with_expr",
            t::eval_all("let point = { x = 10, y = 20 }\npoint.x = point.x + 5\npoint\n"),
            "let point = { x = 10, y = 20 }\npoint.x = point.x + 5\npoint\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "field_assign_with_expr_ast",
            t::ast("let point = { x = 10, y = 20 }\npoint.x = point.x + 5\npoint\n"),
            "let point = { x = 10, y = 20 }\npoint.x = point.x + 5\npoint\n"
        );
    }
    #[test]
    fn ir() {
        ss!(
            "field_assign_with_expr_ir",
            t::ir("let point = { x = 10, y = 20 }\npoint.x = point.x + 5\npoint\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "field_assign_with_expr_wat",
            t::wat("let point = { x = 10, y = 20 }\npoint.x = point.x + 5\npoint\n")
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
    #[test]
    fn ir() {
        ss!(
            "error_cmp_type_mismatch_lte_ir",
            t::ir("# Test that <= errors on type mismatch\n\"bar\" <= 10\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "error_cmp_type_mismatch_lte_wat",
            t::wat("# Test that <= errors on type mismatch\n\"bar\" <= 10\n")
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
    #[test]
    fn ir() {
        ss!(
            "block_nested_ir",
            t::ir(
                "let foo =\n    let bar =\n        let baz =\n            1\n        baz\n    bar\nfoo\n"
            )
        );
    }
    #[test]
    fn wat() {
        ss!(
            "block_nested_wat",
            t::wat(
                "let foo =\n    let bar =\n        let baz =\n            1\n        baz\n    bar\nfoo\n"
            )
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
    #[test]
    fn ir() {
        ss!(
            "fn_hoisting_ir",
            t::ir("add 2 3\nfn add x y = x + y\nadd 2 3\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "fn_hoisting_wat",
            t::wat("add 2 3\nfn add x y = x + y\nadd 2 3\n")
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
    #[test]
    fn ir() {
        ss!(
            "example_07_measures_ir",
            t::ir(
                "# Units of Measure\n# Define and use physical units\n\n# Define base units\nmeasure meter\nmeasure second\n\n# Use base units\n10meter\n5second\n\n# Derived units\nmeasure kilometer = meter 1000\n2kilometer\n\n# Convert between units\nlet distance = 5000meter\nlet km = 5kilometer\ndistance\nkm\n\n# Unit arithmetic\nlet speed = 100meter / 10second\nspeed\n"
            )
        );
    }
    #[test]
    fn wat() {
        ss!(
            "example_07_measures_wat",
            t::wat(
                "# Units of Measure\n# Define and use physical units\n\n# Define base units\nmeasure meter\nmeasure second\n\n# Use base units\n10meter\n5second\n\n# Derived units\nmeasure kilometer = meter 1000\n2kilometer\n\n# Convert between units\nlet distance = 5000meter\nlet km = 5kilometer\ndistance\nkm\n\n# Unit arithmetic\nlet speed = 100meter / 10second\nspeed\n"
            )
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
    #[test]
    fn ir() {
        ss!(
            "example_04_comparison_ir",
            t::ir(
                "# Comparison Operators\n# Compare numbers with ==, !=, <, >, <=, >=\n\n# Equality\n5 == 5\n5 != 3\n\n# Ordering\n10 > 5\n3 < 7\n5 <= 5\n10 >= 10\n"
            )
        );
    }
    #[test]
    fn wat() {
        ss!(
            "example_04_comparison_wat",
            t::wat(
                "# Comparison Operators\n# Compare numbers with ==, !=, <, >, <=, >=\n\n# Equality\n5 == 5\n5 != 3\n\n# Ordering\n10 > 5\n3 < 7\n5 <= 5\n10 >= 10\n"
            )
        );
    }
}
mod field_assign_missing_field {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "field_assign_missing_field",
            t::eval_all("let point = { x = 10, y = 20 }\npoint.z = 30\n"),
            "let point = { x = 10, y = 20 }\npoint.z = 30\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "field_assign_missing_field_ast",
            t::ast("let point = { x = 10, y = 20 }\npoint.z = 30\n"),
            "let point = { x = 10, y = 20 }\npoint.z = 30\n"
        );
    }
    #[test]
    fn ir() {
        ss!(
            "field_assign_missing_field_ir",
            t::ir("let point = { x = 10, y = 20 }\npoint.z = 30\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "field_assign_missing_field_wat",
            t::wat("let point = { x = 10, y = 20 }\npoint.z = 30\n")
        );
    }
}
mod fn_match_phi {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "fn_match_phi",
            t::eval_all(
                "# Test function with match that generates phi nodes\n\nfn abs x = match x > 0 (true -> x) (false -> 0 - x)\n\nabs 5\nabs (-3)\n"
            ),
            "# Test function with match that generates phi nodes\n\nfn abs x = match x > 0 (true -> x) (false -> 0 - x)\n\nabs 5\nabs (-3)\n"
        );
    }
    #[test]
    fn ast() {
        s!(
            "fn_match_phi_ast",
            t::ast(
                "# Test function with match that generates phi nodes\n\nfn abs x = match x > 0 (true -> x) (false -> 0 - x)\n\nabs 5\nabs (-3)\n"
            ),
            "# Test function with match that generates phi nodes\n\nfn abs x = match x > 0 (true -> x) (false -> 0 - x)\n\nabs 5\nabs (-3)\n"
        );
    }
    #[test]
    fn ir() {
        ss!(
            "fn_match_phi_ir",
            t::ir(
                "# Test function with match that generates phi nodes\n\nfn abs x = match x > 0 (true -> x) (false -> 0 - x)\n\nabs 5\nabs (-3)\n"
            )
        );
    }
    #[test]
    fn wat() {
        ss!(
            "fn_match_phi_wat",
            t::wat(
                "# Test function with match that generates phi nodes\n\nfn abs x = match x > 0 (true -> x) (false -> 0 - x)\n\nabs 5\nabs (-3)\n"
            )
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
    #[test]
    fn ir() {
        ss!("multi_expr_ir", t::ir("1\n2\n3\n"));
    }
    #[test]
    fn wat() {
        ss!("multi_expr_wat", t::wat("1\n2\n3\n"));
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
    #[test]
    fn ir() {
        ss!(
            "error_cmp_type_mismatch_eq_ir",
            t::ir("# Test that == errors on type mismatch\n1 == \"hello\"\n")
        );
    }
    #[test]
    fn wat() {
        ss!(
            "error_cmp_type_mismatch_eq_wat",
            t::wat("# Test that == errors on type mismatch\n1 == \"hello\"\n")
        );
    }
}
