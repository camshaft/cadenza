# Numeric Model

These cases witness the behavioral requirements of numeric-model.md: no implicit promotion between
numeric types, defined overflow, exact arithmetic, and deterministic floating-point. Output is
written as `value : Type`, or as a diagnostic code for a rejected program (defaults/diagnostics-schema.md).

## No implicit promotion

### Case: arithmetic within one integer type

**Input:**

```cadenza
2 + 3
```

**Output:**

```
5 : Int64
```

### Case: mixing two numeric types without an explicit conversion is rejected

**Input:**

```cadenza
1 + 2.0
```

**Output:**

```
error CDZ0301: numeric types do not implicitly promote; convert one operand explicitly
```

**Notes:** witnesses numeric-model.md §"Numeric Types Do Not Silently Promote" — there is no implicit
widening from `Int64` to `Float64`.

### Case: an explicit conversion makes the operation well-typed

**Input:**

```cadenza
Float64.of_int(1) + 2.0
```

**Output:**

```
3.0 : Float64
```

## Overflow

### Case: overflow of the default integer traps deterministically

**Input:**

```cadenza
Int64.max + 1
```

**Output:**

```
trap: integer overflow
```

**Notes:** witnesses numeric-model.md §"Overflow Is Defined" with the checked-and-trapping default
pinned in defaults/numeric-model.md; a program that wants wrapping uses the distinct wrapping type.

### Case: wrapping arithmetic uses the distinct wrapping type

**Input:**

```cadenza
Wrapping64.max +% 1
```

**Output:**

```
-9223372036854775808 : Wrapping64
```

## Exactness

### Case: exact division of big-integers is exact

**Input:**

```cadenza
Rational.of(1, 3) + Rational.of(1, 6)
```

**Output:**

```
1/2 : Rational
```

**Notes:** witnesses numeric-model.md §"Exact Arithmetic Is Exact"; the result is normalized to lowest
terms per defaults/numeric-model.md.

## Deterministic floating-point

### Case: floating-point uses the fixed rounding mode

**Input:**

```cadenza
0.1 + 0.2
```

**Output:**

```
0.30000000000000004 : Float64
```

**Notes:** the result is the round-to-nearest-even sum under the pinned deterministic float mode
(contracts/determinism-and-fuel.md §"Floating-Point Emission Is Determinism-Constrained"); it is
byte-identical on every conforming runtime.
