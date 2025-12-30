//! Builder API for constructing semantic definitions.
//!
//! This module provides a fluent interface for building queries, rules, patterns,
//! and expressions. The builders use Rust's type system to guide construction and
//! prevent invalid structures.

use crate::types::*;

// ===== Query Builders =====

/// Start building a query with the given name
pub fn query(name: impl Into<String>) -> QueryBuilder {
    QueryBuilder {
        name: name.into(),
        input: None,
        output: None,
        rules: Vec::new(),
        external: false,
    }
}

/// Builder for constructing queries
pub struct QueryBuilder {
    name: String,
    input: Option<Type>,
    output: Option<Type>,
    rules: Vec<Rule>,
    external: bool,
}

impl QueryBuilder {
    /// Set the input type for this query
    pub fn input(mut self, ty: Type) -> Self {
        self.input = Some(ty);
        self
    }

    /// Set the output type for this query
    pub fn output(mut self, ty: Type) -> Self {
        self.output = Some(ty);
        self
    }

    /// Add a rule to this query
    pub fn rule(mut self, rule: Rule) -> Self {
        self.rules.push(rule);
        self
    }

    /// Mark this query as externally implemented
    pub fn extern_impl(mut self) -> Self {
        self.external = true;
        self
    }

    /// Build the query (consuming the builder)
    pub fn build(self) -> Query {
        Query {
            name: self.name,
            input: self.input.expect("Query must have input type"),
            output: self.output.expect("Query must have output type"),
            rules: self.rules,
            external: self.external,
        }
    }
}

// ===== Pattern Builders =====

/// Wildcard pattern that matches anything
pub fn wildcard() -> Pattern {
    Pattern::Wildcard
}

/// Capture a matched value into a variable
pub fn capture(name: impl Into<String>) -> Pattern {
    Pattern::Capture(name.into())
}

/// Capture all remaining items
pub fn capture_all(name: impl Into<String>) -> Pattern {
    Pattern::CaptureAll(name.into())
}

/// Match any syntax node
pub fn any() -> Pattern {
    Pattern::Any
}

/// Match an integer (with an inner pattern)
pub fn integer(inner: Pattern) -> Pattern {
    Pattern::Integer(Box::new(inner))
}

/// Match a float (with an inner pattern)
pub fn float(inner: Pattern) -> Pattern {
    Pattern::Float(Box::new(inner))
}

/// Match a string literal (with an inner pattern)
pub fn string_lit(inner: Pattern) -> Pattern {
    Pattern::String(Box::new(inner))
}

/// Match a boolean (with an inner pattern)
pub fn bool_pattern(inner: Pattern) -> Pattern {
    Pattern::Bool(Box::new(inner))
}

/// Match a symbol (with an inner pattern)
pub fn symbol(name: impl Into<PatternOrString>) -> Pattern {
    match name.into() {
        PatternOrString::Pattern(p) => Pattern::Symbol(Box::new(p)),
        PatternOrString::String(s) => Pattern::SymbolLit(s),
    }
}

/// Match a function application and start building a rule
pub fn apply(callee: Pattern, args: impl Into<Args>) -> RuleBuilder {
    RuleBuilder {
        pattern: Pattern::Apply {
            callee: Box::new(callee),
            args: args.into().0,
        },
        guard: None,
        result: None,
    }
}

/// Match a function value
pub fn value_fn(params: Pattern, body: Pattern) -> Pattern {
    Pattern::Function {
        params: Box::new(params),
        body: Box::new(body),
    }
}

/// Match a function type
pub fn function_ty(params: Pattern, ret: Pattern) -> Pattern {
    Pattern::FunctionType {
        params: Box::new(params),
        ret: Box::new(ret),
    }
}

/// Match a tuple
pub fn tuple_pattern(patterns: impl Into<Vec<Pattern>>) -> Pattern {
    Pattern::Tuple(patterns.into())
}

/// Match a specific value
pub fn value(v: Value) -> Pattern {
    Pattern::Value(v)
}

/// Match a function definition
pub fn is_function_def() -> Pattern {
    Pattern::FunctionDef
}

