# Cadenza Meta-Compiler

This document describes the meta-compiler architecture for Cadenza, a system that allows compiler semantics to be defined declaratively in Rust data structures and automatically generates efficient query-based implementations. This replaces the imperative tree-walking approach originally attempted.

## Problem Statement

The initial implementation attempted to follow Carp's Object pattern, where a single data structure is progressively annotated through each compilation phase. While this approach works well in Carp's Haskell implementation, it presents difficulties:

First, the evaluator requires manual traversal of the syntax tree with explicit decisions at each node. This makes the code imperative rather than declarative, coupling the semantics tightly to the implementation details of tree walking.

Second, type checking needs to traverse the entire graph to analyze relationships between expressions. The Object-based approach makes this traversal awkward because control flow would discard unevaluated branches, which makes a program only typechecked on evaluated branches, which is unsound.

Third, extending the compiler with new phases requires modifying existing traversal code. There's no clean separation between what the compiler should compute and how it computes it.

The meta-compiler solves these problems by separating the definition of compiler semantics from their implementation. Semantics are defined as pure data structures describing patterns and transformation rules. These definitions are then analyzed and compiled into efficient Rust code that implements the desired behavior.

## Core Architecture

The meta-compiler operates in three distinct stages. In the definition stage, compiler writers construct rule definitions using Rust's type system. In the analysis stage, these definitions are validated and optimized. In the generation stage, efficient query implementations are produced.

### Stage One: Semantic Definition

Compiler semantics are defined as Rust data structures, not functions. A semantic definition consists of one or more queries, where each query defines how to compute a particular attribute of a syntax node. For example, the `eval` query defines how to evaluate expressions to values, while the `type_of` query defines how to infer types.

Each query is defined by a set of rules. A rule consists of three parts: a pattern that matches certain syntax structures, an optional guard that imposes additional constraints, and an expression that computes the result when the pattern matches.

Patterns are structural descriptions of syntax trees. They can match literal values, capture subexpressions into named variables, or destructure complex nodes into their components. Guards allow patterns to be refined with additional predicates, such as checking whether a captured expression has a particular type.

Result expressions describe computations using captured pattern variables. They can reference other queries, creating dependencies between different aspects of the compilation process. For instance, evaluating a function call might query the type of the callee to verify it's actually callable.

The key insight is that these definitions are ordinary Rust values. They can be inspected, validated, and transformed at build time. There's no macro magic or code generation happening at the definition site.

### Stage Two: Analysis

Once semantic definitions are constructed, the build script analyzes them before generating any code. This analysis serves multiple purposes.

Dependency analysis builds a graph showing which queries depend on which others. This graph is used to detect cycles and order code generation. For example, if `eval` queries `type_of` and `type_of` queries `eval`, there's a cyclic dependency that needs special handling.

Pattern analysis checks whether rules within a query might overlap. When two patterns could both match the same syntax, the compiler must decide which takes precedence. The analysis can detect these ambiguities and either report them as errors or use explicit priority annotations to resolve them.

Type analysis verifies that expressions in rules are well-typed. If a rule claims to return an integer but actually constructs a string, that's caught during analysis rather than at runtime.

Optimization analysis identifies opportunities to improve the generated code. For example, if multiple rules share a common pattern prefix, the generated code can factor out that common matching logic.

### Stage Three: Code Generation

After analysis, the build script generates Rust source code implementing the query system. The generated code uses a demand-driven computation framework with memoization and dependency tracking.

For each query, the generator produces a function that pattern-matches on its input. The matching code is structured as a decision tree derived from the query's rules. This tree is optimized during code generation to minimize redundant checks.

When a rule's expression references another query, the generated code includes a call to that query's generated function. The framework tracks these dependencies automatically, invalidating cached results when their dependencies change.

The generator also produces the trait definition that ties all queries together. User code implements this trait's input queries, providing the source syntax tree and other foundational data. The generated query functions are then available as trait methods.

