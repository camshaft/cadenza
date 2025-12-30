//! Core data types for the meta-compiler.
//!
//! These types define the structure of semantic definitions, including queries,
//! rules, patterns, and expressions.

/// A complete semantic definition consisting of multiple queries.
#[derive(Clone, Debug)]
pub struct Semantics {
    /// All queries defined in this semantic definition
    pub queries: Vec<Query>,
}

impl Semantics {
    /// Create a new empty semantic definition
    pub fn new() -> Self {
        Self {
            queries: Vec::new(),
        }
    }

    /// Add a query to this semantic definition
    pub fn add_query(mut self, query: Query) -> Self {
        self.queries.push(query);
        self
    }
}

impl Default for Semantics {
    fn default() -> Self {
        Self::new()
    }
}

/// A query defines how to compute an attribute for syntax nodes.
#[derive(Clone, Debug)]
pub struct Query {
    /// The name of this query
    pub name: String,

    /// The input type for this query
    pub input: Type,

    /// The output type for this query
    pub output: Type,

    /// The rules that define this query's behavior
    pub rules: Vec<Rule>,

    /// Whether this query is implemented externally (in Rust)
    pub external: bool,
}

/// A rule matches patterns and computes results.
#[derive(Clone, Debug)]
pub struct Rule {
    /// The pattern that must match for this rule to apply
    pub pattern: Pattern,

    /// Optional guard constraint that must be satisfied
    pub guard: Option<Guard>,

    /// The expression to evaluate when the pattern matches
    pub result: Expr,
}

/// A pattern matches syntax structures.
#[derive(Clone, Debug)]
pub enum Pattern {
    /// Wildcard pattern that matches anything
    Wildcard,

    /// Capture a matched value into a variable
    Capture(String),

    /// Capture all remaining items (for variadic matching)
    CaptureAll(String),

    /// Match any syntax node
    Any,

    /// Match a specific integer value or bind the integer to a variable
    Integer(Box<Pattern>),

    /// Match a specific float value or bind the float to a variable
    Float(Box<Pattern>),

    /// Match a specific string value or bind the string to a variable
    String(Box<Pattern>),

    /// Match a specific boolean value or bind the boolean to a variable
    Bool(Box<Pattern>),

    /// Match a symbol and optionally capture it
    Symbol(Box<Pattern>),

    /// Match a symbol with a specific name
    SymbolLit(String),

    /// Match a function application
    Apply {
        callee: Box<Pattern>,
        args: Vec<Pattern>,
    },

    /// Match a function value
    Function {
        params: Box<Pattern>,
        body: Box<Pattern>,
    },

    /// Match a function type
    FunctionType {
        params: Box<Pattern>,
        ret: Box<Pattern>,
    },

    /// Match a tuple pattern
    Tuple(Vec<Pattern>),

    /// Match a structural record (anonymous with field patterns)
    Record { fields: Vec<(String, Pattern)> },

    /// Match a nominal record/struct (with name and field patterns)
    Struct {
        name: String,
        fields: Vec<(String, Pattern)>,
    },

    /// Match a structural enum variant (anonymous union)
    EnumVariant {
        variant: String,
        inner: Option<Box<Pattern>>,
    },

    /// Match a nominal enum (with type name and variant)
    Enum {
        name: String,
        variant: String,
        inner: Option<Box<Pattern>>,
    },

    /// Match a specific value (used with Value enum)
    Value(Value),

    /// Match if the expression is a function definition
    FunctionDef,
}

/// An expression computes a result using captured variables.
#[derive(Clone, Debug)]
pub enum Expr {
    /// Reference a captured variable
    Var(String),

    /// Reference to the current node being matched
    CurrentNode,

    /// A constant value
    Const(Value),

    /// Call another query
    Call { query: String, args: Vec<Expr> },

    /// Call a method on a receiver
    MethodCall {
        method: String,
        receiver: Option<Box<Expr>>,
        args: Vec<Expr>,
    },

