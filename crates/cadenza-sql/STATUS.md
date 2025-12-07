# cadenza-sql Implementation Status

## Overview

SQL parser that produces Cadenza-compatible AST for interactive books and visualizations.

## Completed Features

- [x] Basic SQL lexer
- [x] Comment parsing (line and block comments)
- [x] SELECT statement parsing
- [x] INSERT statement parsing
- [x] UPDATE statement parsing
- [x] DELETE statement parsing
- [x] CREATE statement parsing
- [x] DROP statement parsing
- [x] ALTER statement parsing
- [x] WHERE clause support
- [x] ORDER BY clause support
- [x] LIMIT clause support
- [x] Expression parsing (identifiers, numbers, strings, operators)
- [x] Test data files
- [x] Build script for snapshot tests
- [x] Documentation (README)

## Future Enhancements

- [ ] JOIN clauses (INNER JOIN, LEFT JOIN, etc.)
- [ ] GROUP BY and HAVING clauses
- [ ] Subquery support
- [ ] UNION, INTERSECT, EXCEPT
- [ ] More complex expressions (CASE, CAST, etc.)
- [ ] Window functions
- [ ] Common Table Expressions (WITH)
- [ ] More SQL dialects (PostgreSQL, MySQL, SQLite specific features)
- [ ] Better error messages for invalid SQL
- [ ] Type inference for SQL expressions

## Notes

- The parser is intentionally simple and permissive
- Focus is on enabling interactive education, not full SQL validation
- Handler macros can implement dialect-specific behavior
- Compatible with Cadenza's evaluation model
