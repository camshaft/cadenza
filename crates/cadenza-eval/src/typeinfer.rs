//! Hindley-Milner type inference for Cadenza.
//!
//! This module implements Algorithm W (Damas-Milner) for type inference with extensions for:
//! - Units and dimensional analysis
//! - Records with structural typing
//! - Union types
//!
//! The type inference algorithm operates in three phases:
//! 1. Constraint Generation: Walk the AST and generate type equations
//! 2. Constraint Solving: Use unification to solve the equations
//! 3. Generalization: Introduce quantifiers at let-bindings for polymorphism
//!
//! # Overview
//!
//! The type inference system is based on Algorithm W (Damas-Milner) and provides:
//!
//! - **Lazy type checking**: Type inference is on-demand, not automatic during evaluation
//! - **Polymorphism**: Support for parametric polymorphism with quantified types
//! - **Metaprogramming**: Macros can query expression types for code generation
//! - **LSP integration**: Designed for responsive IDE features (hover, diagnostics)
//!
//! # Architecture
//!
//! ## Core Components
//!
//! ### [`InferType`] - Types During Inference
//!
//! `InferType` is separate from the runtime [`Type`] enum to keep inference-specific details
//! (type variables, quantifiers) isolated from the runtime representation.
//!
//! ### [`TypeVar`] - Type Variables
//!
//! Type variables represent unknown types during inference. They are placeholders that get
//! unified with concrete types during the inference process.
//!
//! ### [`Substitution`] - Type Variable Mappings
//!
//! A substitution is a mapping from type variables to types. Substitutions are applied to types
//! to replace variables with their inferred types.
//!
//! ### [`TypeEnv`] - Type Environment
//!
//! The type environment tracks the types of variables in scope.
//!
//! ### [`TypeInferencer`] - The Inference Engine
//!
//! The main interface for type inference. Access it through the compiler:
//!
//! ```ignore
//! let mut inferencer = compiler.type_inferencer_mut();
//! let inferred_type = inferencer.infer_expr(&expr, &env)?;
//! ```
//!
//! # Algorithm
//!
//! ## 1. Constraint Generation
//!
//! When inferring the type of an expression, we generate constraints between types.
//! For example, `fn x -> x + 1` generates:
//! - `typeof(x) = α` (fresh type variable)
//! - `typeof(+) = (Integer, Integer) -> Integer`
//! - `α = Integer` (from the constraint that x must be Integer for +)
//! - `typeof(f) = α -> Integer`
//!
//! ## 2. Unification
//!
//! Unification finds a substitution that makes two types equal:
//! - `unify(α, Integer) = { α ↦ Integer }`
//! - `unify(α -> β, Integer -> String) = { α ↦ Integer, β ↦ String }`
//!
//! The unification algorithm includes an **occurs check** to prevent infinite types.
//!
//! ## 3. Generalization
//!
//! Generalization introduces quantifiers at let-bindings for polymorphism.
//! For example, `let id = fn x -> x` gets type `∀α. α -> α`, which can be used with
//! different concrete types (Integer, String, etc.).
//!
//! Variables that are free in the type but not in the environment are quantified.
//!
//! ## 4. Instantiation
//!
//! Instantiation replaces quantified variables with fresh variables:
//! `∀α. α -> α  ===instantiate===>  β -> β  (where β is fresh)`
//!
//! This allows polymorphic functions to be used with different types.
//!
//! # Usage
//!
//! ## Basic Type Inference
//!
//! ```ignore
//! use cadenza_eval::{Compiler, typeinfer::TypeEnv};
//!
//! let mut compiler = Compiler::new();
//! let mut env = TypeEnv::new();
//!
//! // Parse an expression
//! let parsed = parse("42");
//! let root = parsed.ast();
//! let expr = &root.items().collect::<Vec<_>>()[0];
//!
//! // Infer its type
//! let inferred = compiler.type_inferencer_mut().infer_expr(expr, &env)?;
//! println!("Type: {}", inferred);  // Type: integer
//! ```
//!
//! ## With Environment
//!
//! ```ignore
//! // Add a variable to the environment
//! let x: InternedString = "x".into();
//! env.insert(x, InferType::Concrete(Type::Integer));
//!
//! // Parse an expression using that variable
//! let parsed = parse("x + 1");
//! let expr = &root.items().collect::<Vec<_>>()[0];
//!
//! // Infer its type
//! let inferred = compiler.type_inferencer_mut().infer_expr(expr, &env)?;
//! println!("Type: {}", inferred);  // Type: integer
//! ```
//!
//! ## Polymorphic Functions
//!
//! ```ignore
//! // Create a polymorphic identity function: ∀α. α -> α
//! let type_var = inferencer.fresh_var();
//! let id_type = InferType::Forall(
//!     vec![type_var],
//!     Box::new(InferType::Fn(
//!         vec![InferType::Var(type_var)],
//!         Box::new(InferType::Var(type_var)),
//!     )),
//! );
//!
//! env.insert("id".into(), id_type);
//!
//! // Use it with different types
//! let parsed = parse("id 42");
//! let inferred = compiler.type_inferencer_mut().infer_expr(&expr, &env)?;
//! // Result: integer (type variable was unified with Integer)
//! ```
//!
//! ## In Macros
//!
//! Macros can access the type inferencer for metaprogramming:
//!
//! ```ignore
//! fn my_macro(args: &[Expr], ctx: &mut EvalContext) -> Result<Value> {
//!     let inferencer = ctx.compiler.type_inferencer_mut();
//!     
//!     // Build a type environment from current scope
//!     let mut env = TypeEnv::new();
//!     // ... populate from ctx.env ...
//!     
//!     // Infer type of first argument
//!     let arg_type = inferencer.infer_expr(&args[0], &env)?;
//!     
//!     // Generate code based on the type
//!     match arg_type {
//!         InferType::Concrete(Type::Integer) => {
//!             // Generate integer-specific code
//!         }
//!         InferType::Concrete(Type::String) => {
//!             // Generate string-specific code
//!         }
//!         _ => {
//!             // Handle other types
//!         }
//!     }
//! }
//! ```
//!
//! # Design Rationale
//!
//! ## Why Lazy Type Checking?
//!
//! Type checking is **not automatic** during evaluation for several reasons:
//!
//! 1. **Performance**: The evaluator can run at full speed without type checking overhead
//! 2. **LSP Responsiveness**: IDE features can be implemented without blocking
//! 3. **Incremental Compilation**: Only changed code needs to be re-type-checked
//! 4. **Cancellation**: Long-running type checks can be cancelled if the user makes changes
//!
//! ## Why Separate InferType from Type?
//!
//! Runtime [`Type`] is used for:
//! - Runtime type checking (e.g., comparing types of values)
//! - Displaying types to users
//! - Storing type information in values
//!
//! [`InferType`] is used for:
//! - Type inference with type variables
//! - Polymorphism with quantified types
//! - Constraint solving during inference
//!
//! Keeping them separate:
//! - Avoids polluting the runtime type system with inference-specific details
//! - Makes the type inference system easier to understand and maintain
//! - Allows the runtime type system to evolve independently
//!
//! ## Why Not Type Check Unevaluated Branches Automatically?
//!
//! The current implementation doesn't automatically track and type-check unevaluated branches
//! (e.g., in conditionals). This is a deliberate choice for Phase 1:
//!
//! 1. **Simplicity**: Easier to implement and understand
//! 2. **Performance**: No overhead for tracking unevaluated code
//! 3. **Flexibility**: Can be added later when needed
//!
//! Future work will add support for:
//! - Tracking unevaluated branches during evaluation
//! - Type-checking them in the background
//! - Reporting type errors even in unexecuted code
//!
//! # Future Work
//!
//! ## Type Annotations
//!
//! Add syntax for optional type annotations to provide better error messages,
//! enable earlier error detection, document code intent, and allow partial type inference.
//!
//! ## Dimensional Analysis Integration
//!
//! Integrate type inference with the unit system by extending [`InferType`] with dimension
//! information, adding dimension constraints to unification, and solving dimension equations
//! alongside type equations.
//!
//! ## Unevaluated Branch Tracking
//!
//! Track and type-check code paths not taken at evaluation time. Both branches of conditionals
//! should be type-checked even if only one is executed.
//!
//! ## Background Type Checking
//!
//! For LSP integration, implement background type checking with cancellation support,
//! prioritization, incremental updates, and caching.
//!
//! ## Effect System
//!
//! Extend type inference to track computational effects by extending [`InferType`] with
//! effect information, adding effect constraints to unification, and inferring effects
//! from operations.