/// Match a structural record
pub fn record(fields: impl Into<Vec<(String, Pattern)>>) -> Pattern {
    Pattern::Record {
        fields: fields.into(),
    }
}

pub fn field_pattern(name: impl Into<String>, inner: Pattern) -> (String, Pattern) {
    (name.into(), inner)
}

/// Match a nominal struct
pub fn struct_pattern(
    name: impl Into<String>,
    fields: impl Into<Vec<(String, Pattern)>>,
) -> Pattern {
    Pattern::Struct {
        name: name.into(),
        fields: fields.into(),
    }
}

/// Match a structural enum variant
pub fn enum_variant(variant: impl Into<String>, inner: Option<Pattern>) -> Pattern {
    Pattern::EnumVariant {
        variant: variant.into(),
        inner: inner.map(Box::new),
    }
}

/// Match a nominal enum
pub fn enum_pattern(
    name: impl Into<String>,
    variant: impl Into<String>,
    inner: Option<Pattern>,
) -> Pattern {
    Pattern::Enum {
        name: name.into(),
        variant: variant.into(),
        inner: inner.map(Box::new),
    }
}

// ===== Rule Builders =====

/// Builder for constructing rules
pub struct RuleBuilder {
    pattern: Pattern,
    guard: Option<Guard>,
    result: Option<Expr>,
}

impl RuleBuilder {
    /// Add a guard condition to this rule
    pub fn when(mut self, guard: Guard) -> Self {
        self.guard = Some(guard);
        self
    }

    /// Set the result expression (consuming the builder)
    pub fn then(self, result: Expr) -> Rule {
        Rule {
            pattern: self.pattern,
            guard: self.guard,
            result,
        }
    }
}

/// Extension trait to allow patterns to directly specify results
impl Pattern {
    /// Set the result expression for this pattern
    pub fn then(self, result: Expr) -> Rule {
        Rule {
            pattern: self,
            guard: None,
            result,
        }
    }

    /// Add a guard and return a builder
    pub fn when(self, guard: Guard) -> RuleBuilder {
        RuleBuilder {
            pattern: self,
            guard: Some(guard),
            result: None,
        }
    }
}

// ===== Expression Builders =====

/// Reference a captured variable
pub fn var(name: impl Into<String>) -> Expr {
    Expr::Var(name.into())
}

/// Reference the current node
pub fn current_node() -> Expr {
    Expr::CurrentNode
}

/// A constant value
pub fn const_val(value: impl Into<Value>) -> Expr {
    Expr::Const(value.into())
}

/// String constant
pub fn string(s: impl Into<String>) -> Expr {
    Expr::Const(Value::String(s.into()))
}

/// Call another query
pub fn call(query: impl Into<String>, args: impl Into<Vec<Expr>>) -> Expr {
    Expr::Call {
        query: query.into(),
        args: args.into(),
    }
}

/// Construct a value
pub fn construct(constructor: impl Into<String>, fields: impl Into<Vec<Expr>>) -> Expr {
    Expr::Construct {
        constructor: constructor.into(),
        fields: fields.into(),
    }
}

/// Create an array literal
pub fn array(items: impl Into<Vec<Expr>>) -> Expr {
    Expr::Array(items.into())
}

/// Create a binding tuple for let expressions
pub fn binding(name: impl Into<String>, value: Expr) -> (String, Expr) {
    (name.into(), value)
}

/// Start building a let expression
pub fn let_in(bindings: impl Into<Vec<(String, Expr)>>) -> LetBuilder {
    LetBuilder {
        bindings: bindings.into(),
        body: None,
    }
}

/// Builder for let expressions
pub struct LetBuilder {
    bindings: Vec<(String, Expr)>,
    body: Option<Expr>,
}

impl LetBuilder {
    /// Set the body of the let expression
    pub fn body(self, body: Expr) -> Expr {
        Expr::Let {
            bindings: self.bindings,
            body: Box::new(body),
        }
    }
}