## Handling Mutable State

Many compilation phases need to maintain state that changes during traversal. Evaluation needs an environment mapping variables to values. Ownership analysis needs to track which values are currently owned. Effect checking needs to know which effect handlers are in scope.

Rather than using mutable state internally, queries thread state explicitly through their computations. A query that needs state takes both the syntax node and the current state as input, and returns both the computed result and the updated state.

For example, `eval` takes a node and an environment, and returns a value and a new environment. When evaluating a let binding, the query computes the bound value, extends the environment with the binding, evaluates the body in the extended environment, and returns the body's result.

State threading composes naturally. A query can thread multiple contexts simultaneously: environment for variable bindings, effect context for available effects, and memory state for ownership tracking. Each context flows through the computation as an explicit parameter and return value.

This approach makes state changes visible in the semantic definition. When a rule modifies the environment, that modification appears explicitly in the result expression. There's no hidden mutation to track mentally.

## Source Tracking and Error Recovery

Every computed value carries source location information. When a query produces a result, that result is tagged with the syntax node that produced it. This allows error messages to point precisely to the problematic code.

Queries don't return Option types that silently discard errors. Instead, they return Result types that accumulate diagnostics. When a query encounters an error, it records a diagnostic message with source location and continues processing. This allows the compiler to report multiple errors in a single run rather than stopping at the first problem.

Error recovery is built into the pattern matching semantics. When a rule's pattern matches but its computation fails, the query can continue trying other rules. This is different from throwing an exception that aborts the entire query.

Diagnostic messages themselves are structured data. They include a primary location, optional secondary locations with explanatory labels, and optional notes providing additional context. This structure maps naturally to the kind of error display users expect from modern compilers.

The generated code integrates error recovery throughout. When a rule computation returns an error, the generated matcher continues to the next rule. Only when all rules have been tried does the query propagate the accumulated errors upward.

## Query Composition

Queries frequently need to call other queries. This composition is what makes the system declarative. Rather than manually walking the syntax tree, queries simply state what information they need and let the framework handle the traversal.

Consider type inference for function application. The query needs the callee's type and the argument types. It expresses this by calling `type_of` on the callee and on each argument. These calls might trigger further type inference on those subexpressions. The framework manages this recursive traversal automatically.

Query composition also enables mutual recursion. Type inference might need to evaluate constant expressions to resolve type variables. Evaluation might need type information to perform trait resolution. Each query can freely call the others without worrying about infinite loops, because the framework detects cycles and handles them appropriately.

The composition mechanism extends to external queries. Some computations can't be expressed purely in terms of pattern matching, such as calling external WASM components or invoking SMT solvers. These are marked as external implementations and provided directly in Rust. The generated code treats them the same as generated queries.

## Constraint Solving

Type inference and other analyses generate constraints that must be solved. For example, unifying two types produces a substitution that makes them equal. The meta-compiler models constraint solving as queries that return updated substitution state.

A constraint solving query takes a set of constraints and a current substitution, picks one constraint to solve, computes the updated substitution, and recursively solves the remaining constraints. This transforms iteration into recursion, which the query system handles naturally.

The framework provides cycle detection that prevents infinite recursion when constraints form circular dependencies. If solving constraint A requires solving constraint B, which requires solving A again, the framework detects this and can report it as an error or handle it with a special fixpoint computation.

Constraint solving can be incremental. If only part of the program changes, only the constraints affected by that change need to be resolved. The framework's dependency tracking ensures that cached solutions remain valid when possible.

## Control Flow Analysis

Analyzing control flow requires tracking state along multiple execution paths. When analyzing an if expression, the ownership state after the condition must be threaded through both the then branch and the else branch independently. At the merge point, the states from both branches must be combined.

The meta-compiler handles this by making state merging explicit. Rules that handle conditional expressions evaluate both branches with the same initial state, producing two result states. A merge operation combines these states according to the semantics being implemented.