use crate::{
    diagnostic::{Diagnostic, DiagnosticKind, Result},
    interner::InternedString,
    value::Type,
};
use cadenza_syntax::span::Span;
use rustc_hash::FxHashMap;
use std::fmt;

/// A type variable used during type inference.
///
/// Type variables are placeholders that get unified with concrete types
/// during the inference process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeVar(u32);

impl TypeVar {
    /// Creates a new type variable with the given ID.
    pub fn new(id: u32) -> Self {
        Self(id)
    }

    /// Gets the ID of this type variable.
    pub fn id(&self) -> u32 {
        self.0
    }
}

impl fmt::Display for TypeVar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "t{}", self.0)
    }
}

/// A type that may contain type variables during inference.
///
/// This is separate from the runtime `Type` enum to avoid polluting it with
/// inference-specific details.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum InferType {
    /// A concrete type (no type variables).
    Concrete(Type),
    /// A type variable to be inferred.
    Var(TypeVar),
    /// A function type with argument types and return type.
    Fn(Vec<InferType>, Box<InferType>),
    /// A list type with element type.
    List(Box<InferType>),
    /// A record type with field names and types.
    Record(Vec<(InternedString, InferType)>),
    /// A tuple type with element types.
    Tuple(Vec<InferType>),
    /// A union type (one of several alternatives).
    Union(Vec<InferType>),
    /// A polymorphic type with quantified variables.
    Forall(Vec<TypeVar>, Box<InferType>),
    /// A quantity type with a unit dimension.
    Quantity {
        /// The numeric value type (Integer or Float).
        value_type: Box<InferType>,
        /// The dimension (for now we'll store as a placeholder).
        /// TODO: Integrate with unit system
        dimension: InternedString,
    },
}

