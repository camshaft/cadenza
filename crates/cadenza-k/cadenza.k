// Cadenza Language Definition in K Framework
// ===========================================
//
// This is a reference implementation of the Cadenza language semantics.
// It accepts parenthesized AST produced by `cadenza ast` command.

module CADENZA-SYNTAX
    imports INT
    imports FLOAT
    imports STRING
    imports BOOL

    // Expression syntax matching the S-expression output from cadenza ast
    syntax Expr ::= Int(String)           [klabel(IntLit)]
                  | Float(String)         [klabel(FloatLit)]
                  | String(String)        [klabel(StringLit)]
                  | Bool(String)          [klabel(BoolLit)]
                  | Ident(String)         [klabel(Ident)]
                  | Apply(Expr, Exprs)    [klabel(Apply)]
                  | Op(String)            [klabel(Op)]
                  | Synthetic(String)     [klabel(Synthetic)]
                  | Error()               [klabel(Error)]

    // Multiple expressions (for Apply arguments)
    syntax Exprs ::= List{Expr, ""}       [klabel(Exprs)]

    // Values - the result of evaluation
    syntax Val ::= Int
                 | Float
                 | String
                 | Bool

    syntax Expr ::= Val
    syntax KResult ::= Val
endmodule

module CADENZA
    imports CADENZA-SYNTAX
    imports MAP
    imports LIST

    // Configuration defines the runtime state
    configuration
        <k> $PGM:Expr </k>
        <env> .Map </env>      // Variable bindings: String -> Val
        <store> .Map </store>  // Memory store (for future use)
        <output> .List </output>  // Output for display

    // ==========================================
    // Literal Evaluation Rules
    // ==========================================

    // Integer literals
    rule <k> Int(S:String) => String2Int(S) ... </k>
        [macro]

    // Float literals
    rule <k> Float(S:String) => String2Float(S) ... </k>
        [macro]

    // String literals - already evaluated
    rule <k> String(S:String) => S ... </k>
        [macro]

    // Boolean literals
    rule <k> Bool("true") => true ... </k>
        [macro]
    rule <k> Bool("false") => false ... </k>
        [macro]

    // ==========================================
    // Error handling
    // ==========================================

    // Error nodes remain as-is (for now)
    rule <k> Error() => Error() ... </k>

endmodule
