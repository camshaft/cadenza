// Cadenza Operators Semantics
//
// This module implements the evaluation semantics for operators:
// - Arithmetic operators: +, -, *, /, %
// - Comparison operators: ==, !=, <, <=, >, >=
// - Logical operators: &&, ||, !

requires "../syntax.k"

module CADENZA-OPERATORS
  imports CADENZA-SYNTAX
  imports CADENZA-LITERALS
  imports INT
  imports FLOAT
  imports BOOL

  // =========================================================================
  // Arithmetic Operators
  // =========================================================================
  
  // Integer Addition
  rule <k> 'Apply('Op(+), 'Integer(I1:Int), 'Integer(I2:Int)) 
           => 'Integer(I1 +Int I2) ... </k>
  
  // Integer Subtraction
  rule <k> 'Apply('Op(-), 'Integer(I1:Int), 'Integer(I2:Int)) 
           => 'Integer(I1 -Int I2) ... </k>
  
  // Integer Multiplication
  rule <k> 'Apply('Op(*), 'Integer(I1:Int), 'Integer(I2:Int)) 
           => 'Integer(I1 *Int I2) ... </k>
  
  // Integer Division (produces Rational)
  rule <k> 'Apply('Op(/), 'Integer(I1:Int), 'Integer(I2:Int)) 
           => simplifyRational(I1, I2) ... </k>
    requires I2 =/=Int 0
  
  rule <k> 'Apply('Op(/), 'Integer(_), 'Integer(0)) 
           => 'Error("division by zero") ... </k>
  
  // Integer Modulo
  rule <k> 'Apply('Op(%), 'Integer(I1:Int), 'Integer(I2:Int)) 
           => 'Integer(I1 %Int I2) ... </k>
    requires I2 =/=Int 0
  
  // Float Addition
  rule <k> 'Apply('Op(+), 'Float(F1:Float), 'Float(F2:Float)) 
           => 'Float(F1 +Float F2) ... </k>
  
  // Float Subtraction
  rule <k> 'Apply('Op(-), 'Float(F1:Float), 'Float(F2:Float)) 
           => 'Float(F1 -Float F2) ... </k>
  
  // Float Multiplication
  rule <k> 'Apply('Op(*), 'Float(F1:Float), 'Float(F2:Float)) 
           => 'Float(F1 *Float F2) ... </k>
  
  // Float Division
  rule <k> 'Apply('Op(/), 'Float(F1:Float), 'Float(F2:Float)) 
           => 'Float(F1 /Float F2) ... </k>
  
  // Mixed Integer/Float operations promote to Float
  rule <k> 'Apply('Op(+), 'Integer(I:Int), 'Float(F:Float)) 
           => 'Float(Int2Float(I) +Float F) ... </k>
  
  rule <k> 'Apply('Op(+), 'Float(F:Float), 'Integer(I:Int)) 
           => 'Float(F +Float Int2Float(I)) ... </k>
  
  rule <k> 'Apply('Op(-), 'Integer(I:Int), 'Float(F:Float)) 
           => 'Float(Int2Float(I) -Float F) ... </k>
  
  rule <k> 'Apply('Op(-), 'Float(F:Float), 'Integer(I:Int)) 
           => 'Float(F -Float Int2Float(I)) ... </k>
  
  rule <k> 'Apply('Op(*), 'Integer(I:Int), 'Float(F:Float)) 
           => 'Float(Int2Float(I) *Float F) ... </k>
  
  rule <k> 'Apply('Op(*), 'Float(F:Float), 'Integer(I:Int)) 
           => 'Float(F *Float Int2Float(I)) ... </k>
  
  rule <k> 'Apply('Op(/), 'Integer(I:Int), 'Float(F:Float)) 
           => 'Float(Int2Float(I) /Float F) ... </k>
  
  rule <k> 'Apply('Op(/), 'Float(F:Float), 'Integer(I:Int)) 
           => 'Float(F /Float Int2Float(I)) ... </k>
  
  // Rational Addition
  // (a/b) + (c/d) = (ad + bc) / (bd)
  rule <k> 'Apply('Op(+), 'Rational(N1:Int, D1:Int), 'Rational(N2:Int, D2:Int)) 
           => simplifyRational((N1 *Int D2) +Int (N2 *Int D1), D1 *Int D2) ... </k>
  
  // Rational Subtraction
  rule <k> 'Apply('Op(-), 'Rational(N1:Int, D1:Int), 'Rational(N2:Int, D2:Int)) 
           => simplifyRational((N1 *Int D2) -Int (N2 *Int D1), D1 *Int D2) ... </k>
  
  // Rational Multiplication
  // (a/b) * (c/d) = (ac) / (bd)
  rule <k> 'Apply('Op(*), 'Rational(N1:Int, D1:Int), 'Rational(N2:Int, D2:Int)) 
           => simplifyRational(N1 *Int N2, D1 *Int D2) ... </k>
  
  // Rational Division
  // (a/b) / (c/d) = (ad) / (bc)
  rule <k> 'Apply('Op(/), 'Rational(N1:Int, D1:Int), 'Rational(N2:Int, D2:Int)) 
           => simplifyRational(N1 *Int D2, D1 *Int N2) ... </k>
    requires N2 =/=Int 0
  
  // Mixed Integer/Rational operations
  rule <k> 'Apply('Op(+), 'Integer(I:Int), 'Rational(N:Int, D:Int)) 
           => simplifyRational((I *Int D) +Int N, D) ... </k>
  
  rule <k> 'Apply('Op(+), 'Rational(N:Int, D:Int), 'Integer(I:Int)) 
           => simplifyRational(N +Int (I *Int D), D) ... </k>
  
  // Mixed Rational/Float operations promote to Float
  rule <k> 'Apply('Op(+), 'Rational(N:Int, D:Int), 'Float(F:Float)) 
           => 'Float(rationalToFloat(N, D) +Float F) ... </k>
  
  rule <k> 'Apply('Op(+), 'Float(F:Float), 'Rational(N:Int, D:Int)) 
           => 'Float(F +Float rationalToFloat(N, D)) ... </k>
  
  // =========================================================================
  // Comparison Operators
  // =========================================================================
  
  // Integer Equality
  rule <k> 'Apply('Op(==), 'Integer(I1:Int), 'Integer(I2:Int)) 
           => 'Bool(I1 ==Int I2) ... </k>
  
  // Integer Inequality
  rule <k> 'Apply('Op(!=), 'Integer(I1:Int), 'Integer(I2:Int)) 
           => 'Bool(I1 =/=Int I2) ... </k>
  
  // Integer Less Than
  rule <k> 'Apply('Op(<), 'Integer(I1:Int), 'Integer(I2:Int)) 
           => 'Bool(I1 <Int I2) ... </k>
  
  // Integer Less Than or Equal
  rule <k> 'Apply('Op(<=), 'Integer(I1:Int), 'Integer(I2:Int)) 
           => 'Bool(I1 <=Int I2) ... </k>
  
  // Integer Greater Than
  rule <k> 'Apply('Op(>), 'Integer(I1:Int), 'Integer(I2:Int)) 
           => 'Bool(I1 >Int I2) ... </k>
  
  // Integer Greater Than or Equal
  rule <k> 'Apply('Op(>=), 'Integer(I1:Int), 'Integer(I2:Int)) 
           => 'Bool(I1 >=Int I2) ... </k>
  
  // Float Comparisons
  rule <k> 'Apply('Op(==), 'Float(F1:Float), 'Float(F2:Float)) 
           => 'Bool(F1 ==Float F2) ... </k>
  
  rule <k> 'Apply('Op(!=), 'Float(F1:Float), 'Float(F2:Float)) 
           => 'Bool(F1 =/=Float F2) ... </k>
  
  rule <k> 'Apply('Op(<), 'Float(F1:Float), 'Float(F2:Float)) 
           => 'Bool(F1 <Float F2) ... </k>
  
  rule <k> 'Apply('Op(<=), 'Float(F1:Float), 'Float(F2:Float)) 
           => 'Bool(F1 <=Float F2) ... </k>
  
  rule <k> 'Apply('Op(>), 'Float(F1:Float), 'Float(F2:Float)) 
           => 'Bool(F1 >Float F2) ... </k>
  
  rule <k> 'Apply('Op(>=), 'Float(F1:Float), 'Float(F2:Float)) 
           => 'Bool(F1 >=Float F2) ... </k>
  
  // String Equality
  rule <k> 'Apply('Op(==), 'String(S1:String), 'String(S2:String)) 
           => 'Bool(S1 ==String S2) ... </k>
  
  rule <k> 'Apply('Op(!=), 'String(S1:String), 'String(S2:String)) 
           => 'Bool(S1 =/=String S2) ... </k>
  
  // Boolean Equality
  rule <k> 'Apply('Op(==), 'Bool(B1:Bool), 'Bool(B2:Bool)) 
           => 'Bool(B1 ==Bool B2) ... </k>
  
  rule <k> 'Apply('Op(!=), 'Bool(B1:Bool), 'Bool(B2:Bool)) 
           => 'Bool(B1 =/=Bool B2) ... </k>
  
  // =========================================================================
  // Logical Operators
  // =========================================================================
  
  // Logical AND
  rule <k> 'Apply('Op(&&), 'Bool(B1:Bool), 'Bool(B2:Bool)) 
           => 'Bool(B1 andBool B2) ... </k>
  
  // Logical OR
  rule <k> 'Apply('Op(||), 'Bool(B1:Bool), 'Bool(B2:Bool)) 
           => 'Bool(B1 orBool B2) ... </k>
  
  // Logical NOT (unary)
  rule <k> 'Apply('Op(!), 'Bool(B:Bool)) 
           => 'Bool(notBool B) ... </k>
  
  // =========================================================================
  // Helper Functions
  // =========================================================================
  
  // Simplify a rational to lowest terms
  // If denominator is 1, return an integer
  syntax Expr ::= simplifyRational(Int, Int) [function]
  
  rule simplifyRational(N:Int, 1) => 'Integer(N)
  rule simplifyRational(N:Int, D:Int) => 'Rational(N /Int gcdInt(absInt(N), absInt(D)), 
                                                     D /Int gcdInt(absInt(N), absInt(D)))
    requires D =/=Int 1 andBool D =/=Int 0
  
  // Convert rational to float
  syntax Float ::= rationalToFloat(Int, Int) [function]
  
  rule rationalToFloat(N:Int, D:Int) => Int2Float(N) /Float Int2Float(D)
    requires D =/=Int 0
  
  // Helper to convert Int to Float
  syntax Float ::= Int2Float(Int) [function, hook(INT.int2float)]
  
  // Absolute value
  syntax Int ::= absInt(Int) [function]
  rule absInt(I:Int) => I requires I >=Int 0
  rule absInt(I:Int) => 0 -Int I requires I <Int 0
  
  // GCD for rational simplification
  syntax Int ::= gcdInt(Int, Int) [function]
  rule gcdInt(A:Int, 0) => A
  rule gcdInt(A:Int, B:Int) => gcdInt(B, A %Int B) requires B =/=Int 0

endmodule