impl InferType {
    /// Converts this inferred type to a concrete runtime type.
    ///
    /// Returns an error if the type still contains unresolved type variables.
    pub fn to_concrete(&self) -> Result<Type> {
        match self {
            InferType::Concrete(t) => Ok(t.clone()),
            InferType::Var(v) => Err(Box::new(Diagnostic::new(
                DiagnosticKind::InternalError(format!("unresolved type variable {}", v)),
                Some(Span::new(0, 0)),
            ))),
            InferType::Fn(args, ret) => {
                let mut types = Vec::new();
                for arg in args {
                    types.push(arg.to_concrete()?);
                }
                types.push(ret.to_concrete()?);
                Ok(Type::Fn(types))
            }
            InferType::List(elem) => Ok(Type::List(Box::new(elem.to_concrete()?))),
            InferType::Record(fields) => {
                let mut concrete_fields = Vec::new();
                for (name, ty) in fields {
                    concrete_fields.push((*name, ty.to_concrete()?));
                }
                Ok(Type::Record(concrete_fields))
            }
            InferType::Tuple(elems) => {
                let mut concrete_elems = Vec::new();
                for elem in elems {
                    concrete_elems.push(elem.to_concrete()?);
                }
                Ok(Type::Tuple(concrete_elems))
            }
            InferType::Union(types) => {
                let mut concrete_types = Vec::new();
                for ty in types {
                    concrete_types.push(ty.to_concrete()?);
                }
                Ok(Type::Union(concrete_types))
            }
            InferType::Forall(_, ty) => {
                // When converting to concrete, we strip quantifiers
                // This is only valid if the type variables have been substituted
                ty.to_concrete()
            }
            InferType::Quantity {
                value_type,
                dimension: _,
            } => {
                // For now, just return the value type
                // TODO: Integrate with unit system properly
                value_type.to_concrete()
            }
        }
    }

    /// Creates an InferType from a concrete Type.
    pub fn from_concrete(ty: &Type) -> Self {
        match ty {
            Type::Nil => InferType::Concrete(Type::Nil),
            Type::Bool => InferType::Concrete(Type::Bool),
            Type::Symbol => InferType::Concrete(Type::Symbol),
            Type::Integer => InferType::Concrete(Type::Integer),
            Type::Float => InferType::Concrete(Type::Float),
            Type::String => InferType::Concrete(Type::String),
            Type::Type => InferType::Concrete(Type::Type),
            Type::Unknown => InferType::Concrete(Type::Unknown),
            Type::List(elem) => InferType::List(Box::new(InferType::from_concrete(elem))),
            Type::Fn(types) => {
                if types.is_empty() {
                    InferType::Fn(vec![], Box::new(InferType::Concrete(Type::Nil)))
                } else {
                    let (args, ret) = types.split_at(types.len() - 1);
                    InferType::Fn(
                        args.iter().map(InferType::from_concrete).collect(),
                        Box::new(InferType::from_concrete(&ret[0])),
                    )
                }
            }
            Type::Record(fields) => InferType::Record(
                fields
                    .iter()
                    .map(|(name, ty)| (*name, InferType::from_concrete(ty)))
                    .collect(),
            ),
            Type::Struct { name: _, fields } => {
                // For now, treat structs as nominal records
                // We could add a separate InferType::Struct variant later for better type checking
                // but for initial implementation, structs are similar to records with a name tag
                InferType::Record(
                    fields
                        .iter()
                        .map(|(field_name, ty)| (*field_name, InferType::from_concrete(ty)))
                        .collect(),
                )
            }
            Type::Tuple(elems) => {
                InferType::Tuple(elems.iter().map(InferType::from_concrete).collect())
            }
            Type::Enum(variants) => {
                // Enums are represented as unions of records
                InferType::Union(
                    variants
                        .iter()
                        .map(|(name, ty)| {
                            InferType::Record(vec![(*name, InferType::from_concrete(ty))])
                        })
                        .collect(),
                )
            }
            Type::Union(types) => {
                InferType::Union(types.iter().map(InferType::from_concrete).collect())
            }
        }
    }

    /// Returns the free type variables in this type.
    pub fn free_vars(&self) -> Vec<TypeVar> {
        let mut vars = Vec::new();
        self.collect_free_vars(&mut vars);
        vars.sort();
        vars.dedup();
        vars
    }

    fn collect_free_vars(&self, vars: &mut Vec<TypeVar>) {
        match self {
            InferType::Var(v) => vars.push(*v),
            InferType::Fn(args, ret) => {
                for arg in args {
                    arg.collect_free_vars(vars);
                }
                ret.collect_free_vars(vars);
            }
            InferType::List(elem) => elem.collect_free_vars(vars),
            InferType::Record(fields) => {
                for (_, ty) in fields {
                    ty.collect_free_vars(vars);
                }
            }
            InferType::Tuple(elems) => {
                for elem in elems {
                    elem.collect_free_vars(vars);
                }
            }
            InferType::Union(types) => {
                for ty in types {
                    ty.collect_free_vars(vars);
                }
            }
            InferType::Forall(bound, ty) => {
                let mut free = Vec::new();
                ty.collect_free_vars(&mut free);
                // Remove bound variables
                for v in free {
                    if !bound.contains(&v) {
                        vars.push(v);
                    }
                }
            }
            InferType::Quantity { value_type, .. } => {
                value_type.collect_free_vars(vars);
            }
            InferType::Concrete(_) => {}
        }
    }
}

