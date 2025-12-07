use crate::{parse, testing::verify_cst_coverage};
use insta::assert_debug_snapshot as s;

mod create_table {
    use super::*;
    #[test]
    fn cst() {
        let sql = "CREATE TABLE users (\n    id INTEGER PRIMARY KEY,\n    name TEXT NOT NULL,\n    email TEXT UNIQUE,\n    age INTEGER\n);\n";
        let parse = parse(sql);
        let cst = parse.syntax();

        // Verify CST span coverage and token text accuracy
        verify_cst_coverage(sql);

        s!(
            "create_table_cst",
            &cst,
            "CREATE TABLE users (\n    id INTEGER PRIMARY KEY,\n    name TEXT NOT NULL,\n    email TEXT UNIQUE,\n    age INTEGER\n);\n"
        );
    }
    #[test]
    fn ast() {
        let sql = "CREATE TABLE users (\n    id INTEGER PRIMARY KEY,\n    name TEXT NOT NULL,\n    email TEXT UNIQUE,\n    age INTEGER\n);\n";
        let parse = parse(sql);
        let root = parse.ast();
        s!(
            "create_table_ast",
            root,
            "CREATE TABLE users (\n    id INTEGER PRIMARY KEY,\n    name TEXT NOT NULL,\n    email TEXT UNIQUE,\n    age INTEGER\n);\n"
        );
    }
}
mod delete {
    use super::*;
    #[test]
    fn cst() {
        let sql = "DELETE FROM users WHERE age < 18;\n";
        let parse = parse(sql);
        let cst = parse.syntax();

        // Verify CST span coverage and token text accuracy
        verify_cst_coverage(sql);

        s!("delete_cst", &cst, "DELETE FROM users WHERE age < 18;\n");
    }
    #[test]
    fn ast() {
        let sql = "DELETE FROM users WHERE age < 18;\n";
        let parse = parse(sql);
        let root = parse.ast();
        s!("delete_ast", root, "DELETE FROM users WHERE age < 18;\n");
    }
}
mod insert {
    use super::*;
    #[test]
    fn cst() {
        let sql = "INSERT INTO users (name, email, age) VALUES ('John', 'john@example.com', 25);\n";
        let parse = parse(sql);
        let cst = parse.syntax();

        // Verify CST span coverage and token text accuracy
        verify_cst_coverage(sql);

        s!(
            "insert_cst",
            &cst,
            "INSERT INTO users (name, email, age) VALUES ('John', 'john@example.com', 25);\n"
        );
    }
    #[test]
    fn ast() {
        let sql = "INSERT INTO users (name, email, age) VALUES ('John', 'john@example.com', 25);\n";
        let parse = parse(sql);
        let root = parse.ast();
        s!(
            "insert_ast",
            root,
            "INSERT INTO users (name, email, age) VALUES ('John', 'john@example.com', 25);\n"
        );
    }
}
mod select_where {
    use super::*;
    #[test]
    fn cst() {
        let sql = "SELECT id, name, email FROM users WHERE age > 18;\n";
        let parse = parse(sql);
        let cst = parse.syntax();

        // Verify CST span coverage and token text accuracy
        verify_cst_coverage(sql);

        s!(
            "select_where_cst",
            &cst,
            "SELECT id, name, email FROM users WHERE age > 18;\n"
        );
    }
    #[test]
    fn ast() {
        let sql = "SELECT id, name, email FROM users WHERE age > 18;\n";
        let parse = parse(sql);
        let root = parse.ast();
        s!(
            "select_where_ast",
            root,
            "SELECT id, name, email FROM users WHERE age > 18;\n"
        );
    }
}
mod simple_select {
    use super::*;
    #[test]
    fn cst() {
        let sql = "SELECT * FROM users;\n";
        let parse = parse(sql);
        let cst = parse.syntax();

        // Verify CST span coverage and token text accuracy
        verify_cst_coverage(sql);

        s!("simple_select_cst", &cst, "SELECT * FROM users;\n");
    }
    #[test]
    fn ast() {
        let sql = "SELECT * FROM users;\n";
        let parse = parse(sql);
        let root = parse.ast();
        s!("simple_select_ast", root, "SELECT * FROM users;\n");
    }
}
mod update {
    use super::*;
    #[test]
    fn cst() {
        let sql = "UPDATE users SET age = 26 WHERE name = 'John';\n";
        let parse = parse(sql);
        let cst = parse.syntax();

        // Verify CST span coverage and token text accuracy
        verify_cst_coverage(sql);

        s!(
            "update_cst",
            &cst,
            "UPDATE users SET age = 26 WHERE name = 'John';\n"
        );
    }
    #[test]
    fn ast() {
        let sql = "UPDATE users SET age = 26 WHERE name = 'John';\n";
        let parse = parse(sql);
        let root = parse.ast();
        s!(
            "update_ast",
            root,
            "UPDATE users SET age = 26 WHERE name = 'John';\n"
        );
    }
}
mod with_comments {
    use super::*;
    #[test]
    fn cst() {
        let sql = "-- This is a comment\nSELECT * FROM users;\n\n/* This is a block comment */\nSELECT id, name FROM users WHERE age > 21;\n";
        let parse = parse(sql);
        let cst = parse.syntax();

        // Verify CST span coverage and token text accuracy
        verify_cst_coverage(sql);

        s!(
            "with_comments_cst",
            &cst,
            "-- This is a comment\nSELECT * FROM users;\n\n/* This is a block comment */\nSELECT id, name FROM users WHERE age > 21;\n"
        );
    }
    #[test]
    fn ast() {
        let sql = "-- This is a comment\nSELECT * FROM users;\n\n/* This is a block comment */\nSELECT id, name FROM users WHERE age > 21;\n";
        let parse = parse(sql);
        let root = parse.ast();
        s!(
            "with_comments_ast",
            root,
            "-- This is a comment\nSELECT * FROM users;\n\n/* This is a block comment */\nSELECT id, name FROM users WHERE age > 21;\n"
        );
    }
}
