# Quick Calculation and REPL Environment

## Vision

Enable Cadenza as a powerful calculator and interactive computing environment, emphasizing dimensional analysis for real-world calculations. This is the most immediate and accessible use case, requiring minimal infrastructure while showcasing Cadenza's unique strengths.

Key capabilities:
1. **Dimensional Analysis**: Automatic unit handling and conversion
2. **Interactive REPL**: Immediate feedback for calculations
3. **Scripting**: Save and reuse calculation scripts
4. **Type Safety**: Catch errors before computation
5. **Exploration**: Discover functions and capabilities interactively

## Goals

### Primary Goals

1. **Fast Interactive Calculations**
   - Sub-second startup time
   - Immediate evaluation of expressions
   - History and recall of previous results
   - Tab completion for functions and units
   - Spotlight-style overlay interface (customizable key combo, Esc to hide, persistent sessions)

2. **Dimensional Analysis Excellence**
   - Automatic unit inference and checking
   - Natural unit conversions using pipeline syntax: `100meter |> to feet`
   - Support common units: length, mass, time, energy, data, etc.
   - Derived units computed automatically
   - Type coercion safe with rational numbers (no precision loss)

3. **Data Exploration**
   - Import datasets (CSV, Parquet, etc.)
   - Query and filter data
   - Generate graphs and visualizations
   - Interactive parameters with inline widgets (`@param` macro)
   - Real-time parameter adjustment

4. **Discoverability**
   - List available functions
   - Show function signatures and documentation
   - Example-driven help system
   - Suggest related functions

5. **Scripting Support**
   - Save calculations to files
   - Load and execute scripts
   - Share calculations with others
   - Version control friendly
   - Switch between scratch spaces/sessions

### Secondary Goals

- **Plotting**: Simple graph generation for functions
- **Formatting**: Pretty-print large numbers, scientific notation
- **Constants**: Built-in physical constants (π, e, c, G, etc.)
- **Precision**: Arbitrary precision arithmetic
- **History Search**: Search previous commands and results

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                        REPL                                      │
│                                                                  │
│  > 100 meters/second to mph                                     │
│  223.694 miles/hour                                             │
│                                                                  │
│  > let acceleration = 9.8 meters/second^2                       │
│  9.8 m/s²                                                        │
│                                                                  │
│  > let time = 5 seconds                                         │
│  5.0 s                                                           │
│                                                                  │
│  > let distance = 0.5 * acceleration * time^2                   │
│  122.5 m                                                         │
│                                                                  │
│  > :help sqrt                                                    │
│  sqrt : Float -> Float                                          │
│  Returns the square root of a number                            │
│                                                                  │
│  > :type distance                                               │
│  Quantity<Float, meter>                                         │
└─────────────────────────────────────────────────────────────────┘
```

## Example Usage

### Basic Calculations

```cadenza
# Simple arithmetic
> 2 + 2
4

> 1 / 3
0.333... (or Rational 1/3 with rational numbers)

# With units
> 50 kilometers + 500 meters
50.5 km

> 100 megabytes / 2 seconds
50.0 MB/s

# Unit conversions
> 72 fahrenheit to celsius
22.222... °C

> 1 gigabyte to megabits
8000.0 Mb
```

### Physics Calculations

```cadenza
# Kinetic energy
> let mass = 1000 kilograms
> let velocity = 20 meters/second
> let kinetic_energy = 0.5 * mass * velocity^2
200000.0 J (joules)

# Frequency and wavelength
> let frequency = 440 hertz  # A4 note
> let speed_of_sound = 343 meters/second
> let wavelength = speed_of_sound / frequency
0.78 m
```

### Data Rate Calculations

```cadenza
# Network bandwidth
> 459174794 bytes/second to gigabits/second
3.673 Gbps

# File transfer time
> let file_size = 4.7 gigabytes
> let bandwidth = 100 megabits/second
> let transfer_time = file_size / bandwidth
376.0 s  # or 6.27 minutes

