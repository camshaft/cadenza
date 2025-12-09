# Cadenza Programming Language

Cadenza is a functional programming language with first-class support for units of measure and dimensional analysis.

## Key Features

1. **Units of Measure**: Built-in support for physical units and automatic dimensional analysis
   - Define base units: `measure meter`
   - Define derived units: `measure kilometer = meter 1000`
   - Use in expressions: `10meter`, `5kilometer`
   - Automatic unit checking: prevents adding incompatible units

2. **Functional Programming**: Functions are first-class values
   - Define functions: `fn square x = x * x`
   - Higher-order functions supported

3. **Type Safety**: Static type checking with type inference
   - Types are inferred automatically
   - Compile-time error detection
   - No runtime type errors

4. **Dimensional Analysis**: Compile-time verification of physical dimensions
   - Prevents dimension mismatches: `10meter + 5second` is an error
   - Tracks derived dimensions: `distance / time` gives velocity
   - Unit conversions are explicit

5. **Interactive REPL**: Immediate feedback and exploration
   - Evaluate expressions interactively
   - Define and test functions
   - Load and save scripts

## Primary Use Cases

- **Quick Calculations**: Fast computation with proper unit handling
- **3D Modeling**: Define models in code (like OpenSCAD)
- **Algorithmic Music**: Compose music with code
- **Interactive Books**: Drive simulations and visualizations
- **Scientific Computing**: Calculations with dimensional analysis

## Example Code

```cadenza
# Define units
measure meter
measure second

# Use units in calculations
let distance = 100meter
let time = 10second
let speed = distance / time  # Automatically: meter/second

# Define functions
fn kinetic_energy mass velocity =
    0.5 * mass * velocity * velocity

# Use functions
let energy = kinetic_energy 1000kilogram 20meter/second
```

## Syntax Basics

- Variables: `let name = value`
- Functions: `fn name param = body`
- Comments: `# This is a comment`
- Pipeline: `value |> function`
- Operators: `+`, `-`, `*`, `/`, `==`, `!=`, `<`, `<=`, `>`, `>=`

For more examples, use the `eval` tool to try expressions!