/// Start building a try-let expression
pub fn try_let(bindings: impl Into<Vec<(String, Expr)>>) -> TryLetBuilder {
    TryLetBuilder {
        bindings: bindings.into(),
        body: None,
    }
}

/// Builder for try-let expressions
pub struct TryLetBuilder {
    bindings: Vec<(String, Expr)>,
    body: Option<Expr>,
}

impl TryLetBuilder {
    /// Set the body of the try-let expression
    pub fn body(self, body: Expr) -> TryExpr {
        TryExpr {
            bindings: self.bindings,
            body,
            recovery: None,
        }
    }
}

/// Intermediate builder for try expressions with recovery
pub struct TryExpr {
    bindings: Vec<(String, Expr)>,
    body: Expr,
    recovery: Option<Expr>,
}

impl TryExpr {
    /// Set the recovery expression
    pub fn or_else(mut self, recovery: Expr) -> Self {
        self.recovery = Some(recovery);
        self
    }

    /// Build the try-let with error recovery
    pub fn recover_from_errors(self) -> Expr {
        Expr::TryLet {
            bindings: self.bindings,
            body: Box::new(self.body),
            recovery: self.recovery.map(Box::new),
        }
    }
}

/// Create a do sequence
pub fn do_seq(exprs: impl Into<Vec<Expr>>) -> Expr {
    Expr::Do(exprs.into())
}

/// Create an if expression
pub fn if_expr(condition: Expr, then_branch: Expr, else_branch: Expr) -> Expr {
    Expr::If {
        condition: Box::new(condition),
        then_branch: Box::new(then_branch),
        else_branch: Box::new(else_branch),
    }
}

/// Create a match expression
pub fn match_expr(scrutinee: Expr, arms: Vec<MatchArm>) -> Expr {
    Expr::Match {
        scrutinee: Box::new(scrutinee),
        arms,
    }
}

/// Create a match arm
pub fn match_arm(pattern: Pattern, body: Expr) -> MatchArm {
    MatchArm {
        pattern,
        guard: None,
        body,
    }
}

/// Zip multiple expressions
pub fn zip(exprs: impl Into<Vec<Expr>>) -> Expr {
    Expr::Zip(exprs.into())
}

/// Create a fold expression
pub fn fold<F>(iter: Expr, init: Expr, f: F) -> Expr
where
    F: FnOnce(&str, &str) -> Expr,
{
    let acc = "__fold_acc";
    let item = "__fold_item";
    Expr::Fold {
        iter: Box::new(iter),
        init: Box::new(init),
        acc: acc.to_string(),
        item: item.to_string(),
        body: Box::new(f(acc, item)),
    }
}

/// Create a structural record
pub fn record_expr(fields: impl Into<Vec<(String, Expr)>>) -> Expr {
    Expr::RecordExpr {
        fields: fields.into(),
    }
}

pub fn field_expr(name: impl Into<String>, value: Expr) -> (String, Expr) {
    (name.into(), value)
}

/// Create a nominal struct
pub fn struct_expr(name: impl Into<String>, fields: impl Into<Vec<(String, Expr)>>) -> Expr {
    Expr::StructExpr {
        name: name.into(),
        fields: fields.into(),
    }
}

/// Create a structural enum variant
pub fn enum_variant_expr(variant: impl Into<String>, inner: Option<Expr>) -> Expr {
    Expr::EnumVariantExpr {
        variant: variant.into(),
        inner: inner.map(Box::new),
    }
}

/// Create a nominal enum
pub fn enum_expr(name: impl Into<String>, variant: impl Into<String>, inner: Option<Expr>) -> Expr {
    Expr::EnumExpr {
        name: name.into(),
        variant: variant.into(),
        inner: inner.map(Box::new),
    }
}

/// Create a spanned value
pub fn spanned(value: Expr, source: Expr) -> Expr {
    Expr::Spanned {
        value: Box::new(value),
        source: Box::new(source),
    }
}

/// Create an Ok result
pub fn ok(value: Expr) -> Expr {
    Expr::Ok(Box::new(value))
}

