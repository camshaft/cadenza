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
                 | tuple(List)                        // tuple value (list of vals)
                 | record(Map)                        // record value (map of String -> Val)
                 | list(List)                         // list value (list of vals)
                 | structVal(String, Map)             // struct value (name, fields)
                 | enumVal(String, String, Val)       // enum value (enum name, variant name, data)

    // Type descriptors (for struct/enum definitions)
    syntax TypeDef ::= structDef(String, Map)         // struct definition (name, field types)
                     | enumDef(String, Map)           // enum definition (name, variant -> type)

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

    // Process File node - set file path, hoist definitions, then evaluate non-definitions
    // Hoisting: collect all function/struct/enum definitions first
    rule <k> (File (Path P:Chars) Es:Exprs) => hoistDefs(Es) ~> seqExprs(filterDefs(Es)) ... </k>
         <file> _ => charsToString(P) </file>

    // Filter out definitions from expression list (they were already hoisted)
    syntax Exprs ::= filterDefs(Exprs) [function]
    rule filterDefs(.Exprs) => .Exprs
    rule filterDefs(E:Expr RestExprs:Exprs) => filterDefs(RestExprs)
         requires isDefinition(E)
    rule filterDefs(E:Expr RestExprs:Exprs) => E filterDefs(RestExprs)
         requires notBool isDefinition(E)

    // Hoisting: scan expressions for definitions and bind them
    syntax KItem ::= hoistDefs(Exprs)

    // Empty - done hoisting
    rule <k> hoistDefs(.Exprs) => .K ... </k>

    // Hoist function definition
    rule <k> hoistDefs((Span (_ _) E:Expr) RestExprs:Exprs)
          => hoistDefs(E RestExprs) ... </k>

    rule <k> hoistDefs((Apply (Op EqOp:Chars) LHS:Expr Body:Expr) RestExprs:Exprs)
          => hoistOneDef((Apply (Op EqOp) LHS Body)) ~> hoistDefs(RestExprs) ... </k>
         requires isKw(EqOp, "=") andBool (isFnDef(LHS) orBool isStructDef(LHS) orBool isEnumDef(LHS))

    // Skip non-definition expressions during hoisting
    rule <k> hoistDefs(E:Expr RestExprs:Exprs) => hoistDefs(RestExprs) ... </k>
         requires notBool isDefinition(E)
      [owise]

    // Helper to check if expression is a hoistable definition (fn/struct/enum, NOT let)
    syntax Bool ::= isDefinition(Expr) [function]
    rule isDefinition((Span (_ _) E)) => isDefinition(E)
    rule isDefinition((Apply (Op EqOp:Chars) LHS _)) => isKw(EqOp, "=") andBool 
                                                        (isFnDef(LHS) orBool isStructDef(LHS) orBool isEnumDef(LHS))
                                                        andBool notBool isLetDef(LHS)
    rule isDefinition((Apply (Apply (Ident KwChrs:Chars) _) _)) => isKw(KwChrs, "struct") orBool isKw(KwChrs, "enum")
    rule isDefinition(_) => false [owise]

    // Helper to check if LHS is a let binding (should NOT be hoisted)
    syntax Bool ::= isLetDef(Expr) [function]
    rule isLetDef((Apply (Ident Chrs:Chars) _)) => isKw(Chrs, "let")
    rule isLetDef(_) => false [owise]

    // Helper to check if LHS is struct definition
    syntax Bool ::= isStructDef(Expr) [function]
    rule isStructDef((Apply (Ident Chrs:Chars) _)) => isKw(Chrs, "struct")
    rule isStructDef(_) => false [owise]

    // Helper to check if LHS is enum definition
    syntax Bool ::= isEnumDef(Expr) [function]
    rule isEnumDef((Apply (Ident Chrs:Chars) _)) => isKw(Chrs, "enum")
    rule isEnumDef(_) => false [owise]

    // Hoist a single definition
    syntax KItem ::= hoistOneDef(Expr)

    // Hoist function definition - create closure and bind (discard the value)
    rule <k> hoistOneDef((Apply (Op EqOp:Chars) LHS:Expr Body:Expr))
          => closure(getFnName(LHS), getFnParams(LHS), Body, Env) ~> bindVarDiscard(getFnName(LHS)) ... </k>
         <env> Env:Map </env>
         requires isKw(EqOp, "=") andBool isFnDef(LHS)

    // Hoist struct definition
    rule <k> hoistOneDef((Apply (Apply (Ident KwChrs:Chars) (Ident NameChrs:Chars)) _FieldDefs:Expr))
          => structDef(charsToString(NameChrs), .Map) ~> bindVarDiscard(charsToString(NameChrs)) ... </k>
         requires isKw(KwChrs, "struct")

    // Hoist enum definition
    rule <k> hoistOneDef((Apply (Apply (Ident KwChrs:Chars) (Ident NameChrs:Chars)) _Variants:Expr))
          => enumDef(charsToString(NameChrs), .Map) ~> bindVarDiscard(charsToString(NameChrs)) ... </k>
         requires isKw(KwChrs, "enum")

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

    // ==========================================
    // Keyword and String Comparison Helpers
    // ==========================================

    // Check if chars represent a boolean keyword
    syntax Bool ::= isBoolKeyword(Chars) [function]
    rule isBoolKeyword(Chrs) => charsToString(Chrs) ==String "true" orBool charsToString(Chrs) ==String "false"

    // Check if chars match a specific keyword
    syntax Bool ::= isKw(Chars, String) [function]
    rule isKw(Chrs, Kw) => charsToString(Chrs) ==String Kw

    // Identifier lookup - decode char list and check for booleans
    rule <k> (Ident Chrs:Chars) => true ... </k>
         requires isKw(Chrs, "true")
    rule <k> (Ident Chrs:Chars) => false ... </k>
         requires isKw(Chrs, "false")

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
         requires isKw(LetKw, "let") andBool isKw(EqOp, "=")

    // After expression evaluates to a value, bind it in the environment
    syntax KItem ::= bindVar(String)
                   | bindVarDiscard(String)  // bind but discard result

    rule <k> V:Val ~> bindVar(Name:String) => V ... </k>
         <env> Env:Map => Env[Name <- V] </env>

    rule <k> V:Val ~> bindVarDiscard(Name:String) => .K ... </k>
         <env> Env:Map => Env[Name <- V] </env>

    // ==========================================
    // Variable Lookup
    // ==========================================

    // Look up variable in environment (non-function values)
    rule <k> (Ident Chrs:Chars) => V ... </k>
         <env> ... charsToString(Chrs) |-> V:Val ... </env>
         requires notBool isBoolKeyword(Chrs) andBool notBool isClosure(V)

    // Look up zero-arg function and auto-apply it
    rule <k> (Ident Chrs:Chars) => applyFn(closure(Name, .List, Body, CapturedEnv), .List) ... </k>
         <env> ... charsToString(Chrs) |-> closure(Name, .List, Body, CapturedEnv) ... </env>
         requires notBool isBoolKeyword(Chrs)

    // Look up function with params - return the closure as-is (will be applied later)
    rule <k> (Ident Chrs:Chars) => closure(Name, Params, Body, CapturedEnv) ... </k>
         <env> ... charsToString(Chrs) |-> closure(Name, Params, Body, CapturedEnv) ... </env>
         requires notBool isBoolKeyword(Chrs) andBool size(Params) >Int 0

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
    rule isFnDef((Apply (Ident Chrs:Chars) _)) => isKw(Chrs, "fn")
    rule isFnDef((Apply E:Expr _)) => isFnDef(E)
    rule isFnDef(_) => false [owise]

    // Extract function name - the first Ident after "fn"
    syntax String ::= getFnName(Expr) [function]
    rule getFnName((Apply (Ident FnKw:Chars) (Ident NameChrs:Chars))) => charsToString(NameChrs)
         requires isKw(FnKw, "fn")
    rule getFnName((Apply E:Expr _)) => getFnName(E) [owise]

    // Extract parameters as a List of Strings (in reverse order from parsing)
    syntax List ::= getFnParams(Expr) [function]
    // Base case: (Apply (Ident "fn") (Ident name)) - no params
    rule getFnParams((Apply (Ident FnKw:Chars) (Ident _))) => .List
         requires isKw(FnKw, "fn")
    // Recursive case: (Apply inner (Ident param)) - add param to list
    rule getFnParams((Apply E:Expr (Ident ParamChrs:Chars))) => getFnParams(E) ListItem(charsToString(ParamChrs))
         requires notBool isFnKeyword(E)
    // Handle the case where inner is still fn-related
    rule getFnParams((Apply (Apply (Ident FnKw:Chars) (Ident _)) (Ident ParamChrs:Chars))) 
         => ListItem(charsToString(ParamChrs))
         requires isKw(FnKw, "fn")
    rule getFnParams(_) => .List [owise]

    // Helper to check if an Expr is the fn keyword
    syntax Bool ::= isFnKeyword(Expr) [function]
    rule isFnKeyword((Ident Chrs:Chars)) => isKw(Chrs, "fn")
    rule isFnKeyword(_) => false [owise]

    // Main rule: function definition
    // Pattern: (Apply (Op "=") LHS Body) where isFnDef(LHS)
    rule <k> (Apply (Op EqOp:Chars) LHS:Expr Body:Expr)
          => closure(getFnName(LHS), getFnParams(LHS), Body, Env) 
             ~> bindVar(getFnName(LHS)) ... </k>
         <env> Env:Map </env>
         requires isKw(EqOp, "=") andBool isFnDef(LHS)

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
         requires notBool isBoolKeyword(Chrs) andBool size(Params) >Int 0

    // After single argument evaluates
    syntax KItem ::= evalArg(List, Val)  // accumulated args, closure
    rule <k> V:Val ~> evalArg(Args:List, Clos:Val) 
          => applyFn(Clos, Args ListItem(V)) ... </k>

    // Apply function with multiple arguments - evaluate first, then rest
    rule <k> (Apply (Ident Chrs:Chars) Arg1:Expr Arg2:Expr RestArgs:Exprs) 
          => Arg1 ~> evalArgs(Arg2 RestArgs, .List, closure(Name, Params, Body, CapturedEnv)) ... </k>
         <env> ... charsToString(Chrs) |-> closure(Name, Params, Body, CapturedEnv) ... </env>
         requires notBool isBoolKeyword(Chrs)

    // After arg evaluates, continue with remaining args or apply
    rule <k> V:Val ~> evalArgs(NextArg:Expr RestArgs:Exprs, AccArgs:List, Clos:Val)
          => NextArg ~> evalArgs(RestArgs, AccArgs ListItem(V), Clos) ... </k>

    // All args evaluated - apply the function
    rule <k> V:Val ~> evalArgs(.Exprs, AccArgs:List, Clos:Val)
          => applyFn(Clos, AccArgs ListItem(V)) ... </k>

    // Apply closure: bind all params to args and evaluate body
    // Save current env, switch to closure env with params bound, will restore after
    // For hoisted functions (empty captured env), use calling environment for lookups
    rule <k> applyFn(closure(_, Params:List, Body:Expr, CapturedEnv:Map), Args:List)
          => Body ~> restoreEnv(OrigEnv) ... </k>
         <env> OrigEnv:Map => bindParams(mergeEnvs(CapturedEnv, OrigEnv), Params, Args) </env>
         requires size(Params) ==Int size(Args)

    // Merge environments: captured env takes precedence, but falls back to calling env
    // This allows hoisted functions (with empty captured env) to access module-level bindings
    syntax Map ::= mergeEnvs(Map, Map) [function]
    rule mergeEnvs(CapturedEnv:Map, CallingEnv:Map) => mergeMapInto(CallingEnv, CapturedEnv)

    // Helper to merge all entries from second map into first map (second wins on conflict)
    syntax Map ::= mergeMapInto(Map, Map) [function]
    rule mergeMapInto(Base:Map, .Map) => Base
    rule mergeMapInto(Base:Map, (K:KItem |-> V:KItem) Rest:Map) => mergeMapInto(Base[K <- V], Rest)

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
    // Exclude special form keywords (struct/enum/if) which are not curried function calls
    rule <k> (Apply (Apply E1:Expr E2:Expr .Exprs) E3:Expr RestArgs:Exprs)
          => (Apply E1 E2 .Exprs) ~> evalCurriedArgs(E3 RestArgs) ... </k>
         requires notBool isOp(E1)
          andBool notBool isSpecialFormKw(E1)

    // After inner call evaluates to a closure, apply with remaining args
    syntax KItem ::= evalCurriedArgs(Exprs)
    rule <k> closure(Name, Params, Body, CapturedEnv) ~> evalCurriedArgs(Arg:Expr RestArgs:Exprs)
          => Arg ~> evalArgs(RestArgs, .List, closure(Name, Params, Body, CapturedEnv)) ... </k>

    // Helper to check if an expression is an Op node
    syntax Bool ::= isOp(Expr) [function]
    rule isOp((Op _)) => true
    rule isOp(_) => false [owise]

    // Helper to check if an expression contains a special form keyword at its root (struct, enum, if, then, else)
    // These have their own evaluation rules and should not be treated as curried function calls
    // Recursively checks through nested Apply chains to find the leftmost identifier
    syntax Bool ::= isSpecialFormKw(Expr) [function]
    rule isSpecialFormKw((Ident Chrs:Chars)) => isKw(Chrs, "struct") orBool isKw(Chrs, "enum") 
                                                 orBool isKw(Chrs, "if") orBool isKw(Chrs, "then") orBool isKw(Chrs, "else")
    rule isSpecialFormKw((Apply E:Expr _)) => isSpecialFormKw(E)
    rule isSpecialFormKw(_) => false [owise]

    // Helper function to bind parameters to arguments
    syntax Map ::= bindParams(Map, List, List) [function]
    rule bindParams(Env, .List, .List) => Env
    rule bindParams(Env, ListItem(P:String) RestParams:List, ListItem(A:Val) RestArgs:List)
         => bindParams(Env[P <- A], RestParams, RestArgs)

    // ==========================================
    // Arithmetic Operations
    // ==========================================

    // Addition: a + b
    rule <k> (Apply (Op OpChrs:Chars) E1:Expr E2:Expr)
          => E1 ~> evalBinOp("+", E2) ... </k>
         requires isKw(OpChrs, "+")

    // Subtraction: a - b
    rule <k> (Apply (Op OpChrs:Chars) E1:Expr E2:Expr)
          => E1 ~> evalBinOp("-", E2) ... </k>
         requires isKw(OpChrs, "-")

    // Multiplication: a * b
    rule <k> (Apply (Op OpChrs:Chars) E1:Expr E2:Expr)
          => E1 ~> evalBinOp("*", E2) ... </k>
         requires isKw(OpChrs, "*")

    // Division: a / b
    rule <k> (Apply (Op OpChrs:Chars) E1:Expr E2:Expr)
          => E1 ~> evalBinOp("/", E2) ... </k>
         requires isKw(OpChrs, "/")

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

    // ==========================================
    // Comparison Operations
    // ==========================================

    // Equality: a == b
    rule <k> (Apply (Op OpChrs:Chars) E1:Expr E2:Expr)
          => E1 ~> evalBinOp("==", E2) ... </k>
         requires isKw(OpChrs, "==")

    // Not equal: a != b
    rule <k> (Apply (Op OpChrs:Chars) E1:Expr E2:Expr)
          => E1 ~> evalBinOp("!=", E2) ... </k>
         requires isKw(OpChrs, "!=")

    // Less than: a < b
    rule <k> (Apply (Op OpChrs:Chars) E1:Expr E2:Expr)
          => E1 ~> evalBinOp("<", E2) ... </k>
         requires isKw(OpChrs, "<")

    // Greater than: a > b
    rule <k> (Apply (Op OpChrs:Chars) E1:Expr E2:Expr)
          => E1 ~> evalBinOp(">", E2) ... </k>
         requires isKw(OpChrs, ">")

    // Less than or equal: a <= b
    rule <k> (Apply (Op OpChrs:Chars) E1:Expr E2:Expr)
          => E1 ~> evalBinOp("<=", E2) ... </k>
         requires isKw(OpChrs, "<=")

    // Greater than or equal: a >= b
    rule <k> (Apply (Op OpChrs:Chars) E1:Expr E2:Expr)
          => E1 ~> evalBinOp(">=", E2) ... </k>
         requires isKw(OpChrs, ">=")

    // Comparison results
    rule <k> V2:Int ~> evalBinOp2("==", V1:Int) => V1 ==Int V2 ... </k>
    rule <k> V2:Int ~> evalBinOp2("!=", V1:Int) => V1 =/=Int V2 ... </k>
    rule <k> V2:Int ~> evalBinOp2("<", V1:Int) => V1 <Int V2 ... </k>
    rule <k> V2:Int ~> evalBinOp2(">", V1:Int) => V1 >Int V2 ... </k>
    rule <k> V2:Int ~> evalBinOp2("<=", V1:Int) => V1 <=Int V2 ... </k>
    rule <k> V2:Int ~> evalBinOp2(">=", V1:Int) => V1 >=Int V2 ... </k>

    // Float operations
    rule <k> V2:Float ~> evalBinOp2("+", V1:Float) => V1 +Float V2 ... </k>
    rule <k> V2:Float ~> evalBinOp2("-", V1:Float) => V1 -Float V2 ... </k>
    rule <k> V2:Float ~> evalBinOp2("*", V1:Float) => V1 *Float V2 ... </k>
    rule <k> V2:Float ~> evalBinOp2("/", V1:Float) => V1 /Float V2 ... </k>
         requires V2 =/=Float 0.0

    // ==========================================
    // Conditional (if-then-else)
    // ==========================================

    // Pattern: (Apply (Apply (Apply (Apply (Apply (Ident "if") cond) (Ident "then")) thenExpr) (Ident "else")) elseExpr)
    // We need to match this deeply nested structure

    // Match the full if-then-else pattern
    rule <k> (Apply 
              (Apply 
                (Apply 
                  (Apply 
                    (Apply (Ident IfKw:Chars) CondExpr:Expr)
                    (Ident ThenKw:Chars))
                  ThenExpr:Expr)
                (Ident ElseKw:Chars))
              ElseExpr:Expr)
          => CondExpr ~> evalIf(ThenExpr, ElseExpr) ... </k>
         requires isKw(IfKw, "if") andBool isKw(ThenKw, "then") andBool isKw(ElseKw, "else")

    // After condition evaluates, choose branch
    syntax KItem ::= evalIf(Expr, Expr)  // then branch, else branch
    rule <k> true ~> evalIf(ThenExpr:Expr, _) => ThenExpr ... </k>
    rule <k> false ~> evalIf(_, ElseExpr:Expr) => ElseExpr ... </k>

    // ==========================================
    // Block (scoped multiline expressions)
    // ==========================================

    // Block evaluation: (Apply (Synthetic "__block__") exprs...)
    // Creates a new scope, hoists definitions, evaluates expressions, restores scope
    rule <k> (Apply (Synthetic SynChrs:Chars) Es:Exprs)
          => hoistDefs(Es) ~> seqExprs(filterDefs(Es)) ~> restoreEnv(Env) ... </k>
         <env> Env:Map </env>
         requires isKw(SynChrs, "__block__")

    // ==========================================
    // Compound Types
    // ==========================================

    // Helper function for Synthetic node names
    syntax String ::= "__tuple__"  [function]
    syntax String ::= "__record__" [function]
    syntax String ::= "__list__"   [function]

    // Tuple evaluation: (Apply (Synthetic "__tuple__") elems...)
    rule <k> (Apply (Synthetic SynChrs:Chars) Es:Exprs)
          => evalTupleElems(Es, .List) ... </k>
         requires isKw(SynChrs, "__tuple__")

    // Helper to evaluate tuple elements
    syntax KItem ::= evalTupleElems(Exprs, List)
    rule <k> evalTupleElems(.Exprs, Acc:List) => tuple(Acc) ... </k>
    rule <k> evalTupleElems(E:Expr RestExprs:Exprs, Acc:List)
          => E ~> evalTupleElem(RestExprs, Acc) ... </k>

    syntax KItem ::= evalTupleElem(Exprs, List)
    rule <k> V:Val ~> evalTupleElem(RestExprs:Exprs, Acc:List)
          => evalTupleElems(RestExprs, Acc ListItem(V)) ... </k>

    // List evaluation: (Apply (Synthetic "__list__") elems...)
    rule <k> (Apply (Synthetic SynChrs:Chars) Es:Exprs)
          => evalListElems(Es, .List) ... </k>
         requires isKw(SynChrs, "__list__")

    // Helper to evaluate list elements
    syntax KItem ::= evalListElems(Exprs, List)
    rule <k> evalListElems(.Exprs, Acc:List) => list(Acc) ... </k>
    rule <k> evalListElems(E:Expr RestExprs:Exprs, Acc:List)
          => E ~> evalListElem(RestExprs, Acc) ... </k>

    syntax KItem ::= evalListElem(Exprs, List)
    rule <k> V:Val ~> evalListElem(RestExprs:Exprs, Acc:List)
          => evalListElems(RestExprs, Acc ListItem(V)) ... </k>

    // Record evaluation: (Apply (Synthetic "__record__") fields...)
    // Each field is (Apply (Op "=") (Ident name) value)
    rule <k> (Apply (Synthetic SynChrs:Chars) Es:Exprs)
          => evalRecordFields(Es, .Map) ... </k>
         requires isKw(SynChrs, "__record__")

    // Helper to evaluate record fields
    syntax KItem ::= evalRecordFields(Exprs, Map)
    rule <k> evalRecordFields(.Exprs, Acc:Map) => record(Acc) ... </k>
    rule <k> evalRecordFields((Apply (Op EqOp:Chars) (Ident NameChrs:Chars) ValueExpr:Expr) RestFields:Exprs, Acc:Map)
          => ValueExpr ~> evalRecordField(charsToString(NameChrs), RestFields, Acc) ... </k>
         requires isKw(EqOp, "=")

    syntax KItem ::= evalRecordField(String, Exprs, Map)
    rule <k> V:Val ~> evalRecordField(Name:String, RestFields:Exprs, Acc:Map)
          => evalRecordFields(RestFields, Acc[Name <- V]) ... </k>

    // ==========================================
    // Struct Definition and Construction
    // ==========================================

    // Struct definition: (Apply (Apply (Ident "struct") (Ident name)) record)
    rule <k> (Apply (Apply (Ident KwChrs:Chars) (Ident NameChrs:Chars)) _FieldDefs:Expr)
          => structDef(charsToString(NameChrs), .Map) ~> bindVar(charsToString(NameChrs)) ... </k>
         requires isKw(KwChrs, "struct")

    // ==========================================
    // Enum Definition
    // ==========================================

    // Enum definition: (Apply (Apply (Ident "enum") (Ident name)) variants)
    rule <k> (Apply (Apply (Ident KwChrs:Chars) (Ident NameChrs:Chars)) _Variants:Expr)
          => enumDef(charsToString(NameChrs), .Map) ~> bindVar(charsToString(NameChrs)) ... </k>
         requires isKw(KwChrs, "enum")

    // ==========================================
    // Field Access
    // ==========================================

    // Field access on record: record.field
    // Pattern: (Apply (Op ".") record (Ident field))
    rule <k> (Apply (Op OpChrs:Chars) RecordExpr:Expr (Ident FieldChrs:Chars) .Exprs)
          => RecordExpr ~> getField(charsToString(FieldChrs)) ... </k>
         requires isKw(OpChrs, ".")

    syntax KItem ::= getField(String)
    rule <k> record(Fields:Map) ~> getField(FieldName:String) => {Fields[FieldName]}:>Val ... </k>
         requires FieldName in_keys(Fields)

    rule <k> structVal(_, Fields:Map) ~> getField(FieldName:String) => {Fields[FieldName]}:>Val ... </k>
         requires FieldName in_keys(Fields)

    // ==========================================
    // List Indexing
    // ==========================================

    // TODO Index access: list[index]

    // ==========================================
    // Error handling
    // ==========================================

    // Error nodes represent parse/semantic errors
    // For now, they remain as-is for error modeling
    rule <k> (Error) => (Error) ... </k>

endmodule