For ownership analysis, the merge computes the intersection of owned values, because only values owned in all branches are definitely owned after the merge. For type inference, the merge might compute a union type representing the possible types from different branches.

Loops present a special challenge because the loop body might be executed multiple times. The meta-compiler models this by analyzing the loop body twice: once with the initial state and once with the state produced by the first analysis. If these two analyses produce different states, there's a state evolution that must be accounted for.

## Macro Expansion

Macros receive syntax nodes and produce new syntax nodes. Since the graph representation uses stable node identifiers, macro expansion doesn't modify existing nodes but rather creates new subgraphs.

A macro expansion query takes a macro call node and returns a new node representing the expanded code. The original node remains in the graph for source tracking purposes. When later phases process the expanded code, they can trace back through the expansion to the original macro call.

Macro hygiene is maintained by tracking scopes explicitly. When a macro expands, it creates a new scope for any bindings it introduces. References to these bindings are tagged with their defining scope, preventing accidental capture of user variables.

The meta-compiler supports compile-time code execution for macro expansion. Macros are themselves evaluated expressions that produce syntax tree values. The framework ensures that macro expansion happens in a controlled environment where only pure computations and explicitly allowed effects are permitted.

## Implementation Technologies

The builder API uses Rust's type system to guide rule construction. Methods are designed to chain naturally, with each method returning a builder type that exposes appropriate next steps. This makes invalid rule definitions un-representable at the type level.

Error handling uses Result types throughout, with a custom Diagnostics type that accumulates multiple errors. This type implements standard Result combinators, allowing queries to compose naturally while still collecting all diagnostics.

## Integration with Existing Compiler

The meta-compiler generates code that integrates with Cadenza's existing infrastructure. The syntax tree comes from the `cadenza-syntax` crate's rowan-based parser.

Each generated query becomes a method on the compiler database. User code constructs a database instance, populates it with parsed source files, and calls query methods to drive compilation. The database handles all the complexity of caching and dependency tracking.

The generated queries work with the existing type system defined in the compiler design document. Types, dimensions, traits, effects, and contracts are all first-class values that queries can compute and manipulate.

Diagnostics integrate with the existing error reporting infrastructure. The generated code produces structured diagnostic objects that can be formatted for display in the terminal or IDE. Source locations reference the original syntax nodes, enabling precise error underlining.

## Builder API Design

The builder API provides a fluent interface for constructing semantic definitions. Rather than manually building nested data structures, compiler writers chain method calls that read naturally as the semantic rules they express.

A query definition starts with a call to `query()` providing the query name. Methods like `input()` and `output()` specify types. The `rule()` method adds a rule to the query.

Patterns are constructed with functions like `apply()`, `symbol()`, and `integer()`. These functions take other patterns as arguments, allowing complex patterns to be built compositionally. The `capture()` function marks subpatterns whose matched values should be bound to variables.

Expressions are built similarly, with functions like `call()`, `construct()`, and `let_in()`. The `var()` function references variables captured by patterns. Method chaining allows expressions to be nested naturally.

State threading uses builder methods that make the flow of state explicit. The `let_in()` function defines intermediate bindings that thread state through sequential computations. The `fold()` function threads state through iterations over collections.

Error handling builders like `try_let()` and `error()` integrate error recovery into rule definitions. These builders generate code that catches errors, records diagnostics, and continues processing where possible.

The key to the builder API's usability is its type safety. The Rust type system ensures that only valid rule structures can be built. For example, you can't call `body()` on a let-builder without first calling `bindings()`, because the type returned by `bindings()` is what enables the `body()` method.

## Example: Function Application

To illustrate how semantic definitions work, consider defining evaluation for function application. The pattern matches an application node with a callee and arguments. The expression evaluates the callee, evaluates the arguments, and applies the function value to the argument values.

State threading is explicit. Evaluating the callee returns both a value and an updated environment. This environment is used when evaluating the arguments. The function application itself might modify the environment if it performs side effects, so it returns a new environment along with the result.