impl fmt::Display for InferType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InferType::Concrete(t) => write!(f, "{}", t),
            InferType::Var(v) => write!(f, "{}", v),
            InferType::Fn(args, ret) => {
                write!(f, "fn(")?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", arg)?;
                }
                write!(f, ") -> {}", ret)
            }
            InferType::List(elem) => write!(f, "list[{}]", elem),
            InferType::Record(fields) => {
                write!(f, "{{")?;
                for (i, (name, ty)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", &**name, ty)?;
                }
                write!(f, "}}")
            }
            InferType::Tuple(elems) => {
                write!(f, "(")?;
                for (i, elem) in elems.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", elem)?;
                }
                write!(f, ")")
            }
            InferType::Union(types) => {
                for (i, ty) in types.iter().enumerate() {
                    if i > 0 {
                        write!(f, " | ")?;
                    }
                    write!(f, "{}", ty)?;
                }
                Ok(())
            }
            InferType::Forall(vars, ty) => {
                write!(f, "∀")?;
                for (i, v) in vars.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, ". {}", ty)
            }
            InferType::Quantity {
                value_type,
                dimension,
            } => {
                write!(f, "{}[{}]", value_type, &**dimension)
            }
        }
    }
}

/// A substitution mapping type variables to types.
#[derive(Debug, Clone, Default)]
pub struct Substitution {
    map: FxHashMap<TypeVar, InferType>,
}

impl Substitution {
    /// Creates an empty substitution.
    pub fn new() -> Self {
        Self {
            map: FxHashMap::default(),
        }
    }

    /// Creates a substitution with a single binding.
    pub fn singleton(var: TypeVar, ty: InferType) -> Self {
        let mut map = FxHashMap::default();
        map.insert(var, ty);
        Self { map }
    }

    /// Inserts a binding into this substitution.
    pub fn insert(&mut self, var: TypeVar, ty: InferType) {
        self.map.insert(var, ty);
    }

    /// Looks up a type variable in this substitution.
    pub fn get(&self, var: TypeVar) -> Option<&InferType> {
        self.map.get(&var)
    }

    /// Applies this substitution to a type.
    pub fn apply(&self, ty: &InferType) -> InferType {
        self.apply_impl(ty, &mut Vec::new())
    }

    fn apply_impl(&self, ty: &InferType, visiting: &mut Vec<TypeVar>) -> InferType {
        match ty {
            InferType::Var(v) => {
                // Prevent infinite recursion by tracking visited variables
                if visiting.contains(v) {
                    return ty.clone();
                }

                if let Some(t) = self.get(*v) {
                    visiting.push(*v);
                    let result = self.apply_impl(t, visiting);
                    visiting.pop();
                    result
                } else {
                    ty.clone()
                }
            }
            InferType::Fn(args, ret) => {
                let new_args = args
                    .iter()
                    .map(|arg| self.apply_impl(arg, visiting))
                    .collect();
                let new_ret = Box::new(self.apply_impl(ret, visiting));
                InferType::Fn(new_args, new_ret)
            }
            InferType::List(elem) => InferType::List(Box::new(self.apply_impl(elem, visiting))),
            InferType::Record(fields) => InferType::Record(
                fields
                    .iter()
                    .map(|(name, ty)| (*name, self.apply_impl(ty, visiting)))
                    .collect(),
            ),
            InferType::Tuple(elems) => InferType::Tuple(
                elems
                    .iter()
                    .map(|elem| self.apply_impl(elem, visiting))
                    .collect(),
            ),
            InferType::Union(types) => InferType::Union(
                types
                    .iter()
                    .map(|ty| self.apply_impl(ty, visiting))
                    .collect(),
            ),
            InferType::Forall(vars, ty) => {
                // Don't apply substitutions to bound variables
                // Only apply to free variables in the body
                let mut filtered_subst = Substitution::new();
                for (var, subst_ty) in &self.map {
                    if !vars.contains(var) {
                        filtered_subst.insert(*var, subst_ty.clone());
                    }
                }
                InferType::Forall(
                    vars.clone(),
                    Box::new(filtered_subst.apply_impl(ty, visiting)),
                )
            }
            InferType::Quantity {
                value_type,
                dimension,
            } => InferType::Quantity {
                value_type: Box::new(self.apply_impl(value_type, visiting)),
                dimension: *dimension,
            },
            InferType::Concrete(_) => ty.clone(),
        }
    }

    /// Composes two substitutions: applies `other` then `self`.
    pub fn compose(&self, other: &Substitution) -> Substitution {
        let mut result = Substitution::new();
        // Apply self to all bindings in other
        for (var, ty) in &other.map {
            result.insert(*var, self.apply(ty));
        }
        // Add all bindings from self that are not in other
        for (var, ty) in &self.map {
            if !result.map.contains_key(var) {
                result.insert(*var, ty.clone());
            }
        }
        result
    }
}

