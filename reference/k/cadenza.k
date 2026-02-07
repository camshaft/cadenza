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
    imports LIST

    // Expression syntax matching the S-expression output from cadenza ast
    // All strings are encoded as Unicode char sequences (u32 values in hex)
    // Integers and floats are tagged: (Integer 123) (Float 3.14)
    // Strings: (String 68 65 6c 6c 6f) - char codes for "hello"
    // Identifiers: (Ident 66 6f 6f) - char codes for "foo"
    // Operators: (Op 2b) - char code for "+"
    syntax Expr ::= Integer(Int)          [klabel(IntegerLit)]
                  | Float(Float)          [klabel(FloatLit)]
                  | String(Ints)          [klabel(StringLit)]
                  | Bool(String)          [klabel(BoolLit)]
                  | Ident(Ints)           [klabel(Ident)]
                  | Apply(Expr, Exprs)    [klabel(Apply)]
                  | Op(Ints)              [klabel(Op)]
                  | Synthetic(Ints)       [klabel(Synthetic)]
                  | Error()               [klabel(Error)]

    // Multiple expressions (for Apply arguments)
    syntax Exprs ::= List{Expr, ""}       [klabel(Exprs)]
    
    // List of integers for char sequences
    syntax Ints ::= List{Int, ""}         [klabel(Ints)]

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

    // Integer literals - extract the value
    rule <k> Integer(I:Int) => I ... </k>
        [macro]

    // Float literals - extract the value
    rule <k> Float(F:Float) => F ... </k>
        [macro]

    // String literals - decode char list to string
    rule <k> String(Chars:Ints) => charsToString(Chars) ... </k>
        [macro]

    // Helper function to convert char list to string
    syntax String ::= charsToString(Ints) [function]
    rule charsToString(.Ints) => ""
    rule charsToString(I:Int, Rest:Ints) => chrChar(I) +String charsToString(Rest)

    // Identifier - decode char list to string for lookup
    rule <k> Ident(Chars:Ints) => Ident(charsToString(Chars)) ... </k>
        [macro]

    // Boolean literals (handled as identifiers in the parser)
    rule <k> Ident("true") => true ... </k>
        [macro]
    rule <k> Ident("false") => false ... </k>
        [macro]

    // Boolean literals (if they come as Bool nodes)
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