    /// Construct a value
    Construct {
        constructor: String,
        fields: Vec<Expr>,
    },

    /// Let bindings
    Let {
        bindings: Vec<(String, Expr)>,
        body: Box<Expr>,
    },

    /// Try let bindings with error handling
    TryLet {
        bindings: Vec<(String, Expr)>,
        body: Box<Expr>,
        recovery: Option<Box<Expr>>,
    },

    /// Sequence of expressions
    Do(Vec<Expr>),

    /// Field access by index
    Field(Box<Expr>, usize),

    /// Field access by name
    FieldName(Box<Expr>, String),

    /// Array literal
    Array(Vec<Expr>),

    /// Zip multiple iterables together
    Zip(Vec<Expr>),

    /// For-each loop
    ForEach {
        var: String,
        iter: Box<Expr>,
        body: Box<Expr>,
    },

    /// Fold operation
    Fold {
        iter: Box<Expr>,
        init: Box<Expr>,
        acc: String,
        item: String,
        body: Box<Expr>,
    },

    /// Filter operation
    Filter { iter: Box<Expr>, pred: Box<Expr> },

    /// Map operation
    Map {
        iter: Box<Expr>,
        var: String,
        body: Box<Expr>,
    },

    /// Find operation
    Find {
        iter: Box<Expr>,
        var: String,
        pred: Box<Guard>,
    },

    /// Collect into hash map
    CollectHashMap(Box<Expr>),

    /// Match expression
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
    },

    /// If expression
    If {
        condition: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Box<Expr>,
    },

    /// Tuple construction
    TupleExpr(Vec<Expr>),

    /// Record construction (structural)
    RecordExpr { fields: Vec<(String, Expr)> },

    /// Struct construction (nominal)
    StructExpr {
        name: String,
        fields: Vec<(String, Expr)>,
    },

    /// Enum variant construction (structural)
    EnumVariantExpr {
        variant: String,
        inner: Option<Box<Expr>>,
    },

    /// Enum construction (nominal)
    EnumExpr {
        name: String,
        variant: String,
        inner: Option<Box<Expr>>,
    },

    /// Spanned value (value with source location)
    Spanned { value: Box<Expr>, source: Box<Expr> },

    /// Ok result
    Ok(Box<Expr>),

    /// Error with diagnostic
    ErrorAndReturn {
        diagnostic: Diagnostic,
        fallback: Box<Expr>,
    },

    /// Scope define operation
    ScopeDefine {
        scope: Box<Expr>,
        name: Box<Expr>,
        value: Box<Expr>,
    },

    /// Check if expression matches pattern
    Is { expr: Box<Expr>, pattern: Pattern },

    /// Marker for function definition check
    IsFunctionDef,
}

/// A guard imposes additional constraints on pattern matching.
#[derive(Clone, Debug)]
pub enum Guard {
    /// Match the expression against a pattern
    Match { expr: Expr, pattern: Pattern },

    /// Call a guard function
    Call { func: String, args: Vec<Expr> },

    /// Equality check
    Eq(Expr, Expr),

    /// Logical AND of multiple guards
    And(Vec<Guard>),
}

/// A match arm in a match expression.
#[derive(Clone, Debug)]
pub struct MatchArm {
    /// The pattern to match
    pub pattern: Pattern,

    /// Optional guard condition
    pub guard: Option<Guard>,

    /// The body to execute
    pub body: Expr,
}