/// A type constraint equation between two types.
#[derive(Debug, Clone)]
pub struct Constraint {
    /// The left-hand side of the equation.
    pub lhs: InferType,
    /// The right-hand side of the equation.
    pub rhs: InferType,
    /// The source span that generated this constraint.
    pub span: Span,
}

impl Constraint {
    /// Creates a new constraint.
    pub fn new(lhs: InferType, rhs: InferType, span: Span) -> Self {
        Self { lhs, rhs, span }
    }
}

/// A type environment mapping variables to type schemes.
#[derive(Debug, Clone)]
pub struct TypeEnv {
    bindings: FxHashMap<InternedString, InferType>,
}

impl TypeEnv {
    /// Creates an empty type environment.
    pub fn new() -> Self {
        Self {
            bindings: FxHashMap::default(),
        }
    }

    /// Builds a type environment from a runtime environment and compiler.
    ///
    /// This converts all values in the runtime environment and compiler definitions
    /// to their inferred types, which is useful for type-checking expressions in macros
    /// that need to query types.
    ///
    /// # Example
    ///
    /// ```ignore
    /// fn my_macro(args: &[Expr], ctx: &mut EvalContext) -> Result<Value> {
    ///     // Build type environment from runtime environment
    ///     let type_env = TypeEnv::from_context(ctx.env, ctx.compiler);
    ///     
    ///     // Infer type of first argument
    ///     let arg_type = ctx.compiler.type_inferencer_mut()
    ///         .infer_expr(&args[0], &type_env)?;
    ///     
    ///     // Use the type information...
    /// }
    /// ```
    pub fn from_context(env: &crate::env::Env, compiler: &crate::compiler::Compiler) -> Self {
        let mut type_env = Self::new();
        // Add values from runtime environment
        for (name, value) in env.iter() {
            type_env.add_value(name, value);
        }
        // Add values from compiler definitions (functions, etc.)
        for (name, value) in compiler.defs() {
            type_env.add_value(*name, value);
        }
        type_env
    }

    /// Builds a type environment from a runtime environment only.
    ///
    /// This is useful for testing or when compiler definitions are not needed.
    /// For macros, use `from_context` instead to include both runtime and compiler definitions.
    pub fn from_env(env: &crate::env::Env) -> Self {
        let mut type_env = Self::new();
        for (name, value) in env.iter() {
            type_env.add_value(name, value);
        }
        type_env
    }

    /// Adds a value's type to the environment.
    ///
    /// This converts the runtime value's type to an InferType for use in type checking.
    pub fn add_value(&mut self, name: InternedString, value: &crate::value::Value) {
        let ty = value.type_of();
        self.insert(name, InferType::from_concrete(&ty));
    }

    /// Inserts a binding into the environment.
    pub fn insert(&mut self, name: InternedString, ty: InferType) {
        self.bindings.insert(name, ty);
    }

    /// Looks up a variable in the environment.
    pub fn get(&self, name: InternedString) -> Option<&InferType> {
        self.bindings.get(&name)
    }

    /// Returns the free type variables in this environment.
    pub fn free_vars(&self) -> Vec<TypeVar> {
        let mut vars = Vec::new();
        for ty in self.bindings.values() {
            ty.collect_free_vars(&mut vars);
        }
        vars.sort();
        vars.dedup();
        vars
    }

    /// Applies a substitution to all types in this environment.
    pub fn apply(&self, subst: &Substitution) -> TypeEnv {
        let mut new_env = TypeEnv::new();
        for (name, ty) in &self.bindings {
            new_env.insert(*name, subst.apply(ty));
        }
        new_env
    }
}

impl Default for TypeEnv {
    fn default() -> Self {
        Self::new()
    }
}

/// The type inference engine.
#[derive(Debug)]
pub struct TypeInferencer {
    /// Counter for generating fresh type variables.
    next_var: u32,
}

impl TypeInferencer {
    /// Creates a new type inferencer.
    pub fn new() -> Self {
        Self { next_var: 0 }
    }

    /// Generates a fresh type variable.
    pub fn fresh_var(&mut self) -> TypeVar {
        let var = TypeVar::new(self.next_var);
        self.next_var += 1;
        var
    }

