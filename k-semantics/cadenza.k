// Cadenza Language Definition in K Framework
//
// This is the main K definition file that combines all modules to create
// an executable formal semantics for the Cadenza programming language.
//
// Usage:
//   kompile cadenza.k
//   krun program.cdz

requires "configuration.k"
requires "semantics/literals.k"
requires "semantics/variables.k"
requires "semantics/operators.k"
requires "semantics/functions.k"

module CADENZA
  imports CADENZA-CONFIGURATION
  imports CADENZA-LITERALS
  imports CADENZA-VARIABLES
  imports CADENZA-OPERATORS
  imports CADENZA-FUNCTIONS

  // =========================================================================
  // Program Execution
  // =========================================================================
  
  // A program is a sequence of statements
  // Execute each statement in order, keeping only the last result
  
  // Empty program
  rule <k> .Stmts => 'Unit() ... </k>
  
  // Single statement
  rule <k> S:Stmt => S ... </k>
  
  // Multiple statements: execute first, discard result, continue with rest
  rule <k> (S:Stmt ; Ss:Stmts) => S ~> discardResult ~> Ss ... </k>
  
  // Discard the result of a statement when there are more statements
  syntax KItem ::= "discardResult"
  rule <k> _:Value ~> discardResult => . ... </k>
  
  // =========================================================================
  // Statement Execution
  // =========================================================================
  
  // Let statement: already handled in variables.k
  // Fn statement: already handled in functions.k
  // Assign statement: already handled in variables.k
  // Expression statement: just evaluate the expression
  
  rule <k> E:Expr => E ... </k>
    requires notBool isStmt(E)
  
  // =========================================================================
  // Blocks
  // =========================================================================
  
  // A block creates a new scope, executes statements, returns last value
  rule <k> 'Block(Stmts:Stmts) 
           => pushScope ~> Stmts ~> popScope ... </k>
  
  // =========================================================================
  // Match Expressions (Control Flow)
  // =========================================================================
  
  // Match expression: evaluate the scrutinee, then try each arm
  rule <k> 'Match(E:Expr, Arms:MatchArms) 
           => E ~> 'MatchArms(Arms) ... </k>
    requires notBool isValue(E)
  
  // After evaluating scrutinee, try matching against arms
  rule <k> V:Value ~> 'MatchArms(Arms:MatchArms) 
           => tryMatchArms(V, Arms) ... </k>
  
  syntax KItem ::= "'MatchArms'" "(" MatchArms ")"
  
  // Try to match value against each arm in order
  syntax KItem ::= tryMatchArms(Value, MatchArms) [function]
  
  // TODO: Implement pattern matching
  // For now, this is a placeholder
  rule tryMatchArms(V:Value, ('MatchArm(P:Pattern, Body:Expr), Arms:MatchArms)) 
       => Body
    requires matchesPattern(V, P)
  
  rule tryMatchArms(V:Value, ('MatchArm(P:Pattern, _), Arms:MatchArms)) 
       => tryMatchArms(V, Arms)
    requires notBool matchesPattern(V, P)
  
  rule tryMatchArms(_:Value, .MatchArms) 
       => 'Error("no match found")
  
  // Check if a value matches a pattern
  syntax Bool ::= matchesPattern(Value, Pattern) [function]
  
  // Identifier pattern always matches
  rule matchesPattern(_:Value, _:Id) => true
  
  // Wildcard pattern always matches
  rule matchesPattern(_:Value, 'Wildcard()) => true
  
  // TODO: Implement more pattern matching (tuples, literals, etc.)
  rule matchesPattern(_:Value, _:Pattern) => false [owise]
  
  // =========================================================================
  // Error Handling
  // =========================================================================
  
  // Error syntax for runtime errors
  syntax Expr ::= "'Error'" "(" String ")" [klabel('Error)]
  
  // When an error occurs, put it in the error cell and halt
  rule <k> 'Error(Msg:String) => . ... </k>
       <error> _ => Msg </error>
  
  // =========================================================================
  // Helper Predicates
  // =========================================================================
  
  syntax Bool ::= isStmt(K) [function]
  
  rule isStmt('Let(_, _)) => true
  rule isStmt('Fn(_, _, _)) => true
  rule isStmt('Assign(_, _)) => true
  rule isStmt(_) => false [owise]

endmodule