Error handling is integrated. If evaluating the callee produces an error, the diagnostic is recorded but evaluation continues with an error value. If the callee value isn't actually a function, another diagnostic is recorded. If argument evaluation fails, those diagnostics are collected. All diagnostics are aggregated and returned together.

Source tracking is automatic. The result value is tagged with the application node as its source. If the evaluation is later found to produce a value of the wrong type, the error message can point to this specific application site.

The rule definition doesn't specify how pattern matching should be implemented or how state should be threaded through the evaluation. Those details are determined during code generation. The definition only states what should be computed.

## External Query Implementations

Some computations can't be expressed purely in terms of pattern matching. Calling WASM components requires a runtime. Invoking SMT solvers requires external processes. These operations are marked as external implementations in the query definition.

An external query specifies its signature but not its body. During code generation, the framework generates a trait method that user code must implement. This allows the semantics definition to reference functionality that will be provided at runtime.

External queries integrate seamlessly with generated queries. Dependencies are tracked the same way. Results are cached the same way. The only difference is that the implementation is hand-written rather than generated.

This mechanism allows the meta-compiler to be extended with domain-specific operations without modifying the code generator. Need to query a database during compilation? Define an external query for it. Need to call a language-specific linter? Define an external query. The framework handles the integration.

## Performance Characteristics

The generated code is designed for efficiency. Pattern matching is compiled into decision trees that minimize redundant checks. When multiple rules share a common pattern prefix, that prefix is matched once rather than repeatedly.

Caching means queries are computed only when necessary. If a query's inputs haven't changed since the last computation, the cached result is returned immediately. This makes incremental compilation practical.

State threading uses persistent data structures that make cloning cheap. When state is passed through multiple computations, the physical copying is minimized through structural sharing. Only the modified portions of the state are actually duplicated.

Error recovery adds minimal overhead. Diagnostics are collected incrementally rather than being thrown as exceptions. The control flow remains straightforward, without the complexity of exception handling.

The meta-compiler's performance comes from generating specialized code for each query. There's no generic interpretation of rules at runtime. Everything is compiled to Rust functions that the LLVM optimizer can inline and optimize further.

## Maintenance and Evolution

Because semantic definitions are data, they can be analyzed, tested, and evolved systematically. The build script can generate documentation from the definitions, showing which queries exist and what patterns they handle.

Testing is straightforward. Define input syntax, run queries, check results. Because queries are pure functions, tests don't need complex setup or teardown. Mock implementations of external queries can be provided for testing.

Evolution is supported through versioning. As the language evolves, semantic definitions can be updated without changing the meta-compiler itself. New queries can be added, new patterns can be matched, new computations can be expressed, all through modifications to the definition data structures.

The separation between definition and implementation means improvements to the code generator benefit all queries simultaneously. If a better pattern matching strategy is discovered, regenerating the code applies it everywhere.

## Relationship to Other Systems

The meta-compiler shares conceptual similarities with other systems but has distinct characteristics. It resembles attribute grammars in how it associates computations with syntax, but uses pattern matching rather than grammar-based dispatch.

It resembles term rewriting systems in how it matches patterns and produces results, but integrates state threading and error recovery more deeply. The `cadenza-rewrite` crate provided the initial exploration of pattern-based transformations.

It resembles query systems like Salsa in how it computes attributes on demand, but generates specialized code rather than using generic query machinery. Salsa may be used for the runtime infrastructure, but the meta-compiler generates the query implementations themselves.

It resembles compiler frameworks like Cranelift's ISLE in how it compiles rules to decision trees, but targets a full compiler rather than just instruction selection. The semantic definitions describe entire compilation phases, not just pattern matching.

The meta-compiler is specifically designed for Cadenza's needs: supporting all the compilation phases described in the compiler design document, integrating with the existing syntax and type system, and providing the flexibility needed for a language still under development.