    /// Unifies two types, returning a substitution that makes them equal.
    ///
    /// This implements the standard unification algorithm with an occurs check.
    #[allow(clippy::only_used_in_recursion)]
    pub fn unify(&self, t1: &InferType, t2: &InferType, span: Span) -> Result<Substitution> {
        match (t1, t2) {
            // Two identical concrete types unify with empty substitution
            (InferType::Concrete(a), InferType::Concrete(b)) if a == b => Ok(Substitution::new()),

            // Type variable unifies with anything (occurs check)
            (InferType::Var(v), t) | (t, InferType::Var(v)) => {
                if let InferType::Var(v2) = t
                    && v == v2
                {
                    return Ok(Substitution::new());
                }
                // Occurs check: prevent infinite types
                if t.free_vars().contains(v) {
                    return Err(Box::new(Diagnostic::new(
                        DiagnosticKind::InternalError(format!(
                            "occurs check failed: {} occurs in {}",
                            v, t
                        )),
                        Some(span),
                    )));
                }
                Ok(Substitution::singleton(*v, t.clone()))
            }

            // Function types unify if arguments and return types unify
            (InferType::Fn(args1, ret1), InferType::Fn(args2, ret2)) => {
                if args1.len() != args2.len() {
                    return Err(Box::new(Diagnostic::new(
                        DiagnosticKind::ArityError {
                            expected: args1.len(),
                            actual: args2.len(),
                        },
                        Some(span),
                    )));
                }

                let mut subst = Substitution::new();
                for (arg1, arg2) in args1.iter().zip(args2.iter()) {
                    let s = self.unify(&subst.apply(arg1), &subst.apply(arg2), span)?;
                    subst = s.compose(&subst);
                }
                let s = self.unify(&subst.apply(ret1), &subst.apply(ret2), span)?;
                Ok(s.compose(&subst))
            }

            // List types unify if element types unify
            (InferType::List(elem1), InferType::List(elem2)) => self.unify(elem1, elem2, span),

            // Record types unify if they have the same fields with unifiable types
            (InferType::Record(fields1), InferType::Record(fields2)) => {
                if fields1.len() != fields2.len() {
                    return Err(Box::new(Diagnostic::new(
                        DiagnosticKind::InternalError(format!(
                            "record field count mismatch: expected {} fields, got {}",
                            fields1.len(),
                            fields2.len()
                        )),
                        Some(span),
                    )));
                }

                let mut subst = Substitution::new();
                for ((name1, ty1), (name2, ty2)) in fields1.iter().zip(fields2.iter()) {
                    if name1 != name2 {
                        return Err(Box::new(Diagnostic::new(
                            DiagnosticKind::InternalError(format!(
                                "record field name mismatch: {} vs {}",
                                &**name1, &**name2
                            )),
                            Some(span),
                        )));
                    }
                    let s = self.unify(&subst.apply(ty1), &subst.apply(ty2), span)?;
                    subst = s.compose(&subst);
                }
                Ok(subst)
            }

            // Tuple types unify if element types unify
            (InferType::Tuple(elems1), InferType::Tuple(elems2)) => {
                if elems1.len() != elems2.len() {
                    return Err(Box::new(Diagnostic::new(
                        DiagnosticKind::InternalError(format!(
                            "tuple size mismatch: expected {} elements, got {}",
                            elems1.len(),
                            elems2.len()
                        )),
                        Some(span),
                    )));
                }

                let mut subst = Substitution::new();
                for (elem1, elem2) in elems1.iter().zip(elems2.iter()) {
                    let s = self.unify(&subst.apply(elem1), &subst.apply(elem2), span)?;
                    subst = s.compose(&subst);
                }
                Ok(subst)
            }

            // Forall types need instantiation before unification
            (InferType::Forall(_, _), _) | (_, InferType::Forall(_, _)) => {
                // This should not happen in practice as we instantiate before unifying
                Err(Box::new(Diagnostic::new(
                    DiagnosticKind::InternalError(
                        "cannot unify polymorphic types directly".to_string(),
                    ),
                    Some(span),
                )))
            }

            // Otherwise, types don't unify
            _ => Err(Box::new(Diagnostic::new(
                DiagnosticKind::InternalError(format!(
                    "type mismatch: cannot unify {} with {}",
                    t1, t2
                )),
                Some(span),
            ))),
        }
    }

    /// Generalizes a type by quantifying over free variables.
    ///
    /// Variables that are free in the type but not in the environment
    /// are quantified with forall.
    pub fn generalize(&self, ty: &InferType, env: &TypeEnv) -> InferType {
        let ty_vars = ty.free_vars();
        let env_vars = env.free_vars();

        // Quantify over variables that are free in the type but not in the environment
        let mut quantified = Vec::new();
        for var in ty_vars {
            if !env_vars.contains(&var) {
                quantified.push(var);
            }
        }

        if quantified.is_empty() {
            ty.clone()
        } else {
            quantified.sort();
            InferType::Forall(quantified, Box::new(ty.clone()))
        }
    }

    /// Instantiates a polymorphic type by replacing quantified variables with fresh variables.
    pub fn instantiate(&mut self, ty: &InferType) -> InferType {
        match ty {
            InferType::Forall(vars, body) => {
                let mut subst = Substitution::new();
                for var in vars {
                    let fresh = self.fresh_var();
                    subst.insert(*var, InferType::Var(fresh));
                }
                subst.apply(body)
            }
            _ => ty.clone(),
        }
    }
}

impl Default for TypeInferencer {
    fn default() -> Self {
        Self::new()
    }
}

/// Type inference for expressions.
///
/// This provides type inference that can be used during evaluation,
/// including for unevaluated code paths and macro metaprogramming.
impl TypeInferencer {
    /// Infers the type of an expression given a type environment.
    ///
    /// This generates constraints and solves them to produce a type.
    /// The expression is not evaluated - this is pure type analysis.
    pub fn infer_expr(
        &mut self,
        expr: &cadenza_syntax::ast::Expr,
        env: &TypeEnv,
    ) -> Result<InferType> {
        use cadenza_syntax::ast::Expr;

        match expr {
            Expr::Literal(lit) => self.infer_literal(lit),
            Expr::Ident(ident) => self.infer_ident(ident, env),
            Expr::Apply(apply) => self.infer_apply(apply, env),
            Expr::Op(op) => self.infer_op(op, env),
            Expr::Attr(attr) => self.infer_attr(attr, env),
            Expr::Synthetic(syn) => self.infer_synthetic(syn, env),
            Expr::Error(_) => {
                // Error nodes represent parsing errors, type is unknown
                Ok(InferType::Concrete(Type::Unknown))
            }
        }
    }

