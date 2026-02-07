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

    // S-expression syntax matching the output from cadenza ast
    // All text is encoded as Unicode char sequences (u32 decimal values)
    // Format: (File 60 117 ... (Span 0 2 (Integer 42)))
    
    // Top-level program
    syntax Program ::= "(" "File" Ints Exprs ")"  [klabel(File)]
    
    // Expressions
    syntax Expr ::= "(" "Span" Int Int Expr ")"             [klabel(Span)]
                  | "(" "Integer" Int ")"                    [klabel(IntegerLit)]
                  | "(" "Float" Float ")"                    [klabel(FloatLit)]
                  | "(" "String" Ints ")"                    [klabel(StringLit)]
                  | "(" "Bool" String ")"                    [klabel(BoolLit)]
                  | "(" "Ident" Ints ")"                     [klabel(Ident)]
                  | "(" "Apply" Expr Exprs ")"               [klabel(Apply)]
                  | "(" "Op" Ints ")"                        [klabel(Op)]
                  | "(" "Synthetic" Ints ")"                 [klabel(Synthetic)]
                  | "(" "Error" ")"                          [klabel(Error)]

    // Multiple expressions (for Apply arguments and File contents)
    syntax Exprs ::= List{Expr, ""}          [klabel(Exprs)]
    
    // List of integers for char sequences
    syntax Ints ::= List{Int, ""}            [klabel(Ints)]

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
        <k> $PGM:Program </k>
        <env> .Map </env>      // Variable bindings: String -> Val
        <store> .Map </store>  // Memory store (for future use)
        <output> .List </output>  // Output for display
        <file> "" </file>      // Current file path

    // ==========================================
    // File and Span Handling
    // ==========================================

    // Process File node - set file path and evaluate contents
    rule <k> (File Path:Ints Exprs:Exprs) => Exprs ... </k>
         <file> _ => charsToString(Path) </file>

    // Span nodes - unwrap for evaluation (span info available if needed)
    rule <k> (Span _:Int _:Int E:Expr) => E ... </k>

    // ==========================================
    // Literal Evaluation Rules
    // ==========================================

    // Integer literals - extract the value
    rule <k> (Integer I:Int) => I ... </k>

    // Float literals - extract the value
    rule <k> (Float F:Float) => F ... </k>

    // String literals - decode char list to string
    rule <k> (String Chars:Ints) => charsToString(Chars) ... </k>

    // Helper function to convert char list to string
    syntax String ::= charsToString(Ints) [function]
    rule charsToString(.Ints) => ""
    rule charsToString(I:Int, Rest:Ints) => chrChar(I) +String charsToString(Rest)

    // Identifier lookup - decode char list and check for booleans
    rule <k> (Ident Chars:Ints) => true ... </k>
         requires charsToString(Chars) ==String "true"
    rule <k> (Ident Chars:Ints) => false ... </k>
         requires charsToString(Chars) ==String "false"

    // Boolean literals (if they come as Bool nodes)
    rule <k> (Bool "true") => true ... </k>
    rule <k> (Bool "false") => false ... </k>

    // ==========================================
    // Error handling
    // ==========================================

    // Error nodes represent parse/semantic errors
    // For now, they remain as-is for error modeling
    rule <k> (Error) => (Error) ... </k>

endmodule
