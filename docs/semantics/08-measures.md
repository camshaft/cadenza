# Units of Measure and Dimensional Analysis

Cadenza provides first-class support for physical units and dimensional analysis. Units attach to numeric types, preventing unit confusion errors at compile time while having zero runtime overhead.

## Measure Definitions

The `measure` keyword defines a new unit of measurement.

### Syntax

```
measure <Name>                          # Base unit
measure <Name> = <Number> <OtherUnit>   # Derived unit
```

Base units define new dimensions. Derived units are defined in terms of existing units with a conversion factor.

### Semantics

When a measure is defined:

1. Create a new unit in the unit registry
2. For derived units, establish bidirectional conversion
3. Return `Unit`

Measures are nominal types for numbers - they create distinct types even for the same numeric value.

### Type

Measure definitions return `Unit`.

### Test: Base unit definition

**Input:**

```cadenza
measure meter
```

**Output:**

```repl
() : Unit
```

**Notes:** Defines `meter` as a base unit of length

### Test: Multiple base units

**Input:**

```cadenza
measure meter
measure second
measure kilogram
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
```

**Notes:** Each base unit creates an independent dimension

### Test: Derived unit

**Input:**

```cadenza
measure millimeter
measure inch = 25.4 millimeter
```

**Output:**

```repl
() : Unit
() : Unit
```

**Notes:** An inch is defined as 25.4 millimeters. This creates automatic conversions.

### Test: Chain of derived units

**Input:**

```cadenza
measure millimeter
measure centimeter = 10 millimeter
measure meter = 100 centimeter
measure kilometer = 1000 meter
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
() : Unit
```

**Notes:** Units can be defined in terms of other derived units

---

## Quantity Construction

Quantities are numeric values with attached units. They're created by applying a unit name to a number.

### Syntax

```
<Number><Unit>         # Postfix: 10meter
<Unit> <Number>         # Prefix: meter 10
```

Both forms are equivalent and create a quantity.

### Semantics

Applying a unit to a number creates a quantity value that tracks both the numeric value and the unit. The unit becomes part of the type.

### Type

`Quantity T Unit` where `Unit` is the unit type and `T` is the underlying numeric type (Integer or Float).

### Test: Quantity from integer

**Input:**

```cadenza
measure meter
100meter
```

**Output:**

```repl
() : Unit
100meter : Quantity Integer meter
```

**Notes:** The unit becomes part of the type

### Test: Quantity from float

**Input:**

```cadenza
measure meter
3.5meter
```

**Output:**

```repl
() : Unit
3.5meter : Quantity Float meter
```

### Test: Prefix unit application

**Input:**

```cadenza
measure second
second 10
```

**Output:**

```repl
() : Unit
10second : Quantity Integer second
```

**Notes:** Prefix form: `second 10` equivalent to `10second`

### Test: Quantity from expression

**Input:**

```cadenza
measure meter
let width = 10
let height = 20
meter (width * height)
```

**Output:**

```repl
() : Unit
() : Unit
200 meter : Quantity Integer meter
```

**Notes:** Can apply units to computed values

---

## Dimensional Arithmetic

Arithmetic operations on quantities follow dimensional analysis rules. The result's dimension is determined by the operation.

### Addition and Subtraction

Adding or subtracting quantities requires matching dimensions. The result has the same dimension.

### Test: Add same units

**Input:**

```cadenza
measure meter
100meter + 50meter
```

**Output:**

```repl
() : Unit
150meter : Quantity Integer meter
```

### Test: Add compatible derived units

**Input:**

```cadenza
measure millimeter
measure meter = 1000 millimeter
500millimeter + 1meter
```

**Output:**

```repl
() : Unit
() : Unit
1500millimeter : Quantity Integer millimeter
```

**Notes:** Automatic conversion: 1 meter = 1000 millimeter, so result is 500 + 1000 = 1500 millimeter

### Test: ERROR - Add incompatible dimensions

**Input:**

```cadenza
measure meter
measure second
100meter + 10second
```

**Output:**

```
error: incompatible dimensions
 --> test.cdz:3:1
  |
3 | 100meter + 10second
  | ^^^^^^^^^^^^^^^^^^^ cannot add `meter` and `second`
  |
  = note: left side has dimension: meter
  = note: right side has dimension: second
  = help: addition requires matching dimensions
```

### Multiplication

Multiplying quantities creates derived dimensions.

### Test: Multiply quantities

**Input:**

```cadenza
measure meter
measure second
let distance = 100meter
let time = 10second
distance * time
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
() : Unit
1000meter·second : Quantity Integer meter·second
```

**Notes:** Multiplication creates a compound dimension

### Test: Multiply by scalar

**Input:**

```cadenza
measure meter
100meter * 2
```

**Output:**

```repl
() : Unit
200meter : Quantity Integer meter
```

**Notes:** Multiplying by a dimensionless number preserves the dimension

### Division

Dividing quantities creates ratio dimensions. Dividing by the same dimension yields a dimensionless number.

### Test: Divide quantities (different dimensions)

**Input:**

```cadenza
measure meter
measure second
let distance = 100meter
let time = 10second
distance / time
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
() : Unit
10meter/second : Quantity Integer meter/second
```

**Notes:** Division creates a velocity dimension

### Test: Divide quantities (same dimension)

**Input:**

```cadenza
measure meter
200meter / 100meter
```

**Output:**

```repl
() : Unit
2 : Integer
```

**Notes:** Dividing by the same dimension cancels out, yielding a dimensionless number

### Test: Divide by scalar

**Input:**

```cadenza
measure meter
100meter / 2
```

**Output:**

```repl
() : Unit
50meter : Quantity Integer meter
```