    fn infer_literal(&mut self, lit: &cadenza_syntax::ast::Literal) -> Result<InferType> {
        use cadenza_syntax::ast::LiteralValue;

        let ty = match lit.value() {
            Some(LiteralValue::Integer(_)) => Type::Integer,
            Some(LiteralValue::Float(_)) => Type::Float,
            Some(LiteralValue::String(_)) | Some(LiteralValue::StringWithEscape(_)) => Type::String,
            None => Type::Unknown,
        };
        Ok(InferType::Concrete(ty))
    }

    fn infer_ident(
        &mut self,
        ident: &cadenza_syntax::ast::Ident,
        env: &TypeEnv,
    ) -> Result<InferType> {
        let name = ident.syntax().text().interned();

        if let Some(ty) = env.get(name) {
            // Instantiate polymorphic types
            Ok(self.instantiate(ty))
        } else {
            // Unknown identifier - return a fresh type variable
            // In a full implementation, this would be an error
            Ok(InferType::Var(self.fresh_var()))
        }
    }

    fn infer_apply(
        &mut self,
        apply: &cadenza_syntax::ast::Apply,
        env: &TypeEnv,
    ) -> Result<InferType> {
        // Infer type of the callee
        let callee_ty = if let Some(callee) = apply.callee() {
            self.infer_expr(&callee, env)?
        } else {
            return Ok(InferType::Concrete(Type::Unknown));
        };

        // Infer types of arguments
        let mut arg_types = Vec::new();
        for arg in apply.all_arguments() {
            arg_types.push(self.infer_expr(&arg, env)?);
        }

        // The result type is a fresh type variable
        let result_var = self.fresh_var();
        let result_ty = InferType::Var(result_var);

        // Create a function type for the expected callee type
        let expected_fn_ty = InferType::Fn(arg_types, Box::new(result_ty.clone()));

        // Unify the callee type with the expected function type
        let span = apply.span();
        let subst = self.unify(&callee_ty, &expected_fn_ty, span)?;

        // Apply substitution to get the result type
        Ok(subst.apply(&result_ty))
    }

    fn infer_op(&mut self, op: &cadenza_syntax::ast::Op, env: &TypeEnv) -> Result<InferType> {
        // Operators are looked up as identifiers in the environment
        // At runtime, they evaluate to Symbol values, but for type inference
        // we look up their registered type (e.g., Integer -> Integer -> Integer for +)
        let name = op.syntax().text().interned();

        if let Some(ty) = env.get(name) {
            // Instantiate polymorphic types
            Ok(self.instantiate(ty))
        } else {
            // Unknown operator - return a fresh type variable
            Ok(InferType::Var(self.fresh_var()))
        }
    }

    fn infer_attr(&mut self, attr: &cadenza_syntax::ast::Attr, env: &TypeEnv) -> Result<InferType> {
        // Attributes evaluate to the type of their value expression
        if let Some(value_expr) = attr.value() {
            self.infer_expr(&value_expr, env)
        } else {
            // Missing value, return Nil type
            Ok(InferType::Concrete(Type::Nil))
        }
    }

