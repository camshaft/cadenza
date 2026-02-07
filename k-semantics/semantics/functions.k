// Cadenza Functions Semantics
//
// This module implements:
// - Function definition with closure capture
// - Function application
// - Recursion support (hoisting)
// - Higher-order functions

requires "../syntax.k"

module CADENZA-FUNCTIONS
  imports CADENZA-SYNTAX
  imports CADENZA-LITERALS
  imports CADENZA-VARIABLES
  imports MAP
  imports LIST

  // =========================================================================
  // Function Definition
  // =========================================================================
  
  // Function definition: fn name param1 param2 ... = body
  // Creates a closure capturing the current environment
  // Binds the function to its name (for recursion)
  // Returns Unit
  
  rule <k> 'Fn(Name:Id, Params:Ids, Body:Expr) 
           => 'Unit() ... </k>
       <scope> Env:Map => Env[Name <- 'Closure(Name, Params, Body, Env)] </scope>
  
  // =========================================================================
  // Function Application
  // =========================================================================
  
  // First, evaluate the function expression
  rule <k> 'Apply(F:Expr, Args:Exprs) => F ~> 'ApplyArgs(Args) ... </k>
    requires notBool isValue(F)
  
  // Then evaluate all arguments (left to right)
  rule <k> 'Closure(Name:Id, Params:Ids, Body:Expr, CapturedEnv:Map) 
           ~> 'ApplyArgs(Args:Exprs) 
           => evaluateArgs(Args) ~> 'DoApply(Name, Params, Body, CapturedEnv) ... </k>
  
  // After all arguments are evaluated, perform the application
  rule <k> evaledArgs(Values:Exprs) ~> 'DoApply(Name:Id, Params:Ids, Body:Expr, CapturedEnv:Map)
           => pushScope ~> bindParams(Params, Values) ~> Body ~> popScope ... </k>
       <scope> CurrentScope:Map => CapturedEnv[Name <- 'Closure(Name, Params, Body, CapturedEnv)] </scope>
    requires lengthIds(Params) ==Int lengthExprs(Values)
  
  // Arity mismatch error
  rule <k> evaledArgs(Values:Exprs) ~> 'DoApply(Name:Id, Params:Ids, _, _)
           => 'Error("arity mismatch: function " +String Id2String(Name) 
                      +String " expects " +String Int2String(lengthIds(Params)) 
                      +String " arguments, got " +String Int2String(lengthExprs(Values))) ... </k>
    requires lengthIds(Params) =/=Int lengthExprs(Values)
  
  // =========================================================================
  // Zero-Parameter Functions
  // =========================================================================
  
  // Functions with no parameters are automatically applied when referenced
  rule <k> 'Closure(Name:Id, .Ids, Body:Expr, CapturedEnv:Map) 
           => pushScope ~> Body ~> popScope ... </k>
       <scope> _:Map => CapturedEnv[Name <- 'Closure(Name, .Ids, Body, CapturedEnv)] </scope>
  
  // =========================================================================
  // Helper Functions and Syntax
  // =========================================================================
  
  // Internal constructs for function application
  syntax KItem ::= "'ApplyArgs'" "(" Exprs ")"
                 | "'DoApply'" "(" Id "," Ids "," Expr "," Map ")"
                 | "evaledArgs" "(" Exprs ")"
  
  // Evaluate a list of expressions to values
  syntax KItem ::= evaluateArgs(Exprs) [function]
  
  // TODO: Implement proper sequential evaluation of arguments
  // For now, assume arguments are already values or will be evaluated elsewhere
  
  rule evaluateArgs(Args:Exprs) => evaledArgs(Args)
    requires allValuesExprs(Args)
  
  // Bind parameters to argument values in current scope
  syntax KItem ::= bindParams(Ids, Exprs)
  
  rule <k> bindParams(.Ids, .Exprs) => . ... </k>
  
  rule <k> bindParams((P:Id, Ps:Ids), (V:Value, Vs:Exprs)) 
           => bindParams(Ps, Vs) ... </k>
       <scope> Env:Map => Env[P <- V] </scope>
    requires notBool (P in_keys(Env))
  
  // =========================================================================
  // Helper Predicates
  // =========================================================================
  
  // Check if all expressions in a list are values
  syntax Bool ::= allValuesExprs(Exprs) [function]
  
  rule allValuesExprs(.Exprs) => true
  rule allValuesExprs((E:Expr, Es:Exprs)) => isValue(E) andBool allValuesExprs(Es)
  
  // Get length of Ids list
  syntax Int ::= lengthIds(Ids) [function]
  
  rule lengthIds(.Ids) => 0
  rule lengthIds((_:Id, Ids:Ids)) => 1 +Int lengthIds(Ids)
  
  // Get length of Exprs list
  syntax Int ::= lengthExprs(Exprs) [function]
  
  rule lengthExprs(.Exprs) => 0
  rule lengthExprs((_:Expr, Es:Exprs)) => 1 +Int lengthExprs(Es)

endmodule