# Data center calculation
> let servers = 1000
> let power_per_server = 500 watts
> let total_power = servers * power_per_server
> total_power to kilowatts
500.0 kW
```

### Reusable Scripts

```cadenza
# save as projectile.cdz
fn projectile_range velocity angle gravity =
  let angle_rad = angle * (pi / 180)
  let range = (velocity^2 * sin(2 * angle_rad)) / gravity
  range

# In REPL
> :load projectile.cdz
> projectile_range 50m/s 45° 9.8m/s²
254.84 m
```

### Exploration and Help

```cadenza
# List functions
> :functions
abs, sin, cos, tan, sqrt, log, exp, ...

# Get type signature
> :type sin
sin : Float -> Float

# Get documentation
> :help sin
sin : Float -> Float
Returns the sine of an angle in radians

Example:
  sin (pi / 4)  # Returns 0.707...

# Find related functions
> :search trigonometry
sin, cos, tan, asin, acos, atan, atan2
```

## Required Language Features

### Already Available ✅

1. **Arithmetic operators** - Basic calculations
2. **Functions** - Define reusable calculations
3. **Let bindings** - Store intermediate results
4. **Units of measure** - Dimensional analysis
5. **Type system** - Unit and type safety
6. **Evaluator** - Interactive execution

### In Progress 🚧

1. **Type inference** - Automatic type checking (Phase 2)
2. **REPL infrastructure** - Command handling, history

### Needed for REPL 🔨

1. **REPL Commands**
   - :help, :type, :load, :save, :quit
   - :functions, :search
   - History navigation (up/down arrows)
   - Tab completion

2. **Unit Conversion Syntax**
   - "to" operator: `100 meters to feet`
   - Or function: `convert 100meters feet`
   - List compatible units: `:units length`

3. **Pretty Printing**
   - Format large numbers with commas
   - Scientific notation for very large/small
   - Show units in readable form
   - Configurable precision

4. **Common Constants**
   - Mathematical: π (pi), e, φ (phi)
   - Physical: c (speed of light), G (gravitational constant), h (Planck)
   - Define as standard library

5. **Exponentiation Operator**
   - Syntax: `x^2` or `x ** 2`
   - Type-safe with units (e.g., `meter^2` is area)

6. **History Management**
   - Store command history
   - Recall with up/down arrows
   - Search history with Ctrl+R
   - Access previous result with `_` or `ans`

### Nice to Have 🎁

1. **Symbolic Math**
   - Keep expressions symbolic until needed
   - Simplify algebraic expressions
   - Solve equations symbolically

2. **Plotting**
   - Plot functions: `:plot sin(x) 0 2pi`
   - 2D graphs in terminal or browser

3. **Arbitrary Precision**
   - Compute to arbitrary decimal places
   - Useful for precise engineering calculations

4. **Unit System Customization**
   - Define custom units
   - Create unit aliases
   - Set preferred display units

5. **Session Management**
   - Save/load REPL sessions
   - Export history to script
   - Replay sessions

## Implementation Challenges

### 1. Startup Time

**Challenge**: REPL should start instantly (<100ms).

**Mitigation**:
- Minimal dependencies
- Lazy loading of libraries
- Precompiled standard library
- Optimize interpreter initialization

### 2. Error Messages in Interactive Context

**Challenge**: Errors should be helpful but not overwhelming in REPL.

**Mitigation**:
- Concise error messages by default
- `:explain` command for detailed errors
- Suggest fixes when possible
- Show type mismatches clearly

### 3. Unit Conversion Syntax

**Challenge**: Natural syntax for unit conversions.

**Solution**: Use pipeline syntax with `to` macro. It's intuitive and reads naturally.

```cadenza
# Pipeline with to macro (recommended)
100 meter |> to feet