/// Type descriptors for query signatures.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Type {
    /// Node identifier in the syntax graph
    NodeId,

    /// Value type
    Value,

    /// Type type
    Type,

    /// Symbol type
    Symbol,

    /// String type
    String,

    /// Bool type
    Bool,

    /// Environment identifier
    EnvId,

    /// Effect context type
    EffectCtx,

    /// Memory state type
    MemState,

    /// Diagnostics collection
    Diagnostics,

    /// Trait implementation type
    TraitImpl,

    /// Pattern type
    PatternType,

    /// Constraint set type
    ConstraintSet,

    /// Substitution type
    Substitution,

    /// Function identifier
    FunctionId,

    /// Proof result type
    ProofResult,

    /// Context type
    Context,

    /// Contract type
    Contract,

    // ===== Language Type System =====
    /// Integer type with optional bit width
    Integer { signed: bool, bits: Option<u16> },

    /// Floating point type
    Float,

    /// Rational number type
    Rational { signed: bool, bits: Option<u16> },

    /// Character type
    Char,

    /// Unit type (empty tuple)
    Unit,

    /// List type (dynamically sized, homogeneous)
    List(Box<Type>),

    /// Tuple type (anonymous, fixed size, heterogeneous)
    Tuple(Vec<Type>),

    /// Named tuple type (nominal, fixed size, heterogeneous)
    NamedTuple { name: String, fields: Vec<Type> },

    /// Structural record (anonymous with named fields)
    Record { fields: Vec<(String, Type)> },

    /// Nominal struct (named with typed fields)
    Struct {
        name: String,
        fields: Vec<(String, Type)>,
    },

    /// Structural enum (anonymous union)
    EnumType {
        variants: Vec<(String, Option<Type>)>,
    },

    /// Nominal enum (named union)
    NamedEnum {
        name: String,
        variants: Vec<(String, Option<Type>)>,
    },

    /// Function type
    Function {
        params: Vec<Type>,
        ret: Box<Type>,
        lifetimes: Vec<String>,
    },

    /// Reference type
    Ref {
        target: Box<Type>,
        lifetime: String,
        mutable: bool,
    },

    /// Type variable for inference
    Var(String),

    /// Universally quantified type (polymorphic)
    Forall {
        type_vars: Vec<String>,
        body: Box<Type>,
    },

    /// Type with contract predicates
    Refined {
        base: Box<Type>,
        predicates: Vec<String>,
    },

    /// Type with physical dimension
    Dimensional {
        base: Box<Type>,
        dimension: Dimension,
    },

    /// Type with trait constraints
    Constrained {
        base: Box<Type>,
        traits: Vec<String>,
    },

    /// Type with effect requirements
    Effectful {
        base: Box<Type>,
        effects: Vec<String>,
    },

    // ===== Generic Container Types =====
    /// Option type
    Option(Box<Type>),

    /// Result type
    Result(Box<Type>, Box<Type>),

    /// HashMap type
    HashMap(Box<Type>, Box<Type>),

    /// Array/Vec type
    Array(Box<Type>),

    /// Spanned type (value with source info)
    Spanned(Box<Type>),

    /// Unknown type placeholder
    Unknown,

    /// Error type placeholder
    Error,
}

/// Physical dimension for dimensional analysis
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Dimension {
    /// Base dimension components with their exponents
    /// E.g., velocity = meter^1 * second^-1
    pub components: Vec<(String, i16)>,
}

impl Dimension {
    /// Create a dimensionless quantity
    pub fn dimensionless() -> Self {
        Self {
            components: Vec::new(),
        }
    }

    /// Create a base dimension
    pub fn base(name: impl Into<String>) -> Self {
        Self {
            components: vec![(name.into(), 1)],
        }
    }
}

/// Runtime values that can appear in patterns and constants.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Value {
    /// Integer value
    Integer(i128),

    /// Float value (represented as bits for Eq/Hash)
    Float(u64),

    /// String value
    String(String),

    /// Boolean value
    Bool(bool),

    /// Type value
    Type(Type),

    /// Symbol value
    Symbol(String),

    /// TypeOf operation
    TypeOf,

    /// Error value
    Error,
}

/// Diagnostic information for errors and warnings.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    /// The kind of diagnostic (e.g., "type_error", "runtime_error")
    pub kind: String,

    /// The diagnostic message expression
    pub message: Box<Expr>,

    /// Primary source location expression
    pub primary: Box<Expr>,

    /// Secondary locations with labels
    pub secondary: Vec<(Expr, String)>,

    /// Additional notes
    pub notes: Vec<String>,
}
