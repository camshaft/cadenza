// Cadenza Syntax Module
// 
// This module defines the abstract syntax for Cadenza programs using
// parenthesized AST notation. Since K doesn't support Pratt parsing,
// programs must be written in this format or translated from concrete syntax.

requires "domains.md"

module CADENZA-SYNTAX
  imports INT-SYNTAX
  imports FLOAT-SYNTAX
  imports STRING-SYNTAX
  imports BOOL-SYNTAX
  imports ID-SYNTAX
  imports LIST

  // =========================================================================
  // Top-level Program
  // =========================================================================
  
  syntax Pgm ::= Stmts
  syntax Stmts ::= List{Stmt, ";"} [klabel(stmts)]
  
  // =========================================================================
  // Expressions
  // =========================================================================
  
  syntax Expr ::= Value
                | Ident
                | Apply
                | Op
                | Synthetic
  
  syntax Stmt ::= Expr
                | LetStmt
                | FnStmt
                | AssignStmt
  
  // =========================================================================
  // Values (Results of Evaluation)
  // =========================================================================
  
  syntax Value ::= IntValue
                 | FloatValue
                 | RationalValue
                 | StringValue
                 | BoolValue
                 | UnitValue
                 | ClosureValue
  
  syntax IntValue ::= "'Integer'" "(" Int ")" [klabel('Integer)]
  syntax FloatValue ::= "'Float'" "(" Float ")" [klabel('Float)]
  syntax RationalValue ::= "'Rational'" "(" Int "," Int ")" [klabel('Rational)]
  syntax StringValue ::= "'String'" "(" String ")" [klabel('String)]
  syntax BoolValue ::= "'Bool'" "(" Bool ")" [klabel('Bool)]
  syntax UnitValue ::= "'Unit'" "(" ")" [klabel('Unit)]
  
  // Closure: name, params, body, captured environment
  syntax ClosureValue ::= "'Closure'" "(" Id "," Ids "," Expr "," Map ")" [klabel('Closure)]
  
  // =========================================================================
  // Literals
  // =========================================================================
  
  syntax Ident ::= "'Ident'" "(" Id ")" [klabel('Ident)]
  
  // =========================================================================
  // Let Bindings
  // =========================================================================
  
  // let x = expr
  syntax LetStmt ::= "'Let'" "(" Pattern "," Expr ")" [klabel('Let)]
  
  // Patterns for destructuring
  syntax Pattern ::= IdPattern
                   | TuplePattern
                   | WildcardPattern
  
  syntax IdPattern ::= Id
  syntax TuplePattern ::= "'TuplePattern'" "(" Patterns ")" [klabel('TuplePattern)]
  syntax WildcardPattern ::= "'Wildcard'" "(" ")" [klabel('Wildcard)]
  
  syntax Patterns ::= List{Pattern, ","} [klabel(patterns)]
  
  // =========================================================================
  // Reassignment
  // =========================================================================
  
  // x = expr (modifies existing binding)
  syntax AssignStmt ::= "'Assign'" "(" Id "," Expr ")" [klabel('Assign)]
  
  // =========================================================================
  // Function Definition
  // =========================================================================
  
  // fn name param1 param2 ... = body
  syntax FnStmt ::= "'Fn'" "(" Id "," Ids "," Expr ")" [klabel('Fn)]
  
  syntax Ids ::= List{Id, ","} [klabel(ids)]
  
  // =========================================================================
  // Function Application
  // =========================================================================
  
  // Represents function calls: f(a, b, c)
  syntax Apply ::= "'Apply'" "(" Expr "," Exprs ")" [klabel('Apply)]
  
  syntax Exprs ::= List{Expr, ","} [klabel(exprs)]
  
  // =========================================================================
  // Operators
  // =========================================================================
  
  // Operators are special identifiers that get special handling
  syntax Op ::= "'Op'" "(" OpName ")" [klabel('Op)]
  
  syntax OpName ::= "+" | "-" | "*" | "/" | "%"
                  | "==" | "!=" | "<" | "<=" | ">" | ">="
                  | "&&" | "||" | "!"
  
  // =========================================================================
  // Synthetic Nodes
  // =========================================================================
  
  // Synthetic nodes represent semantic concepts not in source
  syntax Synthetic ::= "'Synthetic'" "(" Id ")" [klabel('Synthetic)]
  
  // =========================================================================
  // Match Expression (Control Flow)
  // =========================================================================
  
  syntax Expr ::= MatchExpr
  
  syntax MatchExpr ::= "'Match'" "(" Expr "," MatchArms ")" [klabel('Match)]
  
  syntax MatchArm ::= "'MatchArm'" "(" Pattern "," Expr ")" [klabel('MatchArm)]
  syntax MatchArms ::= List{MatchArm, ","} [klabel(matcharms)]
  
  // =========================================================================
  // Blocks
  // =========================================================================
  
  syntax Expr ::= BlockExpr
  
  syntax BlockExpr ::= "'Block'" "(" Stmts ")" [klabel('Block)]

endmodule
