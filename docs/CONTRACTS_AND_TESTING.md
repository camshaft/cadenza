# Contracts and Property-Based Testing

## Overview

This document describes Cadenza's design for **contract programming** (preconditions, postconditions, invariants, and constrained types) and **integrated property-based testing**. These features work together to enable incremental opt-in to formal verification, allowing developers to build progressively more robust code.

## Table of Contents

1. [Design Philosophy](#design-philosophy)
2. [Numeric Type Subtyping with Predicates](#numeric-type-subtyping-with-predicates)
3. [Contract Programming](#contract-programming)
4. [Built-in Testing Framework](#built-in-testing-framework)
5. [Property-Based Testing](#property-based-testing)
6. [Static and Dynamic Analysis](#static-and-dynamic-analysis)
7. [Type-Driven Test Generation](#type-driven-test-generation)
8. [Examples](#examples)
9. [Implementation Roadmap](#implementation-roadmap)

---

## Design Philosophy

### Core Principles

1. **Incremental Opt-In**: Start with simple types, add constraints as needed, progress to full formal verification
2. **Compile-Time and Runtime**: Contracts can be checked statically, dynamically, or both
3. **Type-Integrated**: Constraints are part of the type system, not separate annotations
4. **Self-Documenting**: Contracts serve as executable documentation
5. **Test Generation**: Types and contracts guide property test generation

### Inspirations

- **Ada**: Constrained numeric types with range and modular types
- **Eiffel**: Design by Contract with pre/post conditions
- **Rust**: Integrated testing with `#[test]` and property testing with `bolero`
- **Dafny**: Formal verification with SMT solvers
- **QuickCheck**: Property-based testing from Haskell

### Goals

- Make it easy to write robust code with minimal ceremony
- Provide clear error messages when constraints are violated
- Enable gradual adoption: use what you need, when you need it
- Support both quick prototyping and production-grade reliability

---

## Numeric Type Subtyping with Predicates

### Motivation

Integer and Float types are often too permissive. Real-world values have constraints:
- Array indices must be non-negative
- Percentages are typically 0-100
- Age cannot be negative
- Temperatures have physical limits

Ada's approach of constrained subtypes provides compile-time and runtime safety without requiring dependent types or full theorem proving.

### Basic Constrained Types

Define subtypes with range constraints using attributes:

```cadenza
# Range-constrained integer type
@invariant $0 >= 0
type Natural = Integer

# Bounded range
@invariant $0 >= 0.0
@invariant $0 <= 100.0
type Percentage = Float

# Multiple constraints with custom messages
@invariant $0 >= 0 "age cannot be negative"
@invariant $0 < 150 "age must be realistic"
type Age = Integer

# Using the constrained types
fn discount (percent: Percentage) (price: Float) =
  price * (1.0 - percent / 100.0)

let price = discount 15.0 100.0  # OK: 15.0 is in range [0.0, 100.0]
let bad = discount 150.0 100.0   # Error: 150.0 violates Percentage constraint
```

### Predicate Syntax

Constraints use attribute predicates where `$0` represents the value being constrained:

```cadenza
@invariant predicate
type T = BaseType

# Multiple constraints with custom messages
@invariant constraint1 "error message 1"
@invariant constraint2 "error message 2"
type T = BaseType

# Calling functions from attributes to encapsulate checks
@invariant is_valid_email $0 "must be a valid email"
type Email = String
```

### Constraint Checking

**At Construction Time**:
- When a value is assigned to a constrained type variable
- When passing arguments to functions expecting constrained types
- When returning from functions with constrained return types

```cadenza
@invariant $0 > 0
type PositiveInt = Integer

fn reciprocal (n: PositiveInt) = 1.0 / n

# Type checker knows n cannot be zero, so division is safe
```

**Static Analysis** (when possible):
- Literal values: `let x: Natural = -5` → compile-time error "constraint violated: value must be >= 0"
- Known constraints: `let x = 5; let y: Natural = x` → compile-time OK
- Dependent relationships: `let x: Natural = y + 1` → OK if `y: Natural`

**Dynamic Checks** (when necessary):
- Runtime values: `let x: Natural = parse_int input` → runtime check
- Complex predicates: SMT solver may not prove constraint

### Units and Constraints

Constrained types work with dimensional analysis:

```cadenza
# Note: Both dimensional types and the Quantity type are still being designed
# Quantity would be a built-in type that represents dimensioned values
# The syntax meter/second represents the dimension specification
@invariant $0 >= 0.0 "speed cannot be negative"
@invariant $0 < 299792458.0 "speed cannot exceed speed of light"
type Speed = Quantity meter/second

@invariant $0 >= 0.0 "temperature cannot be below absolute zero"
type Temperature = Quantity kelvin
```

### Dependent Types (Future)

More sophisticated constraints that depend on other values:

```cadenza
# Array with length tracked in the type (conceptual syntax)
# The type parameter n is used to constrain the length field
@invariant length == n
type Array (n: Natural) (T: Type) = {
  length = Natural,
  elements = List T
}

# Function that preserves array length
fn map (f: T -> U) (arr: Array n T) -> Array n U = ...
```

**Note**: Dependent types would be valuable to add early in development, as they may be difficult to retrofit later. They enable powerful compile-time guarantees about relationships between values. The exact syntax for how type parameters interact with invariants and how invariants apply to struct types is still being designed.

---

## Contract Programming

### Preconditions, Postconditions, and Invariants

Use `@` attributes to attach contracts to functions and types.

### Function Contracts

```cadenza
fn divide (a: Float) (b: Float) -> Float
  @requires { b != 0.0 "divisor cannot be zero" }
  @ensures { result * b ~= a "result times divisor approximately equals dividend" }
  # Note: ~= is approximate equality operator for floating-point comparisons
=
  a / b

# Calling with invalid input
let x = divide 10.0 0.0  # Error: precondition violated: "divisor cannot be zero"
```

### Multiple Conditions

```cadenza
fn binary_search (arr: List Integer) (target: Integer) -> Integer
  @requires { 
    is_sorted arr "array must be sorted",
    length arr > 0 "array cannot be empty"
  }
  @ensures {
    result >= -1 "result is valid index or -1",
    result < length arr "result is within bounds",
    result >= 0 ==> get arr result == target "if found, element matches target"
  }
=
  # implementation
  ...
```

### Invariants

Type invariants ensure that values always satisfy certain properties:

```cadenza
struct BankAccount {
  balance = Float @ { x >= 0.0 },
  owner = String,
}
  @invariant { balance >= 0.0 "balance cannot be negative" }

# Methods must preserve invariants
fn withdraw (account: BankAccount) (amount: Float) -> BankAccount
  @requires { amount > 0.0 && amount <= account.balance }
  @ensures { result.balance == account.balance - amount }
=
  { ...account, balance = account.balance - amount }

# This would violate the invariant and be rejected
fn break_account (account: BankAccount) -> BankAccount =
  { ...account, balance = -100.0 }  # Error: violates balance >= 0.0 invariant
```

### Loop Invariants (Future)

For formal verification of loops:

```cadenza
fn sum_array (arr: Array n Integer) -> Integer
  @ensures { result == sum_of_elements arr }
=
  let total = 0
  let i = 0
  while i < n
    @invariant { 
      0 <= i && i <= n,
      total == sum (take i arr)
    }
  do
    total = total + arr[i]
    i = i + 1
  total
```

### Attribute Syntax

Contract attributes use `@name { ... }` syntax:

- `@requires { predicate }` - Precondition (checked on entry)
- `@ensures { predicate }` - Postcondition (checked on exit, can reference `result`)
- `@invariant { predicate }` - Type invariant (checked on construction and mutation)
- `@modifies { variable_list }` - Documents side effects (future)

---

## Built-in Testing Framework

### Motivation

Testing should be integrated into the language, not a separate tool. Inspired by Rust's `#[test]` attribute and built-in test runner.

### Test Syntax

Mark functions as tests using `@test` attribute:

```cadenza
@test
fn test_addition =
  assert 1 + 1 == 2
  assert 2 + 2 == 4

@test "edge case: zero"
fn test_zero =
  assert 0 + 0 == 0
  assert 5 + 0 == 5

@test "negative numbers"
fn test_negatives =
  assert -1 + 1 == 0
  assert -5 + -3 == -8

# Use tags to categorize tests for filtering
@test
@tag "unit"
@tag "arithmetic"
fn test_add_positive =
  assert 2 + 3 == 5

@test
@tag "integration"
@tag "database"
fn test_db_connection =
  # integration test code
  assert true
```

### Test Organization

Tests are discovered automatically in the codebase. Tests can be placed in the same file as the code being tested, or in separate test files using the `.test.cdz` extension convention.

```cadenza
# In module: math.cdz

fn add (a: Integer) (b: Integer) = a + b

# Tests in the same file
@test
fn test_add_positive =
  assert add 2 3 == 5

@test
fn test_add_negative =
  assert add -2 3 == 1

# Or in a separate test file: math.test.cdz
# Using the .test.cdz extension helps organize tests separately from production code
@test
fn test_add_zero =
  assert add 0 5 == 5
```

### Test Runner

```bash
# Run all tests
cadenza test

# Run tests in specific module (default behavior with filter)
cadenza test math

# Run tests matching pattern (default mode, similar to cargo test)
cadenza test addition

# Exclude tests by tag
cadenza test --exclude-tag integration

# Include only specific tags
cadenza test --tag unit

# Run tests in watch mode
cadenza test --watch
```

### Setup and Teardown

```cadenza
# Test fixtures with setup/teardown
@test
fn test_with_fixture =
  let db = setup_test_database
  
  # Test code
  let result = query db "SELECT * FROM users"
  assert length result == 0
  
  # Teardown happens automatically (or use defer/finally)
  cleanup_test_database db
```

### Test Organization with Tags

Instead of explicit test groups, use tags to organize and filter tests:

```cadenza
# Use tags to categorize related tests
@test
@tag "arithmetic"
fn test_addition = assert 1 + 1 == 2

@test
@tag "arithmetic"
fn test_subtraction = assert 5 - 3 == 2

@test
@tag "arithmetic"
fn test_multiplication = assert 3 * 4 == 12

# Run all arithmetic tests: cadenza test --tag arithmetic
```

### Expected Failures

```cadenza
@test @should_panic "division by zero"
fn test_divide_by_zero =
  let x = 1 / 0
  x  # Should panic before reaching here
```

---

## Property-Based Testing

### Motivation

Example-based tests are limited. Property-based testing generates many test cases automatically, finding edge cases developers might miss. Inspired by QuickCheck and Rust's `bolero`.

### Basic Property Tests

Use `@property` to define property-based tests:

```cadenza
@property
fn prop_addition_commutative (a: Integer) (b: Integer) =
  assert a + b == b + a

@property
fn prop_sort_idempotent (list: List Integer) =
  let sorted_once = sort list
  let sorted_twice = sort sorted_once
  assert sorted_once == sorted_twice
```

### Property Test Configuration

```cadenza
@property { cases = 1000, max_size = 100 }
fn prop_reverse_twice (list: List Integer) =
  assert reverse (reverse list) == list

@property { timeout = 5s }
fn prop_no_infinite_loop (n: Integer) =
  assert fibonacci n >= 0
```

### Shrinking

When a property fails, automatically find the minimal failing case:

```cadenza
@property
fn prop_all_positive (list: List Integer) =
  assert all (map (x -> x > 0) list)

# If this fails with list = [1, 2, -5, 10, 3], 
# the shrinker will reduce it to list = [-5] or list = [0]
# to show the minimal failing case
```

### Custom Generators

Define how to generate arbitrary values for custom types:

```cadenza
# Automatic generator for simple types
struct Point {
  x = Float,
  y = Float,
}

# The framework automatically generates arbitrary Points

# Custom generator for constrained types
type Email = String @ { is_valid_email x }

@generator Email
fn generate_email =
  # Note: These are hypothetical helper functions for illustration
  let username = generate_alphanumeric 10  # Generate 10 random alphanumeric chars
  let domain = generate_domain             # Generate random domain like "example.com"
  "${username}@${domain}"

# Use in property tests
@property
fn prop_email_roundtrip (email: Email) =
  assert parse_email (format_email email) == email
```

### Stateful Property Testing

Test stateful systems with command sequences:

```cadenza
struct BankAccountModel {
  balance = Float,
}

@invariant $0 > 0.0
type PositiveAmount = Float

# Note: Complex constraints referencing runtime values are still being designed
# The syntax below shows the conceptual interface
@stateful_property
fn prop_bank_account_invariant =
  let model = BankAccountModel { balance = 100.0 }
  
  @command "deposit"
  fn deposit (amount: PositiveAmount) =
    model.balance = model.balance + amount
    assert model.balance >= 0.0
  
  @command "withdraw"  
  fn withdraw (amount: PositiveAmount) =
    # Precondition check: amount must not exceed balance
    if amount <= model.balance then
      model.balance = model.balance - amount
      assert model.balance >= 0.0

# Framework generates random command sequences:
# deposit 50.0, withdraw 30.0, deposit 20.0, withdraw 100.0, ...
# Checks that invariant holds after each command
```

---

## Static and Dynamic Analysis

### Verification Levels

Cadenza supports multiple levels of contract checking:

1. **None**: Contracts are documentation only (fastest)
2. **Dynamic**: Runtime checks (default)
3. **Static**: Compile-time verification (slowest, most thorough)
4. **Hybrid**: Static where possible, dynamic fallback

### Configuration

Verification level is configured at build time, not in the code:

```bash
# Development: full dynamic checking
cadenza build --verification dynamic

# Release: static verification where possible, dynamic fallback
cadenza build --verification hybrid

# Maximum verification (slowest, most thorough)
cadenza build --verification static

# No contract checking (fastest, least safe)
cadenza build --verification none
```

Per-function verification levels can be specified as hints to the compiler:

```cadenza
# Suggest static verification for critical functions
@verification_hint "static"
fn critical_function (x: Integer) -> Integer
  @requires { x > 0 }
  @ensures { result > x }
=
  x * 2
```

### Static Analysis with SMT Solvers

For functions with static verification hints, Cadenza can use SMT solvers (Z3, CVC5) to prove correctness:

```cadenza
@invariant $0 != 0
type NonZero = Integer

fn safe_divide (a: Integer) (b: NonZero) -> Integer
  @verification_hint "static"
=
  a / b  # Provably safe: type system ensures b != 0
```

### Dynamic Checking

When static analysis cannot prove correctness, insert runtime checks:

```cadenza
fn process_user_input (input: String) -> Natural
  @ensures { result >= 0 }
=
  let parsed = parse_int input
  # Runtime check: verify result is non-negative
  if parsed < 0 then
    panic "parsed value must be non-negative"
  else
    parsed
```

### Optimization

In release builds, verified contracts can be removed:

```bash
# Development: full checking
cadenza build --verification dynamic

# Release: remove checks that were statically verified
cadenza build --release --verification optimized
```

---

## Type-Driven Test Generation

### Arbitrary Value Generation

The framework generates values based on type information and constraints:

```cadenza
# Simple types have built-in generators
arbitrary<Integer>      # Any integer
arbitrary<Float>        # Any float
arbitrary<String>       # Any string
arbitrary<Bool>         # true or false

# Constrained types respect constraints
arbitrary<Natural>      # Only non-negative integers
arbitrary<Percentage>   # Only 0.0 to 100.0

# Compound types generate recursively
arbitrary<List Integer>      # List of integers
arbitrary<(Integer, String)> # Pairs
arbitrary<Option String>     # None or Some string
```

### Using Type Invariants

Type invariants guide test generation:

```cadenza
@invariant $0 % 2 == 0
type Even = Integer

@invariant $0 % 2 == 1
type Odd = Integer

@property
fn prop_even_plus_odd_is_odd (e: Even) (o: Odd) =
  let result = e + o
  assert result % 2 == 1

# Generator only produces even numbers for e, odd numbers for o
# No invalid cases are generated
```

### Custom Generators for Complex Types

```cadenza
struct SortedList (T: Type) {
  elements = List T,
}
  @invariant { is_sorted elements }

# Custom generator ensures invariant
@generator (SortedList Integer)
fn generate_sorted_list =
  let list = arbitrary<List Integer>
  SortedList { elements = sort list }

# Or implement Arbitrary trait
impl Arbitrary for SortedList Integer =
  fn arbitrary =
    let list = arbitrary<List Integer>
    SortedList { elements = sort list }
```

### Shrinking Strategies

When a test fails, shrink the input to find minimal failing case:

```cadenza
# Built-in shrinking for primitives
# Integer: 100 -> 50 -> 25 -> 12 -> 6 -> 3 -> 1 -> 0
# String: "hello" -> "hell" -> "hel" -> "he" -> "h" -> ""
# List: [1,2,3,4] -> [1,2,3] -> [1,2] -> [1] -> []

# Custom shrinking for user types
impl Shrink for Email =
  fn shrink (email: Email) =
    # Try removing characters while maintaining Email invariant
    let shorter = remove_char email
    if is_valid_email shorter then [shorter] else []
```

---

## Examples

### Example 1: Array Bounds

```cadenza
# Conceptual syntax - exact handling of type parameters in invariants TBD
@invariant $0 >= 0
@invariant $0 < n  # References type parameter n
type Index (n: Natural) = Integer

fn safe_get (arr: Array n T) (i: Index n) -> T =
  # No bounds check needed: type system ensures i is valid
  arr[i]

@property
fn prop_safe_get_no_panic (arr: Array n Integer) (i: Index n) =
  # This property never panics because Index n guarantees valid index
  let value = safe_get arr i
  assert true  # Always passes
```

**Note**: The exact syntax for how invariants reference type parameters is still being designed. This example illustrates the concept of dependent type constraints.

### Example 2: Validated Input

```cadenza
@invariant length $0 >= 3 "username must be at least 3 characters"
@invariant length $0 <= 20 "username must be at most 20 characters"
@invariant all_alphanumeric $0 "username must be alphanumeric"
type Username = String

@invariant contains $0 "@" "email must contain @"
@invariant valid_email $0 "email must be valid format"
type Email = String

@invariant $0 >= 18 "user must be an adult"
@invariant $0 < 150 "age must be realistic"
type UserAge = Integer

struct User {
  username = Username,
  email = Email,
  age = UserAge,
}

@property
fn prop_user_validation (username: String) (email: String) (age: Integer) =
  match create_user username email age
    Ok user ->
      # If creation succeeded, all constraints are satisfied
      assert length user.username >= 3
      assert length user.username <= 20
      assert contains user.email "@"
      assert user.age >= 18
    Err error ->
      # If creation failed, at least one constraint was violated
      assert true
```

### Example 3: Sorting Contract

```cadenza
fn sort (list: List Integer) -> List Integer
  @ensures {
    length result == length list "preserves length",
    is_sorted result "result is sorted",
    is_permutation result list "result is permutation of input"
  }
=
  # Implementation
  ...

@property
fn prop_sort_contract (list: List Integer) =
  let sorted = sort list
  # Postconditions automatically checked
  assert length sorted == length list
  assert is_sorted sorted
  assert is_permutation sorted list
```

### Example 4: State Machine Testing

```cadenza
enum TrafficLight {
  Red,
  Yellow,
  Green,
}

fn next_light (current: TrafficLight) -> TrafficLight
  @ensures {
    # Ensure valid transitions
    (current == Red && result == Green) ||
    (current == Green && result == Yellow) ||
    (current == Yellow && result == Red)
  }
=
  match current
    Red -> Green
    Green -> Yellow
    Yellow -> Red

@property
fn prop_traffic_light_cycle (initial: TrafficLight) =
  let after_1 = next_light initial
  let after_2 = next_light after_1
  let after_3 = next_light after_2
  # After 3 transitions, we should be back to initial
  assert after_3 == initial
```

### Example 5: Numeric Constraints with Units

```cadenza
# Note: Syntax for dimensional types is still being designed
@invariant $0 >= 0.0 "speed cannot be negative"
@invariant $0 < 299792458.0 "speed cannot exceed speed of light"
type Speed = Quantity meter/second

@invariant $0 >= 0.0 "distance cannot be negative"
type Distance = Quantity meter

@invariant $0 > 0.0 "time must be positive"
type Time = Quantity second

fn calculate_speed (distance: Distance) (time: Time) -> Speed
  @ensures { result >= 0.0 && result < 299792458.0 }
=
  distance / time

@property
fn prop_speed_calculation (distance: Distance) (time: Time) =
  let speed = calculate_speed distance time
  # Postcondition ensures speed is valid
  assert speed >= 0.0
  assert speed < 299792458.0
  # Also check the math
  assert speed * time ~= distance
```

---

## Implementation Roadmap

### Phase 1: Constrained Types (Foundation)

1. **Type system support for constrained types**
   - Add `Type::Constrained { base: Type, predicate: Expr }` variant
   - Extend type checker to validate constraints
   - Constraint checking at assignment and function boundaries

2. **Basic constraint syntax**
   - Parser support for `Type @ { predicate }` syntax
   - AST representation for constrained types
   - Error messages for constraint violations

3. **Dynamic checking**
   - Insert runtime checks for constraints
   - Report violations with clear error messages
   - Track source location of constraint definitions

### Phase 2: Function Contracts

1. **Attribute syntax for contracts**
   - Parser support for `@requires`, `@ensures`, `@invariant`
   - AST representation for contract attributes
   - Attach contracts to function definitions and types

2. **Runtime contract checking**
   - Evaluate preconditions on function entry
   - Evaluate postconditions on function exit
   - Report contract violations with stack traces

3. **Contract inheritance**
   - Subtype contracts must strengthen (not weaken) base contracts
   - Liskov substitution principle enforcement

### Phase 3: Built-in Testing

1. **Test discovery and execution**
   - Scan for `@test` attributes
   - Generate test runner code
   - Report test results (pass/fail/skip)

2. **Test organization**
   - Support test groups and suites
   - Test filtering and selection
   - Watch mode for continuous testing

3. **Test utilities**
   - Setup/teardown support
   - Test fixtures
   - Expected failure handling

### Phase 4: Property-Based Testing

1. **Property test framework**
   - `@property` attribute recognition
   - Arbitrary value generation for built-in types
   - Configurable test case count and sizing

2. **Shrinking**
   - Implement shrinking for built-in types
   - Minimal failing case discovery
   - Shrink strategy customization

3. **Stateful testing**
   - Command-based testing API
   - State model validation
   - Command sequence generation and shrinking

### Phase 5: Static Analysis

1. **SMT solver integration**
   - Translate constraints to SMT-LIB format
   - Integrate Z3 or CVC5
   - Verify contracts statically when possible

2. **Verification levels**
   - Support none/dynamic/static/hybrid modes
   - Optimization: remove verified checks in release builds
   - Error messages for failed static verification

3. **Loop invariants**
   - Syntax for loop invariants
   - Verification condition generation
   - Integration with SMT solver

### Phase 6: Advanced Features

1. **Dependent types (limited)**
   - Array lengths in types
   - Refinement types
   - Type families for common patterns

2. **Custom generators and shrinkers**
   - Arbitrary trait for user types
   - Shrink trait for user types
   - Generator combinators

3. **Coverage-guided generation**
   - Track code coverage during property tests
   - Generate inputs that maximize coverage
   - Integration with fuzzing techniques

---

## Open Questions

1. **Constraint language expressiveness**: How rich should predicates be?
   - Simple comparisons and boolean logic (start here)
   - Quantifiers (forall, exists)
   - Recursive predicates
   - Integration with type system (generic constraints)

2. **Performance**: When to check constraints?
   - Always in debug builds
   - Optional in release builds
   - Only at module boundaries
   - User-configurable per function/type

3. **Error recovery**: What happens when a constraint is violated?
   - Panic (safe but abrupt)
   - Return Result/Option (ergonomic but verbose)
   - Type-directed error handling

4. **Constraint inference**: Can we infer constraints?
   - From usage patterns
   - From test cases
   - From property tests

5. **Integration with effects**: How do contracts interact with effects?
   - Checking IO effects for contracts
   - Pure vs effectful predicates
   - Contract checking with error handling

6. **Syntax bikeshedding**:
   - `@requires` vs `@pre` vs `requires`
   - `@ensures` vs `@post` vs `ensures`
   - Position of attributes (before vs after function signature)

---

## Conclusion

This design provides a comprehensive approach to contract programming and testing in Cadenza:

✅ **Progressive rigor**: Start simple, add contracts as needed  
✅ **Type integration**: Constraints are part of the type system  
✅ **Automatic testing**: Properties test vast input spaces  
✅ **Static verification**: Prove correctness when possible  
✅ **Clear errors**: Violations are reported with context  
✅ **Practical**: Balance theory with real-world usability  

The combination of Ada-style constrained types, Eiffel-style contracts, Rust-style testing, and bolero-style property testing provides a powerful toolkit for building robust software. The incremental approach means developers can adopt these features at their own pace, starting with simple tests and progressing to formal verification as their confidence and requirements grow.

Next steps: Begin Phase 1 implementation with basic constrained type support, then expand to contracts and testing infrastructure.
