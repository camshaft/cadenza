// Cadenza Variables and Bindings Semantics
//
// This module implements:
// - Identifier lookup in the environment
// - Let bindings (immutable variable creation)
// - Reassignment (mutable variable updates)
// - Lexical scoping with environment as a stack of scopes

requires "../syntax.k"

module CADENZA-VARIABLES
  imports CADENZA-SYNTAX
  imports CADENZA-LITERALS
  imports MAP
  imports LIST

  // =========================================================================
  // Identifier Lookup
  // =========================================================================
  
  // Look up an identifier in the environment
  // Search from current scope backwards through parent scopes
  rule <k> 'Ident(X:Id) => V ... </k>
       <scope> Env:Map </scope>
    requires X in_keys(Env)
       andBool isValue({Env[X]}:>Expr)
    ensures V ==K {Env[X]}:>Value
  
  // If not in current scope, search parent scopes
  rule <k> 'Ident(X:Id) => 'Ident(X) ... </k>
       <scope> Env:Map </scope>
       <env> ListItem(ParentScope:Map) Rest:List </env>
    requires notBool (X in_keys(Env))
  
  // Undefined variable error
  rule <k> 'Ident(X:Id) => 'Error("undefined variable: " +String Id2String(X)) ... </k>
       <scope> Env:Map </scope>
       <env> .List </env>
    requires notBool (X in_keys(Env))
  
  // =========================================================================
  // Let Bindings
  // =========================================================================
  
  // Let binding with identifier pattern: evaluate RHS, bind to name, return Unit
  rule <k> 'Let(X:Id, E:Expr) => E ~> 'BindLet(X) ... </k>
  
  // After evaluating the expression, bind it to the identifier
  rule <k> V:Value ~> 'BindLet(X:Id) => 'Unit() ... </k>
       <scope> Env:Map => Env[X <- V] </scope>
  
  // Let binding with wildcard pattern: evaluate RHS, discard result, return Unit
  rule <k> 'Let('Wildcard(), E:Expr) => E ~> 'DiscardLet ... </k>
  
  rule <k> V:Value ~> 'DiscardLet => 'Unit() ... </k>
  
  // Let binding with tuple pattern: evaluate RHS, destructure tuple
  rule <k> 'Let('TuplePattern(Patterns:Patterns), E:Expr) 
           => E ~> 'BindTupleLet(Patterns) ... </k>
  
  // Destructure tuple and bind each element
  // TODO: Implement full tuple destructuring
  // For now, this is a placeholder
  rule <k> 'Tuple(Values:Exprs) ~> 'BindTupleLet(Patterns:Patterns) 
           => 'BindTupleElements(Values, Patterns) ... </k>
  
  // =========================================================================
  // Reassignment
  // =========================================================================
  
  // Reassignment: evaluate RHS, then update the binding
  rule <k> 'Assign(X:Id, E:Expr) => E ~> 'DoAssign(X) ... </k>
  
  // After evaluating, update the binding in current scope
  rule <k> V:Value ~> 'DoAssign(X:Id) => 'Unit() ... </k>
       <scope> Env:Map => Env[X <- V] </scope>
    requires X in_keys(Env)
  
  // If not in current scope, search parent scopes
  // TODO: Implement proper scope search for reassignment
  
  // Error if variable doesn't exist
  rule <k> V:Value ~> 'DoAssign(X:Id) 
           => 'Error("cannot reassign undefined variable: " +String Id2String(X)) ... </k>
       <scope> Env:Map </scope>
    requires notBool (X in_keys(Env))
  
  // =========================================================================
  // Scoping
  // =========================================================================
  
  // Push a new scope onto the environment stack
  syntax KItem ::= "pushScope"
  
  rule <k> pushScope => . ... </k>
       <scope> CurrentScope:Map => .Map </scope>
       <env> Env:List => ListItem(CurrentScope) Env </env>
  
  // Pop a scope from the environment stack
  syntax KItem ::= "popScope"
  
  rule <k> popScope => . ... </k>
       <scope> _:Map => ParentScope </scope>
       <env> ListItem(ParentScope:Map) Rest:List => Rest </env>
  
  // =========================================================================
  // Helper Syntax
  // =========================================================================
  
  // Internal constructs for binding after evaluation
  syntax KItem ::= "'BindLet'" "(" Id ")"
                 | "'DiscardLet'"
                 | "'BindTupleLet'" "(" Patterns ")"
                 | "'BindTupleElements'" "(" Exprs "," Patterns ")"
                 | "'DoAssign'" "(" Id ")"

endmodule