    fn infer_synthetic(
        &mut self,
        syn: &cadenza_syntax::ast::Synthetic,
        env: &TypeEnv,
    ) -> Result<InferType> {
        // Synthetic nodes are looked up like identifiers
        // They represent semantic concepts like __block__, __list__, __record__
        let name = InternedString::new(syn.identifier());

        if let Some(ty) = env.get(name) {
            // Instantiate polymorphic types
            Ok(self.instantiate(ty))
        } else {
            // Unknown synthetic node - return a fresh type variable
            Ok(InferType::Var(self.fresh_var()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unify_concrete_types() {
        let inf = TypeInferencer::new();
        let t1 = InferType::Concrete(Type::Integer);
        let t2 = InferType::Concrete(Type::Integer);
        let result = inf.unify(&t1, &t2, Span::new(0, 0));
        assert!(result.is_ok());
        let subst = result.unwrap();
        assert_eq!(subst.map.len(), 0);
    }

    #[test]
    fn test_unify_var_with_concrete() {
        let inf = TypeInferencer::new();
        let v = TypeVar::new(0);
        let t1 = InferType::Var(v);
        let t2 = InferType::Concrete(Type::Integer);
        let result = inf.unify(&t1, &t2, Span::new(0, 0));
        assert!(result.is_ok());
        let subst = result.unwrap();
        assert_eq!(subst.apply(&t1), t2);
    }

    #[test]
    fn test_unify_function_types() {
        let inf = TypeInferencer::new();
        let t1 = InferType::Fn(
            vec![InferType::Concrete(Type::Integer)],
            Box::new(InferType::Concrete(Type::Integer)),
        );
        let t2 = InferType::Fn(
            vec![InferType::Concrete(Type::Integer)],
            Box::new(InferType::Concrete(Type::Integer)),
        );
        let result = inf.unify(&t1, &t2, Span::new(0, 0));
        assert!(result.is_ok());
    }

    #[test]
    fn test_occurs_check() {
        let inf = TypeInferencer::new();
        let v = TypeVar::new(0);
        let t1 = InferType::Var(v);
        let t2 = InferType::List(Box::new(InferType::Var(v)));
        let result = inf.unify(&t1, &t2, Span::new(0, 0));
        assert!(result.is_err());
    }

    #[test]
    fn test_generalize() {
        let mut inf = TypeInferencer::new();
        let v = inf.fresh_var();
        let ty = InferType::Fn(vec![InferType::Var(v)], Box::new(InferType::Var(v)));
        let env = TypeEnv::new();
        let generalized = inf.generalize(&ty, &env);
        match generalized {
            InferType::Forall(vars, _) => {
                assert_eq!(vars.len(), 1);
                assert_eq!(vars[0], v);
            }
            _ => panic!("expected Forall type"),
        }
    }

    #[test]
    fn test_instantiate() {
        let mut inf = TypeInferencer::new();
        let v = inf.fresh_var(); // Use fresh_var to create the variable
        let ty = InferType::Forall(
            vec![v],
            Box::new(InferType::Fn(
                vec![InferType::Var(v)],
                Box::new(InferType::Var(v)),
            )),
        );
        let instantiated = inf.instantiate(&ty);
        // Should have replaced v with a fresh variable
        match instantiated {
            InferType::Fn(args, ret) => {
                match (&args[0], &*ret) {
                    (InferType::Var(v1), InferType::Var(v2)) => {
                        assert_eq!(v1, v2);
                        // Should be different from the original v
                        assert_ne!(*v1, v);
                    }
                    _ => panic!("expected Var types"),
                }
            }
            _ => panic!("expected Fn type"),
        }
    }

    #[test]
    fn test_substitution_compose() {
        let v1 = TypeVar::new(0);
        let v2 = TypeVar::new(1);

        let s1 = Substitution::singleton(v1, InferType::Concrete(Type::Integer));
        let s2 = Substitution::singleton(v2, InferType::Var(v1));

        let composed = s1.compose(&s2);
        let t = InferType::Var(v2);
        let result = composed.apply(&t);
        assert_eq!(result, InferType::Concrete(Type::Integer));
    }

    #[test]
    fn test_free_vars() {
        let v1 = TypeVar::new(0);
        let v2 = TypeVar::new(1);

        let ty = InferType::Fn(vec![InferType::Var(v1)], Box::new(InferType::Var(v2)));

        let free = ty.free_vars();
        assert_eq!(free.len(), 2);
        assert!(free.contains(&v1));
        assert!(free.contains(&v2));
    }

    #[test]
    fn test_display_type_var() {
        assert_eq!(format!("{}", TypeVar::new(0)), "t0");
        assert_eq!(format!("{}", TypeVar::new(1)), "t1");
        assert_eq!(format!("{}", TypeVar::new(25)), "t25");
    }

    #[test]
    fn test_type_env_from_env() {
        use crate::{env::Env, value::Value};

        let mut env = Env::new();
        let x: InternedString = "x".into();
        let y: InternedString = "y".into();
        env.define(x, Value::Integer(42));
        env.define(y, Value::String("hello".into()));

        let type_env = TypeEnv::from_env(&env);

        // Check that variables were converted to their types
        assert_eq!(type_env.get(x), Some(&InferType::Concrete(Type::Integer)));
        assert_eq!(type_env.get(y), Some(&InferType::Concrete(Type::String)));
    }

    #[test]
    fn test_type_env_from_env_with_scopes() {
        use crate::{env::Env, value::Value};

        let mut env = Env::new();
        let x: InternedString = "x".into();
        let y: InternedString = "y".into();

        env.define(x, Value::Integer(1));
        env.push_scope();
        env.define(y, Value::String("outer".into()));
        env.push_scope();
        // Shadow y in inner scope
        env.define(y, Value::String("inner".into()));

        let type_env = TypeEnv::from_env(&env);

        // Check that only innermost bindings are included
        assert_eq!(type_env.get(x), Some(&InferType::Concrete(Type::Integer)));
        assert_eq!(type_env.get(y), Some(&InferType::Concrete(Type::String)));
    }

    #[test]
    fn test_type_env_from_context() {
        use crate::{
            compiler::Compiler,
            env::Env,
            value::{BuiltinFn, Type, Value},
        };

        let mut env = Env::new();
        let mut compiler = Compiler::new();

        let x: InternedString = "x".into();
        let add: InternedString = "add".into();

        // Add variable to environment
        env.define(x, Value::Integer(42));

        // Add function to compiler
        compiler.define_var(
            add,
            Value::BuiltinFn(BuiltinFn {
                name: "add",
                signature: Type::function(vec![Type::Integer, Type::Integer], Type::Integer),
                func: |_, _| unreachable!(),
            }),
        );

        let type_env = TypeEnv::from_context(&env, &compiler);

        // Check both env and compiler values are included
        assert_eq!(type_env.get(x), Some(&InferType::Concrete(Type::Integer)));
        // Function should have a function type
        assert!(matches!(type_env.get(add), Some(InferType::Fn(_, _))));
    }
}
