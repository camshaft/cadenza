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
    imports MAP

    // S-expression syntax matching the output from cadenza ast
    // All text is encoded as Unicode char sequences (u32 decimal values)
    // Format: (File 60 117 ... (Span 0 2 (Integer 42)))
    
    // Top-level program
    syntax Program ::= "(" "File" FilePath Exprs ")"  [symbol(File)]

    // Path wrapped in parens to distinguish from expressions
    syntax FilePath ::= "(" "Path" Chars ")"  [symbol(PathLit)]

    // Expressions
    syntax Expr ::= "(" "Span" "(" Int Int ")" Expr ")"             [symbol(Span)]
                  | "(" "Integer" Int ")"                   [symbol(IntegerLit)]
                  | "(" "Float" Int ")"                     [symbol(FloatLit)]
                  | "(" "String" Chars ")"  [symbol(StringLit)]
                  | "(" "Ident" Chars ")"   [symbol(Ident)]
                  | "(" "Apply" Expr Exprs ")"              [symbol(Apply)]
                  | "(" "Op" Chars ")"      [symbol(Op)]
                  | "(" "Synthetic" Chars ")"  [symbol(Synthetic)]
                  | "(" "Error" ")"                         [symbol(Error)]

    // Multiple expressions (for Apply arguments and File contents)
    syntax Exprs ::= List{Expr, ""}
    
    // List of integers for char sequences
    syntax Chars ::= List{Int, ""}

    // Values - the result of evaluation
    syntax Val ::= Int
                 | Float
                 | String
                 | Bool
                 | closure(String, List, Expr, Map)  // name, params, body, captured env

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
        <span-start> 0 </span-start>       // Current span start position
        <span-end> 0 </span-end> // Current span end position

    // ==========================================
    // File and Span Handling
    // ==========================================

    // Helper construct for sequential expression evaluation (not an Expr to avoid overlap)
    syntax KItem ::= seqExprs(Exprs)

    // Process File node - set file path and evaluate contents
    rule <k> (File (Path P:Chars) Es:Exprs) => seqExprs(Es) ... </k>
         <file> _ => charsToString(P) </file>

    // Span nodes - capture span info and unwrap for evaluation
    rule <k> (Span (Start:Int End:Int) E:Expr) => E ... </k>
         <span-start> _ => Start </span-start>
         <span-end> _ => End </span-end>

    // ==========================================
    // Expression List Evaluation
    // ==========================================

    // Empty expression list - evaluates to unit (represented as 0)
    rule <k> seqExprs(.Exprs) => 0 ... </k>

    // Single expression - just unwrap and evaluate it
    rule <k> seqExprs(E:Expr .Exprs) => E ... </k>

    // Two or more expressions - evaluate first and save rest as continuation
    rule <k> seqExprs(E1:Expr E2:Expr Es:Exprs) => E1 ~> seqExprs(E2 Es) ... </k>

    // After first expression evaluates to a value, discard it and continue with rest
    rule <k> _:Val ~> seqExprs(Es:Exprs) => seqExprs(Es) ... </k>

    // ==========================================
    // Literal Evaluation Rules
    // ==========================================

    // Integer literals - extract the value
    rule <k> (Integer I:Int) => I ... </k>

    // Float literals - convert from bit representation back to float
    // The AST encodes floats as their 64-bit integer bit representation
    rule <k> (Float Bits:Int) => Int2Float(Bits, 52, 11) ... </k>

    // String literals - decode char list to string
    rule <k> (String Chrs:Chars) => charsToString(Chrs) ... </k>

    // Helper function to convert char list to string
    syntax String ::= charsToString(Chars) [function]
    rule charsToString(.Chars) => ""
    rule charsToString(I:Int Rest:Chars) => chrChar(I) +String charsToString(Rest)

    // Identifier lookup - decode char list and check for booleans
    rule <k> (Ident Chrs:Chars) => true ... </k>
         requires charsToString(Chrs) ==String "true"
    rule <k> (Ident Chrs:Chars) => false ... </k>
         requires charsToString(Chrs) ==String "false"

    // ==========================================
    // Variable Binding (let)
    // ==========================================

    // Handle let binding: (Apply (Op =) (Apply (Ident "let") (Ident name)) expr)
    // The pattern is: let <pattern> = <expr>
    // For now, we only support simple identifier patterns

    // Helper chars for "let" keyword
    syntax Chars ::= "letChars" [function]
    rule letChars => 108 101 116 .Chars  // "let" as char codes

    // Helper chars for "=" operator
    syntax Chars ::= "eqChars" [function]
    rule eqChars => 61 .Chars  // "=" as char codes

    // Rule: (Apply (Op "=") (Apply (Ident "let") (Ident name)) value)
    // When we see an assignment where LHS is a let-application, bind the variable
    rule <k> (Apply (Op EqOp:Chars) (Apply (Ident LetKw:Chars) (Ident NameChrs:Chars)) E:Expr)
          => E ~> bindVar(charsToString(NameChrs)) ... </k>
         requires charsToString(LetKw) ==String "let"
          andBool charsToString(EqOp) ==String "="

    // After expression evaluates to a value, bind it in the environment
    syntax KItem ::= bindVar(String)
    rule <k> V:Val ~> bindVar(Name:String) => V ... </k>
         <env> Env:Map => Env[Name <- V] </env>

    // ==========================================
    // Variable Lookup
    // ==========================================

    // Look up variable in environment (non-function values)
    rule <k> (Ident Chrs:Chars) => V ... </k>
         <env> ... charsToString(Chrs) |-> V:Val ... </env>
         requires charsToString(Chrs) =/=String "true"
          andBool charsToString(Chrs) =/=String "false"
          andBool notBool isClosure(V)

    // Look up zero-arg function and auto-apply it
    rule <k> (Ident Chrs:Chars) => applyFn(closure(Name, .List, Body, CapturedEnv), .List) ... </k>
         <env> ... charsToString(Chrs) |-> closure(Name, .List, Body, CapturedEnv) ... </env>
         requires charsToString(Chrs) =/=String "true"
          andBool charsToString(Chrs) =/=String "false"

    // Look up function with params - return the closure as-is (will be applied later)
    rule <k> (Ident Chrs:Chars) => closure(Name, Params, Body, CapturedEnv) ... </k>
         <env> ... charsToString(Chrs) |-> closure(Name, Params, Body, CapturedEnv) ... </env>
         requires charsToString(Chrs) =/=String "true"
          andBool charsToString(Chrs) =/=String "false"
          andBool size(Params) >Int 0

    // Helper to check if value is a closure
    syntax Bool ::= isClosure(Val) [function]
    rule isClosure(closure(_, _, _, _)) => true
    rule isClosure(_) => false [owise]

    // ==========================================
    // Function Definition (fn)
    // ==========================================

    // Function definitions come as: (Apply (Op "=") LHS Body)
    // where LHS is a nested Apply chain starting with (Ident "fn")
    // We need to extract: function name, parameter list

    // Check if this is a function definition by looking for "fn" at the start
    syntax Bool ::= isFnDef(Expr) [function]
    rule isFnDef((Apply (Ident Chrs:Chars) _)) => charsToString(Chrs) ==String "fn"
    rule isFnDef((Apply E:Expr _)) => isFnDef(E)
    rule isFnDef(_) => false [owise]

    // Extract function name - the first Ident after "fn"
    syntax String ::= getFnName(Expr) [function]
    rule getFnName((Apply (Ident FnKw:Chars) (Ident NameChrs:Chars))) => charsToString(NameChrs)
         requires charsToString(FnKw) ==String "fn"
    rule getFnName((Apply E:Expr _)) => getFnName(E) [owise]

    // Extract parameters as a List of Strings (in reverse order from parsing)
    syntax List ::= getFnParams(Expr) [function]
    // Base case: (Apply (Ident "fn") (Ident name)) - no params
    rule getFnParams((Apply (Ident FnKw:Chars) (Ident _))) => .List
         requires charsToString(FnKw) ==String "fn"
    // Recursive case: (Apply inner (Ident param)) - add param to list
    rule getFnParams((Apply E:Expr (Ident ParamChrs:Chars))) => getFnParams(E) ListItem(charsToString(ParamChrs))
         requires notBool isFnKeyword(E)
    // Handle the case where inner is still fn-related
    rule getFnParams((Apply (Apply (Ident FnKw:Chars) (Ident _)) (Ident ParamChrs:Chars))) 
         => ListItem(charsToString(ParamChrs))
         requires charsToString(FnKw) ==String "fn"
    rule getFnParams(_) => .List [owise]

    // Helper to check if an Expr is the fn keyword
    syntax Bool ::= isFnKeyword(Expr) [function]
    rule isFnKeyword((Ident Chrs:Chars)) => charsToString(Chrs) ==String "fn"
    rule isFnKeyword(_) => false [owise]

    // Main rule: function definition
    // Pattern: (Apply (Op "=") LHS Body) where isFnDef(LHS)
    rule <k> (Apply (Op EqOp:Chars) LHS:Expr Body:Expr)
          => closure(getFnName(LHS), getFnParams(LHS), Body, Env) 
             ~> bindVar(getFnName(LHS)) ... </k>
         <env> Env:Map </env>
         requires charsToString(EqOp) ==String "="
          andBool isFnDef(LHS)

    // ==========================================
    // Function Application
    // ==========================================

    // Helper constructs for function application
    syntax KItem ::= applyFn(Val, List)           // closure, evaluated args
                   | evalArgs(Exprs, List, Val)   // remaining args, accumulated args, closure
                   | evalArg(Val, List, Val)      // after arg eval: arg value, accumulated args, closure

    // Apply function looked up from identifier with single argument
    rule <k> (Apply (Ident Chrs:Chars) Arg:Expr .Exprs) 
          => Arg ~> evalArg(.List, closure(Name, Params, Body, CapturedEnv)) ... </k>
         <env> ... charsToString(Chrs) |-> closure(Name, Params, Body, CapturedEnv) ... </env>
         requires charsToString(Chrs) =/=String "true"
          andBool charsToString(Chrs) =/=String "false"
          andBool size(Params) >Int 0

    // After single argument evaluates
    syntax KItem ::= evalArg(List, Val)  // accumulated args, closure
    rule <k> V:Val ~> evalArg(Args:List, Clos:Val) 
          => applyFn(Clos, Args ListItem(V)) ... </k>

    // Apply function with multiple arguments - evaluate first, then rest
    rule <k> (Apply (Ident Chrs:Chars) Arg1:Expr Arg2:Expr RestArgs:Exprs) 
          => Arg1 ~> evalArgs(Arg2 RestArgs, .List, closure(Name, Params, Body, CapturedEnv)) ... </k>
         <env> ... charsToString(Chrs) |-> closure(Name, Params, Body, CapturedEnv) ... </env>
         requires charsToString(Chrs) =/=String "true"
          andBool charsToString(Chrs) =/=String "false"

    // After arg evaluates, continue with remaining args or apply
    rule <k> V:Val ~> evalArgs(NextArg:Expr RestArgs:Exprs, AccArgs:List, Clos:Val)
          => NextArg ~> evalArgs(RestArgs, AccArgs ListItem(V), Clos) ... </k>

    // All args evaluated - apply the function
    rule <k> V:Val ~> evalArgs(.Exprs, AccArgs:List, Clos:Val)
          => applyFn(Clos, AccArgs ListItem(V)) ... </k>

    // Apply closure: bind all params to args and evaluate body
    // Save current env, switch to closure env with params bound, will restore after
    rule <k> applyFn(closure(_, Params:List, Body:Expr, CapturedEnv:Map), Args:List)
          => Body ~> restoreEnv(OrigEnv) ... </k>
         <env> OrigEnv:Map => bindParams(CapturedEnv, Params, Args) </env>
         requires size(Params) ==Int size(Args)

    // Helper to restore environment after function call
    syntax KItem ::= restoreEnv(Map)
    rule <k> V:Val ~> restoreEnv(SavedEnv:Map) => V ... </k>
         <env> _ => SavedEnv </env>

    // Partial application: more params than args - create new closure with bound args
    rule <k> applyFn(closure(Name, Params:List, Body:Expr, CapturedEnv:Map), Args:List)
          => closure(Name, dropParams(Params, size(Args)), Body, bindParams(CapturedEnv, takeParams(Params, size(Args)), Args)) ... </k>
         requires size(Params) >Int size(Args)

    // Helper functions for partial application
    syntax List ::= takeParams(List, Int) [function]
    rule takeParams(_, 0) => .List
    rule takeParams(ListItem(P) Rest:List, N:Int) => ListItem(P) takeParams(Rest, N -Int 1)
         requires N >Int 0
    rule takeParams(.List, _) => .List

    syntax List ::= dropParams(List, Int) [function]
    rule dropParams(L, 0) => L
    rule dropParams(ListItem(_) Rest:List, N:Int) => dropParams(Rest, N -Int 1)
         requires N >Int 0
    rule dropParams(.List, _) => .List

    // Handle curried application: (Apply (Apply f x) y) pattern
    // First, evaluate the inner Apply to get a closure
    rule <k> (Apply (Apply E1:Expr E2:Expr .Exprs) E3:Expr RestArgs:Exprs)
          => (Apply E1 E2 .Exprs) ~> evalCurriedArgs(E3 RestArgs) ... </k>
         requires notBool isOp(E1)

    // After inner call evaluates to a closure, apply with remaining args
    syntax KItem ::= evalCurriedArgs(Exprs)
    rule <k> closure(Name, Params, Body, CapturedEnv) ~> evalCurriedArgs(Arg:Expr RestArgs:Exprs)
          => Arg ~> evalArgs(RestArgs, .List, closure(Name, Params, Body, CapturedEnv)) ... </k>

    // Helper to check if an expression is an Op node
    syntax Bool ::= isOp(Expr) [function]
    rule isOp((Op _)) => true
    rule isOp(_) => false [owise]

    // Helper function to bind parameters to arguments
    syntax Map ::= bindParams(Map, List, List) [function]
    rule bindParams(Env, .List, .List) => Env
    rule bindParams(Env, ListItem(P:String) RestParams:List, ListItem(A:Val) RestArgs:List)
         => bindParams(Env[P <- A], RestParams, RestArgs)

    // ==========================================
    // Arithmetic Operations
    // ==========================================

    // Addition: a + b
    rule <k> (Apply (Op PlusOp:Chars) E1:Expr E2:Expr)
          => E1 ~> evalBinOp("+", E2) ... </k>
         requires charsToString(PlusOp) ==String "+"

    // Subtraction: a - b
    rule <k> (Apply (Op MinusOp:Chars) E1:Expr E2:Expr)
          => E1 ~> evalBinOp("-", E2) ... </k>
         requires charsToString(MinusOp) ==String "-"

    // Multiplication: a * b
    rule <k> (Apply (Op StarOp:Chars) E1:Expr E2:Expr)
          => E1 ~> evalBinOp("*", E2) ... </k>
         requires charsToString(StarOp) ==String "*"

    // Division: a / b
    rule <k> (Apply (Op SlashOp:Chars) E1:Expr E2:Expr)
          => E1 ~> evalBinOp("/", E2) ... </k>
         requires charsToString(SlashOp) ==String "/"

    // Helper construct for binary operations
    syntax KItem ::= evalBinOp(String, Expr)      // operator, second operand (unevaluated)
                   | evalBinOp2(String, Val)      // operator, first operand value

    // After first operand evaluates, evaluate second
    rule <k> V1:Val ~> evalBinOp(Operator:String, E2:Expr)
          => E2 ~> evalBinOp2(Operator, V1) ... </k>

    // After both operands evaluate, perform the operation
    rule <k> V2:Int ~> evalBinOp2("+", V1:Int) => V1 +Int V2 ... </k>
    rule <k> V2:Int ~> evalBinOp2("-", V1:Int) => V1 -Int V2 ... </k>
    rule <k> V2:Int ~> evalBinOp2("*", V1:Int) => V1 *Int V2 ... </k>
    rule <k> V2:Int ~> evalBinOp2("/", V1:Int) => V1 /Int V2 ... </k>
         requires V2 =/=Int 0

    // Float operations
    rule <k> V2:Float ~> evalBinOp2("+", V1:Float) => V1 +Float V2 ... </k>
    rule <k> V2:Float ~> evalBinOp2("-", V1:Float) => V1 -Float V2 ... </k>
    rule <k> V2:Float ~> evalBinOp2("*", V1:Float) => V1 *Float V2 ... </k>
    rule <k> V2:Float ~> evalBinOp2("/", V1:Float) => V1 /Float V2 ... </k>
         requires V2 =/=Float 0.0

    // ==========================================
    // Error handling
    // ==========================================

    // Error nodes represent parse/semantic errors
    // For now, they remain as-is for error modeling
    rule <k> (Error) => (Error) ... </k>

endmodule
