use crate::{parse, testing::verify_cst_coverage};
use insta::assert_debug_snapshot as s;

mod code_block_params {
    use super::*;
    #[test]
    fn cst() {
        let markdown = "# Interactive Example\n\nThis example shows code with parameters.\n\n```cadenza editable hidden\nlet setup = initialize\n```\n\nThe code above is hidden and editable.\n\n```javascript\nconsole.log(\"Hello\");\n```\n\nDifferent languages are supported!\n";
        let parse = parse(markdown);
        let cst = parse.syntax();

        // Verify CST span coverage and token text accuracy
        verify_cst_coverage(markdown);

        s!(
            "code_block_params_cst",
            &cst,
            "# Interactive Example\n\nThis example shows code with parameters.\n\n```cadenza editable hidden\nlet setup = initialize\n```\n\nThe code above is hidden and editable.\n\n```javascript\nconsole.log(\"Hello\");\n```\n\nDifferent languages are supported!\n"
        );
    }
    #[test]
    fn ast() {
        let markdown = "# Interactive Example\n\nThis example shows code with parameters.\n\n```cadenza editable hidden\nlet setup = initialize\n```\n\nThe code above is hidden and editable.\n\n```javascript\nconsole.log(\"Hello\");\n```\n\nDifferent languages are supported!\n";
        let parse = parse(markdown);
        let root = parse.ast();
        s!(
            "code_block_params_ast",
            root,
            "# Interactive Example\n\nThis example shows code with parameters.\n\n```cadenza editable hidden\nlet setup = initialize\n```\n\nThe code above is hidden and editable.\n\n```javascript\nconsole.log(\"Hello\");\n```\n\nDifferent languages are supported!\n"
        );
    }
}
mod code_blocks {
    use super::*;
    #[test]
    fn cst() {
        let markdown = "# Physics Tutorial\n\nThe range of a projectile is calculated using the formula below.\n\n```cadenza\nmeasure meter\nmeasure degree\nlet velocity = 20\nlet angle = 45\n```\n\nYou can experiment with different values to see how they affect the range!\n";
        let parse = parse(markdown);
        let cst = parse.syntax();

        // Verify CST span coverage and token text accuracy
        verify_cst_coverage(markdown);

        s!(
            "code_blocks_cst",
            &cst,
            "# Physics Tutorial\n\nThe range of a projectile is calculated using the formula below.\n\n```cadenza\nmeasure meter\nmeasure degree\nlet velocity = 20\nlet angle = 45\n```\n\nYou can experiment with different values to see how they affect the range!\n"
        );
    }
    #[test]
    fn ast() {
        let markdown = "# Physics Tutorial\n\nThe range of a projectile is calculated using the formula below.\n\n```cadenza\nmeasure meter\nmeasure degree\nlet velocity = 20\nlet angle = 45\n```\n\nYou can experiment with different values to see how they affect the range!\n";
        let parse = parse(markdown);
        let root = parse.ast();
        s!(
            "code_blocks_ast",
            root,
            "# Physics Tutorial\n\nThe range of a projectile is calculated using the formula below.\n\n```cadenza\nmeasure meter\nmeasure degree\nlet velocity = 20\nlet angle = 45\n```\n\nYou can experiment with different values to see how they affect the range!\n"
        );
    }
}
mod complex {
    use super::*;
    #[test]
    fn cst() {
        let markdown = "# Complex Document\n\nThis document has multiple elements.\n\n## Introduction\n\nWelcome to this tutorial. Here's what we'll cover:\n\n- Topic 1\n- Topic 2\n- Topic 3\n\n## Code Examples\n\nHere's a simple example:\n\n```cadenza\nlet x = 42\n```\n\nAnd another one:\n\n```python\nprint(\"Hello, World!\")\n```\n\n## Conclusion\n\nThat's all for now!\n";
        let parse = parse(markdown);
        let cst = parse.syntax();

        // Verify CST span coverage and token text accuracy
        verify_cst_coverage(markdown);

        s!(
            "complex_cst",
            &cst,
            "# Complex Document\n\nThis document has multiple elements.\n\n## Introduction\n\nWelcome to this tutorial. Here's what we'll cover:\n\n- Topic 1\n- Topic 2\n- Topic 3\n\n## Code Examples\n\nHere's a simple example:\n\n```cadenza\nlet x = 42\n```\n\nAnd another one:\n\n```python\nprint(\"Hello, World!\")\n```\n\n## Conclusion\n\nThat's all for now!\n"
        );
    }
    #[test]
    fn ast() {
        let markdown = "# Complex Document\n\nThis document has multiple elements.\n\n## Introduction\n\nWelcome to this tutorial. Here's what we'll cover:\n\n- Topic 1\n- Topic 2\n- Topic 3\n\n## Code Examples\n\nHere's a simple example:\n\n```cadenza\nlet x = 42\n```\n\nAnd another one:\n\n```python\nprint(\"Hello, World!\")\n```\n\n## Conclusion\n\nThat's all for now!\n";
        let parse = parse(markdown);
        let root = parse.ast();
        s!(
            "complex_ast",
            root,
            "# Complex Document\n\nThis document has multiple elements.\n\n## Introduction\n\nWelcome to this tutorial. Here's what we'll cover:\n\n- Topic 1\n- Topic 2\n- Topic 3\n\n## Code Examples\n\nHere's a simple example:\n\n```cadenza\nlet x = 42\n```\n\nAnd another one:\n\n```python\nprint(\"Hello, World!\")\n```\n\n## Conclusion\n\nThat's all for now!\n"
        );
    }
}
mod headings {
    use super::*;
    #[test]
    fn cst() {
        let markdown = "# Heading 1\n## Heading 2\n### Heading 3\n#### Heading 4\n##### Heading 5\n###### Heading 6\n\nRegular paragraph after headings.\n";
        let parse = parse(markdown);
        let cst = parse.syntax();

        // Verify CST span coverage and token text accuracy
        verify_cst_coverage(markdown);

        s!(
            "headings_cst",
            &cst,
            "# Heading 1\n## Heading 2\n### Heading 3\n#### Heading 4\n##### Heading 5\n###### Heading 6\n\nRegular paragraph after headings.\n"
        );
    }
    #[test]
    fn ast() {
        let markdown = "# Heading 1\n## Heading 2\n### Heading 3\n#### Heading 4\n##### Heading 5\n###### Heading 6\n\nRegular paragraph after headings.\n";
        let parse = parse(markdown);
        let root = parse.ast();
        s!(
            "headings_ast",
            root,
            "# Heading 1\n## Heading 2\n### Heading 3\n#### Heading 4\n##### Heading 5\n###### Heading 6\n\nRegular paragraph after headings.\n"
        );
    }
}
mod lists {
    use super::*;
    #[test]
    fn cst() {
        let markdown =
            "# Shopping List\n\n- Apples\n- Bananas\n- Oranges\n\nSome text after the list.\n";
        let parse = parse(markdown);
        let cst = parse.syntax();

        // Verify CST span coverage and token text accuracy
        verify_cst_coverage(markdown);

        s!(
            "lists_cst",
            &cst,
            "# Shopping List\n\n- Apples\n- Bananas\n- Oranges\n\nSome text after the list.\n"
        );
    }
    #[test]
    fn ast() {
        let markdown =
            "# Shopping List\n\n- Apples\n- Bananas\n- Oranges\n\nSome text after the list.\n";
        let parse = parse(markdown);
        let root = parse.ast();
        s!(
            "lists_ast",
            root,
            "# Shopping List\n\n- Apples\n- Bananas\n- Oranges\n\nSome text after the list.\n"
        );
    }
}
mod simple {
    use super::*;
    #[test]
    fn cst() {
        let markdown = "# Hello World\n\nThis is a simple paragraph.\n";
        let parse = parse(markdown);
        let cst = parse.syntax();

        // Verify CST span coverage and token text accuracy
        verify_cst_coverage(markdown);

        s!(
            "simple_cst",
            &cst,
            "# Hello World\n\nThis is a simple paragraph.\n"
        );
    }
    #[test]
    fn ast() {
        let markdown = "# Hello World\n\nThis is a simple paragraph.\n";
        let parse = parse(markdown);
        let root = parse.ast();
        s!(
            "simple_ast",
            root,
            "# Hello World\n\nThis is a simple paragraph.\n"
        );
    }
}