/// Start building an error
pub fn error(kind: impl Into<String>, message: impl Into<Expr>, location: Expr) -> ErrorBuilder {
    ErrorBuilder {
        kind: kind.into(),
        message: Box::new(message.into()),
        primary: Box::new(location),
        secondary: Vec::new(),
        notes: Vec::new(),
    }
}

/// Builder for error diagnostics
pub struct ErrorBuilder {
    kind: String,
    message: Box<Expr>,
    primary: Box<Expr>,
    secondary: Vec<(Expr, String)>,
    notes: Vec<String>,
}

impl ErrorBuilder {
    /// Add a secondary location with label
    pub fn with_secondary_location(mut self, node: Expr, label: impl Into<String>) -> Self {
        self.secondary.push((node, label.into()));
        self
    }

    /// Add a note
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// Build the error with a fallback expression
    pub fn and_return(self, fallback: Expr) -> Expr {
        Expr::ErrorAndReturn {
            diagnostic: Diagnostic {
                kind: self.kind,
                message: self.message,
                primary: self.primary,
                secondary: self.secondary,
                notes: self.notes,
            },
            fallback: Box::new(fallback),
        }
    }
}

// ===== Type Builders =====

/// Node ID type
pub fn node_id() -> Type {
    Type::NodeId
}

/// Value type
pub fn value_type() -> Type {
    Type::Value
}

/// Type type
pub fn type_type() -> Type {
    Type::Type
}

/// Symbol type
pub fn symbol_type() -> Type {
    Type::Symbol
}

/// String type
pub fn string_type() -> Type {
    Type::String
}

/// Environment ID type
pub fn env_id() -> Type {
    Type::EnvId
}

/// Effect context type
pub fn effect_ctx() -> Type {
    Type::EffectCtx
}

/// Memory state type
pub fn mem_state() -> Type {
    Type::MemState
}

/// Diagnostics type
pub fn diagnostics_type() -> Type {
    Type::Diagnostics
}

/// Option type
pub fn option(inner: Type) -> Type {
    Type::Option(Box::new(inner))
}

/// Result type
pub fn result(ok: Type, err: Type) -> Type {
    Type::Result(Box::new(ok), Box::new(err))
}

/// HashMap type
pub fn hash_map(key: Type, value: Type) -> Type {
    Type::HashMap(Box::new(key), Box::new(value))
}

/// Array type
pub fn array_type(elem: Type) -> Type {
    Type::Array(Box::new(elem))
}

/// Tuple type
pub fn tuple_type(types: impl Into<Vec<Type>>) -> Type {
    Type::Tuple(types.into())
}

/// Spanned type
pub fn spanned_type(inner: Type) -> Type {
    Type::Spanned(Box::new(inner))
}

// ===== Language Type Builders =====

/// Integer type
pub fn integer_type(signed: bool, bits: Option<u16>) -> Type {
    Type::Integer { signed, bits }
}

/// Float type
pub fn float_type() -> Type {
    Type::Float
}

/// Rational type
pub fn rational_type(signed: bool, bits: Option<u16>) -> Type {
    Type::Rational { signed, bits }
}

/// Char type
pub fn char_type() -> Type {
    Type::Char
}

/// Unit type
pub fn unit_type() -> Type {
    Type::Unit
}

/// List type
pub fn list_type(elem: Type) -> Type {
    Type::List(Box::new(elem))
}

/// Named tuple type
pub fn named_tuple_type(name: impl Into<String>, fields: impl Into<Vec<Type>>) -> Type {
    Type::NamedTuple {
        name: name.into(),
        fields: fields.into(),
    }
}

/// Structural record type
pub fn record_type(fields: impl Into<Vec<(String, Type)>>) -> Type {
    Type::Record {
        fields: fields.into(),
    }
}

/// Nominal struct type
pub fn struct_type(name: impl Into<String>, fields: impl Into<Vec<(String, Type)>>) -> Type {
    Type::Struct {
        name: name.into(),
        fields: fields.into(),
    }
}

