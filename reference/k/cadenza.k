// Cadenza Language Definition in K Framework
// ===========================================
//
// This is a reference implementation of the Cadenza language semantics.
// It accepts parenthesized AST produced by `cadenza ast` command.
//
// Scoping Model:
// - Uses mutable scopes with a scope chain (parent references)
// - Closures capture values by copy at definition time (immutable capture)
// - Functions are hoisted but cannot be called until captures are ready
// - Lexical scoping: functions capture from their definition scope
// - Use Cell<T> for shared mutable state

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

    // Scope identifier (unique integer)
    syntax ScopeId ::= Int

    // Values - the result of evaluation
    syntax Val ::= Int
                 | Float
                 | String
                 | Bool
                 | closure(String, List, Expr, Map)        // name, params, body, captured env (by value)
                 | tuple(List)                             // tuple value (list of vals)
                 | record(Map)                             // record value (map of String -> Val)
                 | list(List)                              // list value (list of vals)
                 | structVal(String, Map)                  // struct value (name, fields)
                 | enumVal(String, String, Val)            // enum value (enum name, variant name, data)
                 | cell(Int)                               // cell reference (points to store)
                 | pending(String)                         // placeholder for hoisted but not-yet-ready function
                 | RuntimeError()                          // runtime error value (terminal)

    // Type descriptors (for struct/enum definitions)
    syntax TypeDef ::= structDef(String, Map)         // struct definition (name, field types)
                     | enumDef(String, Map)           // enum definition (name, variant -> type)

    // TypeDef is a Val so it can be stored in the environment
    syntax Val ::= TypeDef

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
        <scopes>
            // Scope 0 is the root/global scope with no parent
            0 |-> scope(-1, .Map)
        </scopes>
        <current-scope> 0 </current-scope>  // Currently active scope ID
        <next-scope> 1 </next-scope>        // Next available scope ID
        <store> .Map </store>               // Memory store (for Cell values)
        <next-cell> 0 </next-cell>          // Next available cell ID
        <output> .List </output>            // Output for display
        <file> "" </file>                   // Current file path
        <span-start> 0 </span-start>        // Current span start position
        <span-end> 0 </span-end>            // Current span end position

    // Scope structure: parent scope ID and local bindings
    syntax Scope ::= scope(ScopeId, Map)  // parent, bindings

    // ==========================================
    // Scope Operations
    // ==========================================

    // Create a new child scope and return its ID
    syntax KItem ::= pushScope()
                   | popScope(ScopeId)  // restore to given scope ID

    // Push creates a new scope with current as parent
    rule <k> pushScope() => .K ... </k>
         <scopes> Scopes:Map => Scopes[NextId <- scope(Current, .Map)] </scopes>
         <current-scope> Current:Int => NextId </current-scope>
         <next-scope> NextId:Int => NextId +Int 1 </next-scope>

    // Pop restores to the given scope ID
    rule <k> popScope(SavedScope:Int) => .K ... </k>
         <current-scope> _ => SavedScope </current-scope>

    // Bind a variable in the current scope
    syntax KItem ::= bindInScope(String, Val)

    rule <k> bindInScope(Name:String, V:Val) => .K ... </k>
         <scopes> ... Current |-> scope(Parent, Bindings:Map) => Current |-> scope(Parent, Bindings[Name <- V]) ... </scopes>
         <current-scope> Current:Int </current-scope>

    // Look up a variable by walking the scope chain
    // Note: If we find a `pending` value, continue looking in parent scopes
    // This allows recursive calls to find the actual closure in outer scopes
    syntax Val ::= lookupVar(String, ScopeId, Map) [function]

    // Found in current scope and it's NOT pending - return it
    rule lookupVar(Name:String, ScopeId:Int, (ScopeId |-> scope(_, Bindings:Map)) _:Map)
         => {Bindings[Name]}:>Val
         requires Name in_keys(Bindings)
          andBool notBool isPendingVal({Bindings[Name]}:>Val)

    // Found in current scope but it's pending - continue to parent to find actual value
    rule lookupVar(Name:String, ScopeId:Int, Scopes:Map)
         => lookupVar(Name, getParent(ScopeId, Scopes), Scopes)
         requires ScopeId in_keys(Scopes)
          andBool Name in_keys(getBindings(ScopeId, Scopes))
          andBool isPendingVal({getBindings(ScopeId, Scopes)[Name]}:>Val)
          andBool getParent(ScopeId, Scopes) >=Int 0

    // Not in current scope, walk to parent (if parent exists)
    rule lookupVar(Name:String, ScopeId:Int, Scopes:Map)
         => lookupVar(Name, getParent(ScopeId, Scopes), Scopes)
         requires ScopeId in_keys(Scopes)
          andBool notBool (Name in_keys(getBindings(ScopeId, Scopes)))
          andBool getParent(ScopeId, Scopes) >=Int 0
         [owise]

    // Helper to check if value is pending (for function version)
    syntax Bool ::= isPendingVal(Val) [function]
    rule isPendingVal(pending(_)) => true
    rule isPendingVal(_) => false [owise]

    // Helper to get bindings from a scope
    syntax Map ::= getBindings(ScopeId, Map) [function]
    rule getBindings(ScopeId:Int, (ScopeId |-> scope(_, Bindings:Map)) _:Map) => Bindings

    // Helper to get parent from a scope
    syntax Int ::= getParent(ScopeId, Map) [function]
    rule getParent(ScopeId:Int, (ScopeId |-> scope(Parent:Int, _)) _:Map) => Parent

    // Check if a variable exists in scope chain
    syntax Bool ::= varExists(String, ScopeId, Map) [function]
    rule varExists(Name:String, ScopeId:Int, Scopes:Map)
         => Name in_keys(getBindings(ScopeId, Scopes))
            orBool (getParent(ScopeId, Scopes) >=Int 0
                    andBool varExists(Name, getParent(ScopeId, Scopes), Scopes))
         requires ScopeId in_keys(Scopes)
    rule varExists(_, ScopeId:Int, Scopes:Map) => false
         requires notBool (ScopeId in_keys(Scopes))

    // Capture environment: collect all visible variables into a Map
    syntax Map ::= captureEnv(ScopeId, Map) [function]
    rule captureEnv(ScopeId:Int, Scopes:Map)
         => mergeMapInto(captureEnv(getParent(ScopeId, Scopes), Scopes), getBindings(ScopeId, Scopes))
         requires ScopeId in_keys(Scopes) andBool getParent(ScopeId, Scopes) >=Int 0
    rule captureEnv(ScopeId:Int, Scopes:Map)
         => getBindings(ScopeId, Scopes)
         requires ScopeId in_keys(Scopes) andBool getParent(ScopeId, Scopes) <Int 0
    rule captureEnv(ScopeId:Int, _:Map) => .Map
         requires ScopeId <Int 0

    // Helper to merge all entries from second map into first map (second wins on conflict)
    syntax Map ::= mergeMapInto(Map, Map) [function]
    rule mergeMapInto(Base:Map, .Map) => Base
    rule mergeMapInto(Base:Map, (K:KItem |-> V:KItem) Rest:Map) => mergeMapInto(Base[K <- V], Rest)

    // ==========================================
    // File and Span Handling
    // ==========================================

    // Helper construct for sequential expression evaluation (not an Expr to avoid overlap)
    syntax KItem ::= seqExprs(Exprs)

    // Process File node - set file path, hoist definitions, then evaluate all expressions in order
    // Hoisting: collect all function/struct/enum names first (as pending placeholders)
    // Then evaluate ALL expressions in source order (including definitions)
    // This ensures let bindings are evaluated before function definitions that capture them
    rule <k> (File (Path P:Chars) Es:Exprs) => hoistDefs(Es) ~> seqExprs(Es) ... </k>
         <file> _ => charsToString(P) </file>

    // Hoisting: scan expressions for definitions and bind placeholder names
    // This makes names available but not yet callable
    syntax KItem ::= hoistDefs(Exprs)

    // Empty - done hoisting
    rule <k> hoistDefs(.Exprs) => .K ... </k>

    // Hoist through Span wrapper
    rule <k> hoistDefs((Span (_ _) E:Expr) RestExprs:Exprs)
          => hoistDefs(E RestExprs) ... </k>

    // Hoist function/struct/enum definition - create pending placeholder
    rule <k> hoistDefs((Apply (Op EqOp:Chars) LHS:Expr _Body:Expr) RestExprs:Exprs)
          => hoistOneDef(LHS) ~> hoistDefs(RestExprs) ... </k>
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

    // Hoist a single definition - just bind the name with a pending placeholder
    syntax KItem ::= hoistOneDef(Expr)

    // Hoist function definition - bind name as pending
    rule <k> hoistOneDef(LHS:Expr) => bindInScope(getFnName(LHS), pending(getFnName(LHS))) ... </k>
         requires isFnDef(LHS)

    // Hoist struct definition - bind name as pending
    rule <k> hoistOneDef((Apply (Ident KwChrs:Chars) (Ident NameChrs:Chars)))
          => bindInScope(charsToString(NameChrs), pending(charsToString(NameChrs))) ... </k>
         requires isKw(KwChrs, "struct")

    // Hoist enum definition - bind name as pending
    rule <k> hoistOneDef((Apply (Ident KwChrs:Chars) (Ident NameChrs:Chars)))
          => bindInScope(charsToString(NameChrs), pending(charsToString(NameChrs))) ... </k>
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

    // After expression evaluates to a value, bind it in the current scope
    syntax KItem ::= bindVar(String)

    rule <k> V:Val ~> bindVar(Name:String) => V ... </k>
         <scopes> ... Current |-> scope(Parent, Bindings:Map) => Current |-> scope(Parent, Bindings[Name <- V]) ... </scopes>
         <current-scope> Current:Int </current-scope>

    // ==========================================
    // Variable Lookup
    // ==========================================

    // Variable lookup uses a helper to avoid repeated lookups
    // First, we convert the identifier lookup into a helper that will do the actual lookup
    rule <k> (Ident Chrs:Chars) => lookupIdent(charsToString(Chrs)) ... </k>
         requires notBool isBoolKeyword(Chrs)

    // Helper construct to perform variable lookup
    syntax KItem ::= lookupIdent(String)

    // Look up variable in scope chain and handle based on value type
    // Non-closure, non-pending: return the value directly
    rule <k> lookupIdent(Name:String) => lookupVar(Name, Current, Scopes) ... </k>
         <scopes> Scopes:Map </scopes>
         <current-scope> Current:Int </current-scope>
         requires varExists(Name, Current, Scopes)
          andBool notBool isClosure(lookupVar(Name, Current, Scopes))
          andBool notBool isPending(lookupVar(Name, Current, Scopes))

    // Look up zero-arg function and auto-apply it
    rule <k> lookupIdent(Name:String) => applyFn(lookupVar(Name, Current, Scopes), .List) ... </k>
         <scopes> Scopes:Map </scopes>
         <current-scope> Current:Int </current-scope>
         requires varExists(Name, Current, Scopes)
          andBool isClosure(lookupVar(Name, Current, Scopes))
          andBool isZeroArgClosure(lookupVar(Name, Current, Scopes))

    // Look up function with params - return the closure as-is (will be applied later)
    rule <k> lookupIdent(Name:String) => lookupVar(Name, Current, Scopes) ... </k>
         <scopes> Scopes:Map </scopes>
         <current-scope> Current:Int </current-scope>
         requires varExists(Name, Current, Scopes)
          andBool isClosure(lookupVar(Name, Current, Scopes))
          andBool notBool isZeroArgClosure(lookupVar(Name, Current, Scopes))

    // Error: trying to use a pending (not-yet-ready) function
    // This should produce a runtime error - function captures not yet evaluated
    rule <k> lookupIdent(Name:String) => (Error) ... </k>
         <scopes> Scopes:Map </scopes>
         <current-scope> Current:Int </current-scope>
         requires varExists(Name, Current, Scopes)
          andBool isPending(lookupVar(Name, Current, Scopes))

    // Helper to check if value is a closure
    syntax Bool ::= isClosure(Val) [function]
    rule isClosure(closure(_, _, _, _)) => true
    rule isClosure(_) => false [owise]

    // Helper to check if value is a zero-arg closure
    syntax Bool ::= isZeroArgClosure(Val) [function]
    rule isZeroArgClosure(closure(_, .List, _, _)) => true
    rule isZeroArgClosure(_) => false [owise]

    // Helper to check if value is pending
    syntax Bool ::= isPending(Val) [function]
    rule isPending(pending(_)) => true
    rule isPending(_) => false [owise]

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

    // Main rule: function definition (non-hoisted, e.g., in expressions)
    // Pattern: (Apply (Op "=") LHS Body) where isFnDef(LHS)
    rule <k> (Apply (Op EqOp:Chars) LHS:Expr Body:Expr)
          => closure(getFnName(LHS), getFnParams(LHS), Body, captureEnv(CurrentScope, Scopes))
             ~> bindVar(getFnName(LHS)) ... </k>
         <scopes> Scopes:Map </scopes>
         <current-scope> CurrentScope:Int </current-scope>
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
          => Arg ~> evalArg(.List, lookupVar(charsToString(Chrs), Current, Scopes)) ... </k>
         <scopes> Scopes:Map </scopes>
         <current-scope> Current:Int </current-scope>
         requires notBool isBoolKeyword(Chrs)
          andBool varExists(charsToString(Chrs), Current, Scopes)
          andBool isClosure(lookupVar(charsToString(Chrs), Current, Scopes))
          andBool notBool isZeroArgClosure(lookupVar(charsToString(Chrs), Current, Scopes))

    // After single argument evaluates
    syntax KItem ::= evalArg(List, Val)  // accumulated args, closure
    rule <k> V:Val ~> evalArg(Args:List, Clos:Val) 
          => applyFn(Clos, Args ListItem(V)) ... </k>

    // Apply function with multiple arguments - evaluate first, then rest
    rule <k> (Apply (Ident Chrs:Chars) Arg1:Expr Arg2:Expr RestArgs:Exprs) 
          => Arg1 ~> evalArgs(Arg2 RestArgs, .List, lookupVar(charsToString(Chrs), Current, Scopes)) ... </k>
         <scopes> Scopes:Map </scopes>
         <current-scope> Current:Int </current-scope>
         requires notBool isBoolKeyword(Chrs)
          andBool varExists(charsToString(Chrs), Current, Scopes)
          andBool isClosure(lookupVar(charsToString(Chrs), Current, Scopes))

    // After arg evaluates, continue with remaining args or apply
    rule <k> V:Val ~> evalArgs(NextArg:Expr RestArgs:Exprs, AccArgs:List, Clos:Val)
          => NextArg ~> evalArgs(RestArgs, AccArgs ListItem(V), Clos) ... </k>

    // All args evaluated - apply the function
    rule <k> V:Val ~> evalArgs(.Exprs, AccArgs:List, Clos:Val)
          => applyFn(Clos, AccArgs ListItem(V)) ... </k>

    // Apply closure: create new scope and populate with captured env + args
    // The new scope has the current scope as parent so recursive calls can find actual closures
    rule <k> applyFn(closure(_, Params:List, Body:Expr, CapturedEnv:Map), Args:List)
          => pushScopeWithEnvAndParent(CapturedEnv, CurrentScope) ~> bindParamsK(Params, Args) ~> Body ~> popScopeRestore(CurrentScope) ... </k>
         <current-scope> CurrentScope:Int </current-scope>
         requires size(Params) ==Int size(Args)

    // Create new scope with captured environment and parent for recursive lookups
    syntax KItem ::= pushScopeWithEnvAndParent(Map, ScopeId)
                   | popScopeRestore(ScopeId)

    rule <k> pushScopeWithEnvAndParent(CapturedEnv:Map, ParentScope:Int) => .K ... </k>
         <scopes> Scopes:Map => Scopes[NextId <- scope(ParentScope, CapturedEnv)] </scopes>
         <current-scope> _ => NextId </current-scope>
         <next-scope> NextId:Int => NextId +Int 1 </next-scope>

    rule <k> V:Val ~> popScopeRestore(SavedScope:Int) => V ... </k>
         <current-scope> _ => SavedScope </current-scope>

    // Bind parameters to arguments in current scope
    syntax KItem ::= bindParamsK(List, List)

    rule <k> bindParamsK(.List, .List) => .K ... </k>

    rule <k> bindParamsK(ListItem(P:String) RestParams:List, ListItem(A:Val) RestArgs:List)
          => bindInScope(P, A) ~> bindParamsK(RestParams, RestArgs) ... </k>

    // Partial application: more params than args - create new closure with bound args merged into captured env
    rule <k> applyFn(closure(Name, Params:List, Body:Expr, CapturedEnv:Map), Args:List)
          => closure(Name, dropParams(Params, size(Args)), Body, bindParamsToMap(CapturedEnv, takeParams(Params, size(Args)), Args)) ... </k>
         requires size(Params) >Int size(Args)

    // Helper function to bind parameters to a map (for partial application)
    syntax Map ::= bindParamsToMap(Map, List, List) [function]
    rule bindParamsToMap(Env:Map, .List, .List) => Env
    rule bindParamsToMap(Env:Map, ListItem(P:String) RestParams:List, ListItem(A:Val) RestArgs:List)
         => bindParamsToMap(Env[P <- A], RestParams, RestArgs)

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
    // Creates a new scope, hoists definitions, evaluates all expressions in order, restores scope
    rule <k> (Apply (Synthetic SynChrs:Chars) Es:Exprs)
          => pushScope() ~> hoistDefs(Es) ~> seqExprs(Es) ~> popScopeRestoreK(CurrentScope) ... </k>
         <current-scope> CurrentScope:Int </current-scope>
         requires isKw(SynChrs, "__block__")

    // Helper to pop scope and restore, consuming unit result
    syntax KItem ::= popScopeRestoreK(ScopeId)
    rule <k> V:Val ~> popScopeRestoreK(SavedScope:Int) => V ... </k>
         <current-scope> _ => SavedScope </current-scope>

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
    // Convert to the terminal RuntimeError value
    rule <k> (Error) => RuntimeError() ... </k>

endmodule