**Notes:** Dividing by a dimensionless number preserves the dimension

---

## Compound Dimensions

Operations on quantities can create complex dimensions with multiple base units.

### Test: Acceleration dimension

**Input:**

```cadenza
measure meter
measure second
let velocity = 100meter / 10second
velocity / 5second
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
2 meter/second² : Quantity Integer meter/second²
```

**Notes:** Acceleration = velocity / time = (meter/second) / second = meter/second²

### Test: Energy dimension

**Input:**

```cadenza
measure kilogram
measure meter
measure second
let mass = 5kilogram
let distance = 10meter
let time = 2second
let force = mass * distance / (time * time)
force
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
() : Unit
() : Unit
12.5kilogram·meter/second² : Quantity Float kilogram·meter/second²
```

**Notes:** Force = mass × acceleration = kg·m/s²

### Test: Dimension simplification

**Input:**

```cadenza
measure meter
let area = 10meter * 5meter
let length = 2meter
area / length
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
25 meter : Quantity Integer meter
```

**Notes:** (meter²) / meter = meter (dimension simplifies)

---

## Type Safety with Units

The type system prevents mixing incompatible units at compile time.

### Test: Type error on incompatible addition

**Input:**

```cadenza
measure meter
measure foot
100meter + 10foot
```

**Output:**

```
error: incompatible unit dimensions
 --> test.cdz:3:1
  |
3 | 100meter + 10foot
  | ^^^^^^^^^^^^^^^^^ cannot add `meter` and `foot`
  |
  = note: left side has dimension: meter
  = note: right side has dimension: foot
  = help: addition requires compatible dimensions
```

**Notes:** Without a defined conversion, different units are incompatible

### Test: Comparison requires matching dimensions

**Input:**

```cadenza
measure meter
measure kilogram
100meter > 50kilogram
```

**Output:**

```
error: incompatible dimensions in comparison
 --> test.cdz:3:1
  |
3 | 100meter > 50kilogram
  | ^^^^^^^^^^^^^^^^^^^^^ cannot compare `meter` and `kilogram`
  |
  = note: left side has dimension: meter
  = note: right side has dimension: kilogram
  = help: comparison requires compatible dimensions
```

---

## Automatic Unit Conversion

When derived units with established conversions are used in operations, automatic conversion occurs.

### Test: Automatic conversion in arithmetic

**Input:**

```cadenza
measure millimeter
measure inch = 25.4 millimeter
let metric = 500millimeter
let imperial = 2inch
metric + imperial
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
() : Unit
550.8millimeter : Quantity Float millimeter
```

**Notes:** 2 inches converted to 50.8 millimeter, then added

### Test: Conversion in comparison

**Input:**

```cadenza
measure millimeter
measure inch = 25.4 millimeter
1inch > 20millimeter
```

**Output:**

```repl
() : Unit
() : Unit
true : Bool
```

**Notes:** 1 inch = 25.4 millimeter > 20 millimeter

---

## Dimensionless Quantities

Operations that cancel out dimensions produce plain numbers.

### Test: Ratio of same dimension

**Input:**

```cadenza
measure meter
let total = 100meter
let part = 25meter
part / total
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
1 / 4 : Rational
```

**Notes:** Dividing same dimensions yields a dimensionless ratio

### Test: Using dimensionless result

**Input:**

```cadenza
measure meter
let circumference = 10.0meter
let diameter = 3.18meter
let pi_approx = circumference / diameter
pi_approx * 2
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
6.29 : Float
```

**Notes:** The dimensionless ratio can be used in further calculations

---

## Unit Prefixes

Standard SI prefixes for creating scaled units.

### Test: SI prefixes

**Input:**

```cadenza
@si kilo
measure meter
let distance = 5kilometer  # Auto-generated from kilo prefix
distance
```

**Output:**

```repl
() : Unit
5000 meter : Quantity Integer meter
```

**Notes:** Prefixes like kilo-, milli-, micro- automatically generate derived units

---

## Querying Units at Compile Time

Units can be queried and inspected at compile time for metaprogramming.

### Test: Get unit from quantity

**Input:**

```cadenza
measure meter
let distance = 100meter
typeof distance
```

**Output:**

```repl
() : Unit
() : Unit
Quantity Integer meter : Type
```

**Notes:** The type includes the unit information

### Test: Extract dimension

**Input:**

```cadenza
measure meter
measure second
let velocity = 100meter / 10second
Quantity.to_string (typeof velocity)
```

**Output:**

```repl
() : Unit
() : Unit
() : Unit
"meter/second" : String
```

**Notes:** Can extract the dimension as a string representation

---

## Compiler Queries

Units and dimensional analysis require:

- `eval(measure)` - Register unit in registry, return Unit
- `eval(quantity)` - Attach unit to number, create quantity value
- `eval(unit_arithmetic)` - Check dimension compatibility, compute result dimension
- `typeof(quantity)` - Return Quantity<Unit, NumericType>
- `check_dimension_compat(unit1, unit2, op)` - Validate dimensions for operation
- `derive_dimension(unit1, unit2, op)` - Compute result dimension from operation
- `find_conversion(unit1, unit2)` - Look up conversion factor between units
- `simplify_dimension(dim)` - Simplify compound dimensions (meter²/meter → meter)

## Implementation Notes

- Units are nominal types attached to numeric values
- Dimensions tracked at compile time, erased in generated code (zero runtime cost)
- Conversion factors stored in unit registry
- Automatic conversion when compatible units are mixed
- Dimension checking happens during type checking
- Quantities are represented as the underlying number in generated code
- Unit arithmetic follows standard dimensional analysis rules
- Derived units create conversion graphs (breadth-first search for conversions)
