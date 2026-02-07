// Cadenza Literals Semantics
//
// This module implements the evaluation semantics for literal values.
// Literals are the simplest expressions - they evaluate to themselves.

requires "../syntax.k"

module CADENZA-LITERALS
  imports CADENZA-SYNTAX
  imports INT
  imports FLOAT
  imports STRING
  imports BOOL

  // =========================================================================
  // Integer Literals
  // =========================================================================
  
  // An integer literal evaluates to itself
  rule <k> 'Integer(I:Int) => 'Integer(I) ... </k>
  
  // =========================================================================
  // Float Literals
  // =========================================================================
  
  // A float literal evaluates to itself
  rule <k> 'Float(F:Float) => 'Float(F) ... </k>
  
  // =========================================================================
  // Rational Literals
  // =========================================================================
  
  // Rationals are created by division, so they're already in normal form
  rule <k> 'Rational(N:Int, D:Int) => 'Rational(N, D) ... </k>
    requires D =/=Int 0
  
  // Division by zero is an error
  rule <k> 'Rational(_:Int, 0) => 'Error("division by zero") ... </k>
  
  // =========================================================================
  // String Literals
  // =========================================================================
  
  // A string literal evaluates to itself
  rule <k> 'String(S:String) => 'String(S) ... </k>
  
  // =========================================================================
  // Boolean Literals
  // =========================================================================
  
  // Boolean literals evaluate to themselves
  rule <k> 'Bool(B:Bool) => 'Bool(B) ... </k>
  
  // =========================================================================
  // Unit Literal
  // =========================================================================
  
  // Unit (empty tuple) evaluates to itself
  rule <k> 'Unit() => 'Unit() ... </k>
  
  // =========================================================================
  // Helper: Check if a value is a Value
  // =========================================================================
  
  syntax Bool ::= isValue(Expr) [function]
  
  rule isValue('Integer(_)) => true
  rule isValue('Float(_)) => true
  rule isValue('Rational(_, _)) => true
  rule isValue('String(_)) => true
  rule isValue('Bool(_)) => true
  rule isValue('Unit()) => true
  rule isValue('Closure(_, _, _, _)) => true
  rule isValue(_) => false [owise]

endmodule
