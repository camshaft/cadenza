# Literals

Literals denote values directly. Each has a statically determined type (type-system.md §"Every
Expression Has A Static Type") and a canonical value form (contracts/deterministic-value-form.md).

Cases are shown in the conventional display of the canonical representation; because display is
decoupled from representation (defaults/code-shape.md), the same case in the homoiconic display
denotes the same representation and executes identically. Output is written as `value : Type`.

## Integers

### Case: a decimal integer literal

**Input:**

```cadenza
42
```

**Output:**

```
42 : Int64
```

### Case: an integer literal with digit separators

**Input:**

```cadenza
1_000_000
```

**Output:**

```
1000000 : Int64
```

## Floating-point

### Case: a floating-point literal

**Input:**

```cadenza
3.5
```

**Output:**

```
3.5 : Float64
```

### Case: negative zero is distinct in the canonical value form

**Input:**

```cadenza
-0.0
```

**Output:**

```
-0.0 : Float64
```

## Booleans

### Case: the boolean literals

**Input:**

```cadenza
true
```

**Output:**

```
true : Bool
```

## Strings

### Case: a string literal

**Input:**

```cadenza
"hello"
```

**Output:**

```
"hello" : String
```

### Case: a string literal is normalized to the canonical text form

**Input:**

```cadenza
"café"
```

**Output:**

```
"café" : String
```

**Notes:** the string is stored in the pinned text normalization form
(defaults/hashing-and-encoding.md), so two literals differing only in normalization are one value.
