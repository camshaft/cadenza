use crate::testing as t;
use insta::assert_debug_snapshot as s;
mod cmp_gt {
    use super::*;
    #[test]
    fn eval() {
        s!("cmp_gt", t::eval_all("2 > 1\n"), "2 > 1\n");
    }
}
mod example_literals {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "example_literals",
            t::eval_all(
                "# Literal Values\n# Different types of literals\n\n# Integers\n42\n-17\n0\n\n# Floating point\n3.14159\n-2.5\n1.0\n\n# Strings\n\"hello\"\n\"world\"\n\"hello world\"\n"
            ),
            "# Literal Values\n# Different types of literals\n\n# Integers\n42\n-17\n0\n\n# Floating point\n3.14159\n-2.5\n1.0\n\n# Strings\n\"hello\"\n\"world\"\n\"hello world\"\n"
        );
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
}
mod example_welcome {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "example_welcome",
            t::eval_all(
                "# Welcome to Cadenza!\n# A functional language with units of measure\n\n# Try some basic expressions\n42\n3.14159\n1 + 2 * 3\n\n# Define variables\nlet name = \"Cadenza\"\nlet version = 0.1\n\n# Create functions\nfn square x = x * x\nsquare 5\n"
            ),
            "# Welcome to Cadenza!\n# A functional language with units of measure\n\n# Try some basic expressions\n42\n3.14159\n1 + 2 * 3\n\n# Define variables\nlet name = \"Cadenza\"\nlet version = 0.1\n\n# Create functions\nfn square x = x * x\nsquare 5\n"
        );
    }
}
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
}
mod lit_float {
    use super::*;
    #[test]
    fn eval() {
        s!("lit_float", t::eval_all("3.14\n"), "3.14\n");
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
}
mod arith_mul {
    use super::*;
    #[test]
    fn eval() {
        s!("arith_mul", t::eval_all("4 * 5\n"), "4 * 5\n");
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
}
mod example_measures {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "example_measures",
            t::eval_all(
                "# Units of Measure\n# Define and use physical units\n\n# Define base units\nmeasure meter\nmeasure second\n\n# Use base units\n10meter\n5second\n\n# Derived units\nmeasure kilometer = meter 1000\n2kilometer\n\n# Convert between units\nlet distance = 5000meter\nlet km = 5kilometer\ndistance\nkm\n\n# Unit arithmetic\nlet speed = 100meter / 10second\nspeed\n"
            ),
            "# Units of Measure\n# Define and use physical units\n\n# Define base units\nmeasure meter\nmeasure second\n\n# Use base units\n10meter\n5second\n\n# Derived units\nmeasure kilometer = meter 1000\n2kilometer\n\n# Convert between units\nlet distance = 5000meter\nlet km = 5kilometer\ndistance\nkm\n\n# Unit arithmetic\nlet speed = 100meter / 10second\nspeed\n"
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
}
mod arith_float_mul {
    use super::*;
    #[test]
    fn eval() {
        s!("arith_float_mul", t::eval_all("3.0 * 2.0\n"), "3.0 * 2.0\n");
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
mod cmp_le {
    use super::*;
    #[test]
    fn eval() {
        s!("cmp_le", t::eval_all("1 <= 1\n"), "1 <= 1\n");
    }
}
mod example_variables {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "example_variables",
            t::eval_all(
                "# Variables with let\n# Define and use variables\n\n# Simple binding\nlet x = 42\nx\n\n# Multiple bindings\nlet a = 1\nlet b = 2\na + b\n\n# Using expressions\nlet result = 10 * 5 + 3\nresult\n\n# Variable reassignment\nlet counter = 0\nlet counter = counter + 1\nlet counter = counter + 1\ncounter\n"
            ),
            "# Variables with let\n# Define and use variables\n\n# Simple binding\nlet x = 42\nx\n\n# Multiple bindings\nlet a = 1\nlet b = 2\na + b\n\n# Using expressions\nlet result = 10 * 5 + 3\nresult\n\n# Variable reassignment\nlet counter = 0\nlet counter = counter + 1\nlet counter = counter + 1\ncounter\n"
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
mod cmp_ge {
    use super::*;
    #[test]
    fn eval() {
        s!("cmp_ge", t::eval_all("1 >= 1\n"), "1 >= 1\n");
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
}
mod cmp_lt {
    use super::*;
    #[test]
    fn eval() {
        s!("cmp_lt", t::eval_all("1 < 2\n"), "1 < 2\n");
    }
}
mod arith_mixed_rev {
    use super::*;
    #[test]
    fn eval() {
        s!("arith_mixed_rev", t::eval_all("2.5 + 1\n"), "2.5 + 1\n");
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
}
mod cmp_ne {
    use super::*;
    #[test]
    fn eval() {
        s!("cmp_ne", t::eval_all("1 != 2\n"), "1 != 2\n");
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
}
mod arith_mixed {
    use super::*;
    #[test]
    fn eval() {
        s!("arith_mixed", t::eval_all("1 + 2.5\n"), "1 + 2.5\n");
    }
}
mod example_comparison {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "example_comparison",
            t::eval_all(
                "# Comparison Operators\n# Compare numbers with ==, !=, <, >, <=, >=\n\n# Equality\n5 == 5\n5 != 3\n\n# Ordering\n10 > 5\n3 < 7\n5 <= 5\n10 >= 10\n"
            ),
            "# Comparison Operators\n# Compare numbers with ==, !=, <, >, <=, >=\n\n# Equality\n5 == 5\n5 != 3\n\n# Ordering\n10 > 5\n3 < 7\n5 <= 5\n10 >= 10\n"
        );
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
}
mod arith_float {
    use super::*;
    #[test]
    fn eval() {
        s!("arith_float", t::eval_all("1.5 + 2.5\n"), "1.5 + 2.5\n");
    }
}
mod example_functions {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "example_functions",
            t::eval_all(
                "# Functions\n# Define and call functions\n\n# Simple function\nfn double x = x * 2\ndouble 5\n\n# Multi-parameter function\nfn add x y = x + y\nadd 3 7\n\n# Nested functions\nfn make_adder n = fn x = x + n\nlet add5 = make_adder 5\nadd5 10\n\n# Function with closure\nlet outer = 100\nfn capture = outer + 1\ncapture\n"
            ),
            "# Functions\n# Define and call functions\n\n# Simple function\nfn double x = x * 2\ndouble 5\n\n# Multi-parameter function\nfn add x y = x + y\nadd 3 7\n\n# Nested functions\nfn make_adder n = fn x = x + n\nlet add5 = make_adder 5\nadd5 10\n\n# Function with closure\nlet outer = 100\nfn capture = outer + 1\ncapture\n"
        );
    }
}
mod arith_div {
    use super::*;
    #[test]
    fn eval() {
        s!("arith_div", t::eval_all("20 / 4\n"), "20 / 4\n");
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
}
mod example_arithmetic {
    use super::*;
    #[test]
    fn eval() {
        s!(
            "example_arithmetic",
            t::eval_all(
                "# Arithmetic Operations\n# Basic math with integers and floats\n\n# Addition and subtraction\n1 + 2\n10 - 3\n\n# Multiplication and division\n4 * 5\n20 / 4\n\n# Operator precedence\n2 + 3 * 4\n(2 + 3) * 4\n\n# Floating point\n3.14 * 2.0\n10.5 / 2.0\n"
            ),
            "# Arithmetic Operations\n# Basic math with integers and floats\n\n# Addition and subtraction\n1 + 2\n10 - 3\n\n# Multiplication and division\n4 * 5\n20 / 4\n\n# Operator precedence\n2 + 3 * 4\n(2 + 3) * 4\n\n# Floating point\n3.14 * 2.0\n10.5 / 2.0\n"
        );
    }
}
mod multi_expr {
    use super::*;
    #[test]
    fn eval() {
        s!("multi_expr", t::eval_all("1\n2\n3\n"), "1\n2\n3\n");
    }
}