# Also supports infix for readability
100 meters to feet
```

Type coercion is safe with rational numbers (no precision loss), so implicit conversion can be allowed.

### 4. History and State Management

**Challenge**: REPL maintains state across commands.

**Considerations**:
- Variables persist between commands
- Functions remain defined
- History should be searchable
- Clear/reset mechanism needed

**Solution**: Store environment state, provide `:reset` command to start fresh.

### 5. Autocomplete Implementation

**Challenge**: Tab completion for functions, variables, units.

**Considerations**:
- Complete function names
- Complete variable names in scope
- Complete unit names
- Complete REPL commands

**Solution**: Build completion tree from environment, use LSP infrastructure for suggestions.

## Success Criteria

This REPL environment would be successful if it achieves:

### Core Functionality
- ✅ Evaluate expressions interactively
- ✅ Dimensional analysis with unit conversions
- ✅ Define and use variables
- ✅ Define and call functions
- ✅ Load and save scripts
- ✅ Built-in help system

### Performance
- ✅ Startup time <100ms
- ✅ Expression evaluation <10ms for typical calculations
- ✅ Responsive autocomplete (<50ms)

### User Experience
- ✅ Intuitive unit conversion syntax
- ✅ Clear, helpful error messages
- ✅ Easy discovery of functions
- ✅ Comfortable editing (readline-like)
- ✅ Persistent history

### Accuracy
- ✅ Correct dimensional analysis
- ✅ Proper unit conversions
- ✅ Numerical precision appropriate for use case
- ✅ Handles edge cases (division by zero, overflow)

## Next Steps

### Phase 1: Basic REPL
1. Implement readline-based input
2. Parse and evaluate expressions
3. Display results with units
4. Store command history
5. Basic error handling

### Phase 2: Unit Conversions
1. Implement "to" operator
2. Add common unit definitions (SI, imperial)
3. Support derived units
4. Unit compatibility checking
5. Pretty printing for units

### Phase 3: Enhanced Interaction
1. Tab completion for names
2. Multi-line input support
3. Syntax highlighting
4. Help system (:help, :type commands)
5. Load/save scripts

### Phase 4: Mathematical Functions
1. Trigonometric functions (sin, cos, tan)
2. Exponential and logarithmic (exp, log, ln)
3. Constants (pi, e)
4. Statistical functions (mean, stddev)
5. Physical constants

### Phase 5: Advanced Features
1. Command history search (Ctrl+R)
2. Precision configuration
3. Output formatting options
4. Simple plotting capability
5. Session save/restore

## Relationship to Existing Tools

### GNU bc
Command-line calculator.
- **Cadenza advantage**: Units, type safety, modern syntax, functions

### Python REPL
Interactive Python interpreter.
- **Cadenza advantage**: Dimensional analysis, faster for calculations, specialized for math

### Wolfram Alpha
Computational knowledge engine.
- **Cadenza advantage**: Offline, open source, programmable, integrates with other Cadenza uses

### F# Interactive (fsi)
Interactive F# environment.
- **Cadenza advantage**: Simpler syntax for calculations, built-in units

### Julia REPL
Interactive Julia for scientific computing.
- **Cadenza advantage**: Dimensional analysis integrated, simpler for casual use

## Conclusion

The REPL/calculator use case is Cadenza's most accessible entry point:

**Strengths**:
- **Immediate value**: Useful on day one
- **Low barrier**: No complex setup required
- **Showcases uniqueness**: Dimensional analysis shines here
- **Foundation**: REPL benefits all other use cases

**Implementation Priority**: HIGH
- Requires minimal infrastructure
- Benefits all other environments
- Provides immediate user value
- Validates core language design

**Current Status**: Most features available now
- Basic evaluation works
- Unit system exists
- Need REPL interface wrapper
- Need help system

**Timeline**: Could be production-ready in Phase 2 (alongside type system completion).

This use case is the perfect starting point for Cadenza adoption and demonstrates its practical value immediately.
