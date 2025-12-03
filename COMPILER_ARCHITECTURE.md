# Cadenza Compiler Architecture

## Overview

This document describes the comprehensive architecture for the Cadenza compiler, covering type checking, multi-phase compilation, module system, code generation, LSP integration, and more. The design prioritizes incremental compilation, excellent error messages, and seamless integration between compile-time and runtime execution.

## Table of Contents

1. [Design Principles](#design-principles)
2. [Multi-Phase Compiler Architecture](#multi-phase-compiler-architecture)
3. [Type System](#type-system)
4. [Module System](#module-system)
5. [Monomorphization and Trait System](#monomorphization-and-trait-system)
6. [Effect System](#effect-system)
7. [Compilation Targets](#compilation-targets)
8. [LSP Integration](#lsp-integration)
9. [MCP Integration](#mcp-integration)
10. [Error Handling and Source Tracking](#error-handling-and-source-tracking)

---

## Design Principles

### Core Philosophy

1. **Compile-time and Runtime Blur**: The evaluator IS the compiler's first phase. Code runs at compile-time to generate more code.
2. **Incremental Everything**: Parsing, evaluation, type checking, and code generation should all be incremental.
3. **Errors are First-Class**: Multi-error reporting, beautiful diagnostics, and precise source tracking throughout.
4. **LSP-Native**: The compiler and LSP server are the same thing—one is queryable, one is batch.
5. **Units and Dimensions**: Type system includes dimensional analysis for physical units.
6. **No Hidden Magic**: Explicit compiler API, explicit phase separation, explicit type constraints.

### Non-Goals

- Runtime type information (RTTI) in compiled code
- Dynamic typing in the language itself
- Backward compatibility during early development

---

## Multi-Phase Compiler Architecture

The compiler operates in distinct phases, with each phase building on the previous:

```
┌─────────────────────────────────────────────────────────────────┐
│                         Source Code                              │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  Phase 1: Lexing & Parsing                                       │
│  - Tokenize source                                               │
│  - Build lossless CST (rowan)                                    │
│  - Preserve all whitespace and comments                          │
│  - Output: CST with full source tracking                         │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  Phase 2: Evaluation (Macro Expansion & Module Building)         │
│  - Tree-walk interpreter evaluates top-level expressions         │
│  - Macro expansion happens here                                  │
│  - Functions/types are accumulated in Compiler state             │
│  - Unevaluated branches are collected for later type checking    │
│  - Output: Compiler state + AST nodes for exports                │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  Phase 3: Type Checking (Hindley-Milner)                         │
│  - Collect type constraints from all code paths                  │
│  - Include unevaluated branches (guards, conditionals)           │
│  - Infer types using constraint solving                          │
│  - Check dimensional analysis for unit types                     │
│  - Validate trait requirements                                   │
│  - Validate effect contexts                                      │
│  - Output: Typed AST + Type signatures for exports               │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  Phase 4: Monomorphization                                       │
│  - Instantiate generic functions with concrete types             │
│  - Resolve trait requirements to concrete implementations        │
│  - Specialize code for each type usage                           │
│  - Output: Monomorphic IR                                        │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  Phase 5: IR Lowering & Optimization                             │
│  - Convert to target-independent IR                              │
│  - Perform optimizations (constant folding, DCE, etc.)           │
│  - Prepare for code generation                                   │
│  - Output: Optimized IR                                          │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  Phase 6: Code Generation                                        │
│  - Target-specific code generation                               │
│  - See "Compilation Targets" section                             │
│  - Output: Executable/WASM/JS/etc.                               │
└─────────────────────────────────────────────────────────────────┘
```

### Key Architectural Decisions

#### Evaluation Before Type Checking

The evaluator runs **before** type checking because:

1. **Macros generate code**: Macros run at compile-time and produce new AST nodes that must be type-checked.
2. **Compile-time computation**: Some types may depend on compile-time computations (e.g., array sizes, unit conversions).
3. **Module building**: The evaluator accumulates exports into the Compiler state, which the type checker validates.

**Note on error handling**: If evaluation fails or produces errors, those diagnostics are recorded in the Compiler state but don't prevent type checking. The type checker operates on both successfully evaluated code and unevaluated branches, ensuring we get comprehensive type errors even when evaluation issues exist. This provides better overall error reporting than stopping at the first evaluation failure.

#### Handling Unevaluated Branches

**Problem**: If the interpreter encounters a conditional that isn't evaluated (e.g., `if false then ... else ...`), we still need to type-check the unevaluated branch.

**Solution**: During evaluation, track all code paths:

```rust
enum EvaluationResult {
    /// Code was evaluated to a value
    Evaluated(Value),
    /// Code was not evaluated but should be type-checked
    Unevaluated(Expr),
}
```

The evaluator emits both evaluated values AND unevaluated expressions to the compiler. The type checker processes both:
- Evaluated paths: Type check against the actual runtime value
- Unevaluated paths: Infer types without executing

**Example**:
```cadenza
let config = load_config()  # Evaluated at compile-time

let process =
    if config.mode == "debug"
        fn x -> debug_print x  # Might not be evaluated
    else
        fn x -> x              # Might not be evaluated
```

Both branches are type-checked even if only one is evaluated.

#### AST Representation

**Question**: Do we use the same AST for the interpreter and for exports to the compiler?

**Answer**: Yes, with annotations.

```rust
pub struct TypedExpr {
    /// The original expression AST node
    expr: Expr,
    /// The inferred type (filled in by type checker)
    ty: Type,
    /// Source location
    span: Span,
}
```

- **Interpreter**: Uses raw `Expr` nodes from the parser
- **After evaluation**: Expressions are annotated with values/types
- **Type checker**: Adds type information to create `TypedExpr`
- **Exports**: Use `TypedExpr` for cross-module type information

This allows:
- Interpreter to work with simple AST
- Type checker to annotate without rebuilding
- Exports to carry full type information

---

## Type System

### Hindley-Milner Type Inference

We use Hindley-Milner (HM) type inference with extensions for:
- Units and dimensional analysis
- Traits and type classes
- Effect types
- Row polymorphism (for records)

#### Type Representation

```rust
pub enum Type {
    /// Concrete types
    Nil,
    Bool,
    Integer,
    Float,
    String,
    
    /// Parametric types
    List(Box<Type>),
    Tuple(Vec<Type>),
    Record(Vec<(InternedString, Type)>),
    
    /// Function types: args... -> return
    Fn(Vec<Type>, Box<Type>),
    
    /// Type variables (for inference)
    Var(TypeVar),
    
    /// Quantified types (for polymorphism)
    Forall(Vec<TypeVar>, Box<Type>),
    
    /// Units and quantities
    Quantity {
        value_type: Box<Type>,  // Integer or Float
        dimension: Dimension,
    },
    
    /// Trait constraints
    Constrained {
        ty: Box<Type>,
        traits: Vec<TraitRef>,
    },
    
    /// Effect types
    Effect {
        result: Box<Type>,
        effects: Vec<EffectRef>,
    },
}
```

#### Type Inference Algorithm

We implement Algorithm W (Damas-Milner) with constraint generation:

1. **Constraint Generation**: Walk the AST and generate type constraints
   ```rust
   fn generate_constraints(expr: &Expr, env: &TypeEnv) -> (Type, Vec<Constraint>)
   ```

2. **Constraint Solving**: Use unification to solve constraints
   ```rust
   fn unify(t1: Type, t2: Type) -> Result<Substitution>
   ```

3. **Generalization**: Introduce `forall` quantifiers at let-bindings
   ```rust
   fn generalize(ty: Type, env: &TypeEnv) -> Type
   ```

4. **Instantiation**: Replace quantified variables with fresh type variables
   ```rust
   fn instantiate(ty: Type) -> Type
   ```

#### Dimensional Analysis Integration

Unit types are checked during type inference:

```cadenza
let distance = 100meter
let time = 5second
let velocity = distance / time  # Type: Quantity<Float, meter/second>

# Type error: cannot add incompatible dimensions
let invalid = distance + time  # Error: cannot add meter and second
```

The type checker maintains dimension equations and solves them alongside type constraints:

```rust
pub struct DimensionConstraint {
    left: Dimension,
    right: Dimension,
    reason: ConstraintReason,
}
```

#### Type Checking Unevaluated Code

When we encounter an unevaluated branch:

1. **Mark as speculative**: Tag the type checking context
2. **Generate constraints**: Same as evaluated code
3. **Solve constraints**: Ensure type soundness
4. **Record result**: Store inferred types but don't generate code

```rust
impl TypeChecker {
    fn check_expr(&mut self, expr: &Expr, expected: Type, mode: CheckMode) -> Result<Type> {
        match mode {
            CheckMode::Evaluated => {
                // Normal type checking + code generation
            }
            CheckMode::Unevaluated => {
                // Type checking only, no code generation
                // Still ensures type soundness
            }
        }
    }
}
```

### Type Errors

Type errors include:
- Expected vs actual type
- Source location with full context
- Suggestion for fixes when possible
- Stack trace showing call chain

---

## Module System

### Module Structure

```cadenza
# mymodule.cdz

# Private function (not exported by default)
let internal_helper = fn x -> x + 1

# Public function (explicit export)
@export
let public_fn = fn x -> x * 2

# Public type
@export
measure meter

# Public macro
@export
defmacro my_macro args = ...
```

### Import/Export Mechanism

#### Exports

Modules explicitly mark exports with `@export` attribute. The evaluator collects these into the compiler state:

```rust
pub struct ModuleExports {
    /// Exported functions with type signatures
    pub functions: Map<InternedString, TypedFunction>,
    
    /// Exported types
    pub types: Map<InternedString, Type>,
    
    /// Exported macros (evaluated at import site)
    pub macros: Map<InternedString, Macro>,
    
    /// Exported units/measures
    pub units: Map<InternedString, Unit>,
}
```

#### Imports

```cadenza
# Import specific items
import mymodule (public_fn, meter)

# Import all exports
import mymodule *

# Qualified import
import mymodule as m
# Access with module prefix (uses function application syntax)
let x = (m.public_fn) 10
```

The type checker ensures:
1. Imported names exist in the source module
2. Type signatures match at the boundary
3. No circular dependencies

#### Cross-Module Type Checking

When importing:

1. **Load module**: Parse and evaluate the imported module
2. **Extract types**: Get type signatures from exports
3. **Check usage**: Verify imported values are used correctly

```rust
pub struct ModuleRegistry {
    /// Loaded modules with their exports
    modules: Map<InternedString, ModuleExports>,
    
    /// Dependency graph for cycle detection
    dependencies: Map<InternedString, Vec<InternedString>>,
}
```

Circular dependencies are detected during evaluation phase before type checking.

---

## Monomorphization and Trait System

### Goals

1. **Generic functions**: Write functions once for many types
2. **Implicit traits**: No manual trait annotations (infer requirements)
3. **Zero-cost abstractions**: Specialized code at compile-time
4. **Type classes**: Support ad-hoc polymorphism like Haskell

### Trait Definition

```cadenza
# Define a trait
@trait
let Numeric = trait
    add : Self -> Self -> Self
    mul : Self -> Self -> Self
    zero : Self

# Implement trait for a type
@impl Numeric for Integer
    add = fn a b -> a + b
    mul = fn a b -> a * b
    zero = 0
```

### Implicit Trait Constraints

**Key Innovation**: Don't require explicit trait bounds. Instead, **infer** them.

```cadenza
# User writes this:
let sum = fn list ->
    fold list 0 (fn acc x -> acc + x)

# Compiler infers:
let sum : forall a. [Numeric a] => List a -> a
```

The type checker:
1. Encounters `acc + x`
2. Requires both operands support `+`
3. Generates constraint: `a : Numeric`
4. Unifies with call sites

#### Trait Constraint Inference Algorithm

```rust
fn infer_traits(expr: &Expr, ty: Type) -> Vec<TraitConstraint> {
    match expr {
        // a + b requires Numeric
        Expr::Apply(op, [a, b]) if op == "+" => {
            vec![TraitConstraint::new(ty, Trait::Numeric)]
        }
        
        // Recursive inference
        Expr::Apply(f, args) => {
            let f_constraints = infer_traits(f, ...);
            let arg_constraints = args.iter().flat_map(|arg| infer_traits(arg, ...));
            f_constraints.chain(arg_constraints).collect()
        }
        
        _ => vec![]
    }
}
```

### Monomorphization

After type checking, we have:
```
fn sum : forall a. [Numeric a] => List a -> a
```

At call sites:
```cadenza
sum [1, 2, 3]        # Instantiate with a = Integer
sum [1.0, 2.0, 3.0]  # Instantiate with a = Float
```

Monomorphization generates specialized versions:

```rust
// Generated code:
fn sum_Integer(list: List<Integer>) -> Integer { ... }
fn sum_Float(list: List<Float>) -> Float { ... }
```

#### Monomorphization Algorithm

1. **Collect instantiations**: Track all call sites
2. **Generate specialized functions**: Create concrete versions
3. **Replace calls**: Update call sites to use specialized functions
4. **Dead code elimination**: Remove unused specializations

```rust
pub struct Monomorphizer {
    /// Map from generic function to instantiations
    instantiations: Map<FunctionId, Vec<Instantiation>>,
    
    /// Generated specialized functions
    specialized: Vec<MonomorphicFunction>,
}

impl Monomorphizer {
    fn instantiate(&mut self, func: &TypedFunction, types: Vec<Type>) -> FunctionId {
        // Check if already instantiated
        if let Some(id) = self.find_instantiation(func.id, &types) {
            return id;
        }
        
        // Create new specialized function
        let specialized = self.specialize(func, types);
        let id = self.specialized.len();
        self.specialized.push(specialized);
        self.instantiations.entry(func.id).or_default().push(Instantiation { types, id });
        id
    }
}
```

---

## Effect System

### Motivation

Effects represent computational context (like Reader monad in Haskell):
- Database connections
- Logging
- Configuration
- Permissions

**Key idea**: Effects are implicit parameters checked at compile-time.

### Effect Definition

```cadenza
# Define an effect
@effect
let Logger = effect
    log : String -> ()

# Function using an effect
let process = fn data ->
    Logger.log "Processing"
    # ... do work
```

### Implicit Effect Propagation

The compiler infers effect requirements:

```cadenza
let top_level = fn data ->
    let result = process data  # process requires Logger
    result                      # So top_level also requires Logger
```

Inferred signature:
```
top_level : forall a. [Logger] => a -> a
```

### Effect Handlers

Provide effect implementations at call sites:

```cadenza
# Define a handler
let console_logger = handler Logger
    log = fn msg -> print "LOG: " msg

# Use the handler
with console_logger
    top_level my_data
```

### Effect Type Checking

Effects extend the type system:

```rust
pub enum Type {
    // ...
    Effect {
        result: Box<Type>,
        effects: Vec<EffectRef>,
    },
}
```

Type checking ensures:
1. Required effects are provided
2. Effect handlers match effect signatures
3. Effects are propagated correctly

```rust
pub struct EffectConstraint {
    /// Required effect
    effect: EffectRef,
    /// Source expression needing the effect
    source: Expr,
    /// Where effect must be provided
    boundary: Span,
}
```

The type checker verifies all effect constraints are satisfied before the top-level boundary.

---

## Compilation Targets

### Target Requirements

1. **Browser (in-browser compilation and execution)**
   - Must compile entirely in the browser
   - No server-side processing required
   - Options: TypeScript, JavaScript, WASM

2. **Standalone Executable**
   - Native performance
   - No runtime dependencies
   - Options: LLVM, Rust, C, Zig, Cranelift

3. **Nice-to-have: Microcontrollers**
   - Embedded systems
   - Resource-constrained environments

### Browser Targets

#### Option 1: WebAssembly (WASM)

**Pros**:
- Near-native performance
- Compact binary format
- Security sandbox
- Well-supported

**Cons**:
- Requires runtime (WASM engine)
- FFI with JS can be complex
- Debugging harder than JS

**Approach**: Use `wasm32-unknown-unknown` target, compile IR to WASM bytecode.

```rust
pub struct WasmBackend {
    module: wasm_encoder::Module,
}

impl WasmBackend {
    fn compile_function(&mut self, func: &MonomorphicFunction) {
        // Emit WASM instructions
    }
}
```

#### Option 2: TypeScript

**Pros**:
- Type safety
- Good tooling
- Easy debugging
- Source maps

**Cons**:
- Requires transpilation step
- Larger output than WASM
- TypeScript runtime needed

**Approach**: Emit TypeScript code from IR, use `tsc` or `esbuild` for bundling.

#### Option 3: JavaScript

**Pros**:
- No compilation needed
- Universal browser support
- Easy debugging

**Cons**:
- No static type checking
- Larger output
- Less efficient than WASM

**Approach**: Emit JS code directly from IR.

#### Recommendation for Browser

**Hybrid approach**:
- **AOT (Ahead-of-time)**: Compile to WASM for production
  - Best performance
  - Smallest bundle size
  
- **In-browser**: Compile to JavaScript for development
  - Easier debugging
  - Source maps
  - No WASM compilation complexity

The compiler can target both from the same IR.

### Standalone Targets

#### Option 1: LLVM

**Pros**:
- Industry-standard
- Excellent optimizations
- Many target architectures
- Used by Rust, Swift, etc.

**Cons**:
- Large dependency
- Complex API
- Longer compile times

**Approach**: Use `inkwell` crate for LLVM bindings.

```rust
pub struct LlvmBackend<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
}
```

#### Option 2: Emit Rust

**Pros**:
- Leverage Rust compiler
- Excellent error messages
- Already set up in our workflow
- Good debugging

**Cons**:
- Requires Rust toolchain
- Slower compilation
- Not truly standalone

**Approach**: Emit Rust code from IR, invoke `rustc`.

#### Option 3: Emit C

**Pros**:
- Universal
- Highly portable
- Mature toolchain
- Works everywhere

**Cons**:
- Manual memory management
- Less type safety
- Older language

**Approach**: Emit C code from IR, invoke `gcc` or `clang`.

#### Option 4: Cranelift

**Pros**:
- Fast compilation
- Designed for JIT/AOT
- Used by Wasmtime
- Pure Rust

**Cons**:
- Less optimization than LLVM
- Fewer target architectures
- Younger project

**Approach**: Use Cranelift as a library for code generation.

```rust
pub struct CraneliftBackend {
    module: cranelift_module::Module<cranelift_module::ObjectBackend>,
}
```

#### Recommendation for Standalone

**Primary**: Cranelift
- Fast compilation (critical for LSP responsiveness)
- Pure Rust (easier integration)
- Good enough optimization

**Optional**: LLVM
- Enable with feature flag for production builds
- Better optimization for release builds

### Intermediate Representation (IR)

All backends consume the same IR:

```rust
pub enum IrInstr {
    /// Load a constant value
    Const(Value),
    
    /// Binary operation
    BinOp {
        op: BinOp,
        lhs: IrValue,
        rhs: IrValue,
    },
    
    /// Function call
    Call {
        func: FunctionId,
        args: Vec<IrValue>,
    },
    
    /// Conditional branch
    Branch {
        cond: IrValue,
        then_block: BlockId,
        else_block: BlockId,
    },
    
    /// Return from function
    Return(IrValue),
}

pub struct IrFunction {
    pub name: InternedString,
    pub params: Vec<IrParam>,
    pub return_ty: Type,
    pub blocks: Vec<IrBlock>,
}
```

This IR is:
- Simple to generate from typed AST
- Easy to optimize
- Translates cleanly to all targets

---

## LSP Integration

### Core Principle

**The LSP server IS the compiler.**

Instead of having separate tools:
- Compiler: batch mode
- LSP: query mode

They share the same codebase and state.

### Architecture

```
┌────────────────────────────────────────────────────────┐
│                    LSP Server                          │
│                                                        │
│  ┌──────────────────────────────────────────────────┐ │
│  │         Incremental Compiler                     │ │
│  │  - Maintains parsed trees                        │ │
│  │  - Tracks evaluation state                       │ │
│  │  - Caches type information                       │ │
│  │  - Incrementally updates on changes              │ │
│  └──────────────────────────────────────────────────┘ │
│                        │                               │
│  ┌─────────────────────┴─────────────────────────┐    │
│  │                                               │    │
│  ▼                                               ▼    │
│  LSP Protocol Handlers              Compiler State    │
│  - textDocument/hover                - Parsed trees   │
│  - textDocument/completion           - Type info      │
│  - textDocument/definition           - Errors         │
│  - textDocument/semanticTokens       - Symbols        │
└────────────────────────────────────────────────────────┘
```

### Incremental Compilation

Use **rowan** (red-green tree) for incremental parsing:

```rust
pub struct IncrementalCompiler {
    /// Current CST (green tree)
    cst: GreenNode,
    
    /// Syntax node (red tree with parent pointers)
    syntax: SyntaxNode,
    
    /// Type information cache
    types: Map<NodeId, Type>,
    
    /// Diagnostics cache
    diagnostics: Vec<Diagnostic>,
}

impl IncrementalCompiler {
    fn update(&mut self, change: TextChange) {
        // 1. Apply change to CST (rowan handles incrementality)
        self.cst = self.cst.apply_change(change);
        
        // 2. Re-evaluate changed nodes only
        let changed_nodes = self.find_changed_nodes();
        for node in changed_nodes {
            self.reevaluate_node(node);
        }
        
        // 3. Re-type-check affected expressions
        self.recheck_types();
    }
}
```

### LSP Features

#### 1. Hover (Type Information)

```rust
fn hover(&self, position: Position) -> Option<Hover> {
    let node = self.find_node_at_position(position)?;
    let ty = self.types.get(&node.id())?;
    
    Some(Hover {
        contents: format!("Type: {}", ty),
    })
}
```

#### 2. Go to Definition

```rust
fn definition(&self, position: Position) -> Option<Location> {
    let node = self.find_node_at_position(position)?;
    
    if let Some(ident) = node.as_ident() {
        // Look up in symbol table
        let def = self.find_definition(ident)?;
        return Some(def.location);
    }
    
    None
}
```

#### 3. Completions

```rust
fn completion(&self, position: Position) -> Vec<CompletionItem> {
    let scope = self.scope_at_position(position);
    
    // Collect visible symbols
    scope.visible_symbols()
        .map(|(name, ty)| CompletionItem {
            label: name.to_string(),
            detail: Some(ty.to_string()),
            kind: CompletionItemKind::Variable,
        })
        .collect()
}
```

#### 4. Semantic Tokens

Provide syntax highlighting based on semantic information:

```rust
fn semantic_tokens(&self) -> Vec<SemanticToken> {
    let mut tokens = vec![];
    
    self.walk_ast(|node, ty| {
        let token_type = match node {
            Node::Ident if self.is_function(node) => SemanticTokenType::Function,
            Node::Ident if self.is_type(node) => SemanticTokenType::Type,
            Node::Ident => SemanticTokenType::Variable,
            Node::Literal => SemanticTokenType::Number,
            _ => return,
        };
        
        tokens.push(SemanticToken {
            delta_line: ...,
            delta_start: ...,
            length: ...,
            token_type: token_type as u32,
            token_modifiers: 0,
        });
    });
    
    tokens
}
```

#### 5. Diagnostics (As-You-Type)

```rust
fn publish_diagnostics(&self, uri: Uri) {
    let diagnostics = self.diagnostics.iter()
        .map(|d| lsp_types::Diagnostic {
            range: d.span.to_lsp_range(),
            severity: Some(d.level.to_lsp_severity()),
            message: d.message.clone(),
            source: Some("cadenza".to_string()),
            ..Default::default()
        })
        .collect();
    
    self.client.publish_diagnostics(uri, diagnostics, None);
}
```

### Responsiveness

Key to LSP performance:
1. **Incremental parsing**: Only re-parse changed regions
2. **Lazy type checking**: Only type-check visible code
3. **Background processing**: Type-check in background thread
4. **Cancellation**: Cancel outdated requests

```rust
pub struct LspServer {
    compiler: Arc<Mutex<IncrementalCompiler>>,
    background_thread: JoinHandle<()>,
    cancel_token: CancellationToken,
}

impl LspServer {
    fn on_change(&mut self, change: TextChange) {
        // Cancel any in-progress type checking
        self.cancel_token.cancel();
        
        // Quick parse + evaluate
        self.compiler.lock().update_fast(change);
        
        // Publish immediate diagnostics (parse errors)
        self.publish_diagnostics();
        
        // Start background type checking
        self.spawn_background_typecheck();
    }
}
```

---

## MCP Integration

### Model Context Protocol (MCP)

MCP is a protocol for LLM agents to interact with tools and data sources. Making it first-class in Cadenza enables:

1. **Language-level MCP support**: Define tools in Cadenza
2. **Type-safe tool definitions**: Tools are typed functions
3. **Automatic MCP server generation**: Compiler generates MCP servers
4. **Agent-friendly APIs**: Optimize for LLM consumption

### MCP Server as Compilation Target

```cadenza
# Define an MCP tool
@mcp_tool
let calculate_area = fn width height ->
    width * height

# Define a tool with description
@mcp_tool(name: "calculate_area", description: "Calculate area of rectangle")
let calculate_area : Integer -> Integer -> Integer = fn width height ->
    width * height

# Compiler generates MCP server that exposes these tools
```

### Generated MCP Server

The compiler can generate:

```rust
pub struct McpServer {
    tools: Map<InternedString, McpTool>,
}

pub struct McpTool {
    name: String,
    description: String,
    input_schema: JsonSchema,
    function: CompiledFunction,
}

impl McpServer {
    fn handle_request(&self, req: McpRequest) -> Result<McpResponse> {
        let tool = self.tools.get(&req.tool_name)?;
        let args = self.parse_args(&req.arguments, &tool.input_schema)?;
        let result = tool.function.call(args)?;
        Ok(McpResponse::new(result))
    }
}
```

### Type System Integration

MCP tools are just typed functions:

```cadenza
# The type system ensures:
let safe_tool : Integer -> Integer -> Integer = fn x y ->
    if x < 0 or y < 0
        error "Negative dimensions not allowed"
    else
        x * y
```

Type signatures automatically generate JSON schemas for MCP:

```json
{
  "name": "safe_tool",
  "description": "...",
  "input_schema": {
    "type": "object",
    "properties": {
      "x": { "type": "integer" },
      "y": { "type": "integer" }
    },
    "required": ["x", "y"]
  }
}
```

### Benefits for LLM Agents

1. **Type safety**: Agents can't call tools with wrong types
2. **Clear contracts**: JSON schemas from type signatures
3. **Discoverable**: Tools list with descriptions
4. **Composable**: Agents can chain tool calls
5. **Debuggable**: Full stack traces for errors

### Compiler Flag

```bash
# Compile to MCP server
cadenza compile --target mcp mytools.cdz -o mcp-server

# Run MCP server
./mcp-server
```

---

## Error Handling and Source Tracking

### Principles

1. **Beautiful errors**: Inspired by Rust, Elm, and Roc
2. **Multiple errors**: Don't stop at the first error
3. **Precise locations**: Exact source spans for every diagnostic
4. **Helpful suggestions**: Offer fixes when possible
5. **Full stack traces**: Show the call chain for runtime errors

### Error Representation

Using `miette` for diagnostic rendering:

```rust
use miette::{Diagnostic, SourceSpan};

#[derive(Debug, Diagnostic)]
pub enum DiagnosticKind {
    #[diagnostic(code(E001), severity(Error))]
    #[diagnostic(help("Add a type annotation or provide more context"))]
    TypeMismatch {
        #[label("expected {expected}")]
        expected_span: SourceSpan,
        expected: Type,
        
        #[label("found {actual}")]
        actual_span: SourceSpan,
        actual: Type,
    },
    
    #[diagnostic(code(E002), severity(Error))]
    UndefinedVariable {
        #[label("not found in scope")]
        span: SourceSpan,
        name: InternedString,
        
        #[help]
        suggestion: Option<String>,
    },
    
    #[diagnostic(code(E003), severity(Error))]
    DimensionMismatch {
        #[label("dimension: {left_dim}")]
        left_span: SourceSpan,
        left_dim: Dimension,
        
        #[label("dimension: {right_dim}")]
        right_span: SourceSpan,
        right_dim: Dimension,
        
        operation: String,
    },
}
```

### Example Error Output

```
error[E001]: type mismatch
  ┌─ src/main.cdz:10:5
  │
10│     let x = "hello"
  │             ------- expected Integer
11│     calculate_area x 5
  │                    ^ found String
  │
  = help: Add a type annotation or provide more context
```

### Source Tracking

Every AST node carries source information:

```rust
pub struct Expr {
    kind: ExprKind,
    span: Span,
    file: InternedString,
}

pub struct Span {
    start: u32,
    end: u32,
}
```

The `Span` maps to line/column via a `SourceMap`:

```rust
pub struct SourceMap {
    files: Map<InternedString, SourceFile>,
}

pub struct SourceFile {
    name: InternedString,
    source: String,
    line_starts: Vec<u32>,
}

impl SourceFile {
    fn position(&self, offset: u32) -> (usize, usize) {
        // Binary search to find line number (returns Ok for exact match, Err for insertion point)
        let line = match self.line_starts.binary_search(&offset) {
            Ok(line) => line,
            Err(line) => line.saturating_sub(1), // Line before the insertion point
        };
        // Calculate column (with bounds check)
        let line_start = self.line_starts.get(line).copied().unwrap_or(0);
        let column = offset.saturating_sub(line_start);
        (line, column as usize)
    }
}
```

### Stack Traces

Runtime errors include full call stacks:

```rust
pub struct StackFrame {
    function: InternedString,
    file: InternedString,
    span: Span,
}

pub struct RuntimeError {
    kind: ErrorKind,
    stack: Vec<StackFrame>,
}
```

Example output:

```
error: division by zero
  ┌─ src/math.cdz:15:12
  │
15│     x / y
  │         ^ division by zero
  │
  = stack trace:
    0: divide (src/math.cdz:15:12)
    1: calculate (src/math.cdz:20:5)
    2: main (src/main.cdz:5:10)
```

### Multi-Error Reporting

The compiler collects all diagnostics:

```rust
pub struct Compiler {
    diagnostics: Vec<Diagnostic>,
    // ...
}

impl Compiler {
    fn record_diagnostic(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }
    
    fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.is_error())
    }
    
    fn report_all(&self) {
        for diagnostic in &self.diagnostics {
            eprintln!("{:?}", miette::Report::new(diagnostic.clone()));
        }
    }
}
```

---

## Implementation Roadmap

### Phase 1: Foundation (Current)
- ✅ Lexer and parser
- ✅ Tree-walk evaluator
- ✅ Basic value types
- ✅ Macro expansion
- ✅ Unit system

### Phase 2: Type System
- [ ] Implement HM type inference
- [ ] Add type checker after evaluation
- [ ] Integrate dimensional analysis
- [ ] Handle unevaluated branches
- [ ] Add type annotations (optional)

### Phase 3: Module System
- [ ] Define module structure
- [ ] Implement import/export
- [ ] Cross-module type checking
- [ ] Module registry
- [ ] Dependency resolution

### Phase 4: Traits and Effects
- [ ] Define trait system
- [ ] Implement trait inference
- [ ] Add effect types
- [ ] Effect handlers
- [ ] Constraint solving

### Phase 5: Code Generation
- [ ] Design IR
- [ ] Implement monomorphization
- [ ] IR optimization passes
- [ ] JavaScript backend (for browser)
- [ ] Cranelift backend (for native)

### Phase 6: LSP
- [ ] LSP server skeleton
- [ ] Incremental compilation
- [ ] Hover/go-to-definition
- [ ] Completions
- [ ] Semantic tokens
- [ ] As-you-type diagnostics

### Phase 7: Advanced Features
- [ ] WASM backend
- [ ] LLVM backend (optional)
- [ ] MCP integration
- [ ] Advanced optimizations
- [ ] Debugger support

---

## Open Questions

1. **AST cloning**: Currently `Expr` doesn't implement `Clone`. Do we need to clone for macro expansion, or can we work with references?

2. **Type annotation syntax**: What syntax for optional type annotations?
   ```cadenza
   let x : Integer = 42
   let f : Integer -> Integer = fn x -> x + 1
   ```

3. **Effect syntax**: How do users define and use effects?
   ```cadenza
   @effect
   let Database = effect
       query : String -> Result
   ```

4. **Trait syntax**: How verbose should trait definitions be?

5. **Code size**: With monomorphization, code size can explode. When do we use dynamic dispatch?

6. **FFI**: How do we call into Rust/JS/C from Cadenza?

7. **Package manager**: How do we distribute and install libraries?

---

## Conclusion

This architecture provides:

✅ **Type safety** through HM inference  
✅ **Correctness** by checking all code paths  
✅ **Performance** via monomorphization and multiple backends  
✅ **Developer experience** with LSP integration  
✅ **Extensibility** through MCP support  
✅ **Beautiful errors** with precise source tracking  

The multi-phase design allows each stage to focus on its concerns while maintaining a clean separation. The evaluator bootstraps the language, the type checker ensures soundness, and code generation produces efficient artifacts.

Next steps: Begin implementing Phase 2 (Type System) starting with basic HM inference.