/// Structural enum type
pub fn enum_type(variants: impl Into<Vec<(String, Option<Type>)>>) -> Type {
    Type::EnumType {
        variants: variants.into(),
    }
}

/// Nominal enum type
pub fn named_enum_type(
    name: impl Into<String>,
    variants: impl Into<Vec<(String, Option<Type>)>>,
) -> Type {
    Type::NamedEnum {
        name: name.into(),
        variants: variants.into(),
    }
}

/// Function type
pub fn function_type(
    params: impl Into<Vec<Type>>,
    ret: Type,
    lifetimes: impl Into<Vec<String>>,
) -> Type {
    Type::Function {
        params: params.into(),
        ret: Box::new(ret),
        lifetimes: lifetimes.into(),
    }
}

/// Reference type
pub fn ref_type(target: Type, lifetime: impl Into<String>, mutable: bool) -> Type {
    Type::Ref {
        target: Box::new(target),
        lifetime: lifetime.into(),
        mutable,
    }
}

/// Type variable
pub fn type_var(name: impl Into<String>) -> Type {
    Type::Var(name.into())
}

/// Forall type
pub fn forall_type(type_vars: impl Into<Vec<String>>, body: Type) -> Type {
    Type::Forall {
        type_vars: type_vars.into(),
        body: Box::new(body),
    }
}

/// Refined type with predicates
pub fn refined_type(base: Type, predicates: impl Into<Vec<String>>) -> Type {
    Type::Refined {
        base: Box::new(base),
        predicates: predicates.into(),
    }
}

/// Dimensional type
pub fn dimensional_type(base: Type, dimension: Dimension) -> Type {
    Type::Dimensional {
        base: Box::new(base),
        dimension,
    }
}

/// Constrained type with traits
pub fn constrained_type(base: Type, traits: impl Into<Vec<String>>) -> Type {
    Type::Constrained {
        base: Box::new(base),
        traits: traits.into(),
    }
}

/// Effectful type
pub fn effectful_type(base: Type, effects: impl Into<Vec<String>>) -> Type {
    Type::Effectful {
        base: Box::new(base),
        effects: effects.into(),
    }
}

/// Unknown type
pub fn unknown_type() -> Type {
    Type::Unknown
}

/// Error type
pub fn error_type() -> Type {
    Type::Error
}

// ===== Guard Builders =====

/// Logical AND of guards
pub fn and(guards: impl Into<Vec<Guard>>) -> Guard {
    Guard::And(guards.into())
}

// ===== Helper Types =====

/// Helper to accept either a pattern or a string
pub enum PatternOrString {
    Pattern(Pattern),
    String(String),
}

impl From<Pattern> for PatternOrString {
    fn from(p: Pattern) -> Self {
        PatternOrString::Pattern(p)
    }
}

impl From<String> for PatternOrString {
    fn from(s: String) -> Self {
        PatternOrString::String(s)
    }
}

impl From<&str> for PatternOrString {
    fn from(s: &str) -> Self {
        PatternOrString::String(s.to_string())
    }
}

/// Helper for converting argument lists
pub struct Args(pub Vec<Pattern>);

impl From<Vec<Pattern>> for Args {
    fn from(v: Vec<Pattern>) -> Self {
        Args(v)
    }
}

impl<const N: usize> From<[Pattern; N]> for Args {
    fn from(arr: [Pattern; N]) -> Self {
        Args(arr.to_vec())
    }
}

impl From<Pattern> for Args {
    fn from(p: Pattern) -> Self {
        Args(vec![p])
    }
}

/// Helper for converting to Value
impl From<i128> for Value {
    fn from(i: i128) -> Self {
        Value::Integer(i)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::String(s)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::String(s.to_string())
    }
}

impl From<Type> for Value {
    fn from(t: Type) -> Self {
        Value::Type(t)
    }
}

/// Helper for converting strings to Expr (for message building)
impl From<String> for Expr {
    fn from(s: String) -> Self {
        Expr::Const(Value::String(s))
    }
}

impl From<&str> for Expr {
    fn from(s: &str) -> Self {
        Expr::Const(Value::String(s.to_string()))
    }
}
