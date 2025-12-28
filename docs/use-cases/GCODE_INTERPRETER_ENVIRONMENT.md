# GCode Interpreter Environment for 3D Printing

## Vision

Enable Cadenza to power a 3D printer firmware similar to Klipper's Python component. The system would:

1. **Register GCode handlers** as Cadenza functions in an environment
2. **Stream GCode commands** to the system for compilation and execution
3. **Configure handlers** via a Cadenza module (strongly-typed configuration)
4. **Precompile** as much as possible to native code while maintaining a runtime
5. **Optionally compile GCode** to Cadenza modules for type-checking and bytecode generation

This approach provides type safety, performance, and validation before print jobs begin, while leveraging Cadenza's dimensional analysis for proper unit handling in motion control.

## Goals

### Primary Goals

1. **Type-Safe Configuration**
   - Configuration files written in Cadenza, not YAML/JSON
   - Compile-time validation of printer parameters
   - Dimensional analysis ensures unit correctness (e.g., mm/s for velocity)
   - IDE support with autocomplete and inline errors

2. **Extensible Handler System**
   - Register GCode command handlers as Cadenza functions
   - Handlers receive parsed command parameters
   - Type-checked handler signatures
   - Custom handlers for non-standard GCode

3. **Ahead-of-Time Compilation**
   - Precompile configuration and handlers to native code
   - Ship Cadenza parser/interpreter as runtime dependency
   - Fast startup times for firmware
   - Optimize common paths

4. **GCode Validation**
   - Optionally parse GCode files as Cadenza modules
   - Type-check commands against handler signatures
   - Validate parameters against configuration constraints
   - Catch errors before starting print jobs

5. **Performance**
   - Native code for performance-critical paths
   - Interpreted execution for GCode commands (acceptable overhead)
   - Efficient bytecode representation
   - Minimal runtime overhead

### Secondary Goals

- **Real-time Constraints**: Handle timing requirements for motion control
- **Streaming Execution**: Process GCode incrementally without loading entire files
- **Error Recovery**: Graceful handling of invalid commands during printing
- **Debugging Tools**: Inspect handler state, trace execution, visualize toolpaths
- **Simulation Mode**: Dry-run print jobs without hardware

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                     Printer Configuration                        │
│                     (config.cdz module)                          │
│                                                                  │
│  - Define printer parameters (dimensions, speeds, units)        │
│  - Register GCode handlers as Cadenza functions                 │
│  - Type-checked at load time                                    │
└─────────────────────────────────────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                  Cadenza Compilation Phase                       │
│                                                                  │
│  1. Parse and evaluate config.cdz                               │
│  2. Type-check handler signatures                               │
│  3. Compile handlers to native code (via Rust backend)          │
│  4. Generate handler registry                                   │
└─────────────────────────────────────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                     Runtime Environment                          │
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │               GCode Command Stream                       │   │
│  │  G1 X100 Y50 F3000  (streamed line by line)            │   │
│  └──────────────────────────────────────────────────────────┘   │
│                               │                                  │
│                               ▼                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │            GCode Parser (Cadenza Runtime)                │   │
│  │  - Tokenize command and arguments                        │   │
│  │  - Build AST or direct invocation                        │   │
│  └──────────────────────────────────────────────────────────┘   │
│                               │                                  │
│                               ▼                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │            Handler Dispatch                              │   │
│  │  - Lookup handler by command code                        │   │
│  │  - Type-check arguments against signature                │   │
│  │  - Invoke native/interpreted handler                     │   │
│  └──────────────────────────────────────────────────────────┘   │
│                               │                                  │
│                               ▼                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │            Handler Execution                             │   │
│  │  - Access configuration parameters                       │   │
│  │  - Validate against constraints                          │   │
│  │  - Execute motion control / hardware commands            │   │
│  │  - Update printer state                                  │   │
│  └──────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

### Optional: GCode as Cadenza Module

```
┌─────────────────────────────────────────────────────────────────┐
│                     GCode File                                   │
│                     (print.gcode)                                │
└─────────────────────────────────────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│              GCode-to-Cadenza Transpiler                         │
│                                                                  │
│  - Parse GCode syntax                                            │
│  - Map commands to handler function calls                        │
│  - Generate Cadenza module (print.cdz)                           │
└─────────────────────────────────────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                  Cadenza Type-Checking                           │
│                                                                  │
│  - Verify all commands are valid handlers                       │
│  - Type-check arguments                                          │
│  - Validate parameter ranges against config                      │
│  - Generate bytecode for efficient interpretation                │
└─────────────────────────────────────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                  Bytecode Execution                              │
│                                                                  │
│  - Interpret type-checked bytecode                               │
│  - No runtime parsing overhead                                   │
│  - Validated before print starts                                 │
└─────────────────────────────────────────────────────────────────┘
```

## Example Usage

### Configuration File (config.cdz)

```cadenza
# Printer physical parameters
measure millimeter
measure millimeter_per_second

let printer_config = {
  # Physical dimensions
  bed_x = 250millimeter,
  bed_y = 210millimeter,
  bed_z = 200millimeter,
  
  # Movement limits
  max_velocity = 300millimeter_per_second,
  max_acceleration = 3000millimeter_per_second,  # Would need derived unit: mm/s²
  
  # Hardware setup
  steps_per_mm_x = 80.0,
  steps_per_mm_y = 80.0,
  steps_per_mm_z = 400.0,
  steps_per_mm_e = 95.0,
}

# Define state type for printer
let printer_state = {
  position_x = 0millimeter,
  position_y = 0millimeter,
  position_z = 0millimeter,
  feedrate = 0millimeter_per_second,
}

# Register GCode handler for G1 (linear move)
fn handle_g1 state x y z f =
  # Validate coordinates within bounds
  assert (x <= printer_config.bed_x) "X coordinate exceeds bed size"
  assert (y <= printer_config.bed_y) "Y coordinate exceeds bed size"
  assert (z <= printer_config.bed_z) "Z coordinate exceeds bed size"
  
  # Validate feedrate
  let feedrate = if f == nil then state.feedrate else f
  assert (feedrate <= printer_config.max_velocity) "Feedrate exceeds maximum"
  
  # Calculate steps
  let steps_x = (x - state.position_x) * printer_config.steps_per_mm_x
  let steps_y = (y - state.position_y) * printer_config.steps_per_mm_y
  let steps_z = (z - state.position_z) * printer_config.steps_per_mm_z
  
  # Execute motion (calls into hardware abstraction layer)
  move_linear steps_x steps_y steps_z feedrate
  
  # Return updated state
  { ...state, position_x = x, position_y = y, position_z = z, feedrate = feedrate }

# Register GCode handler for G28 (home)
fn handle_g28 state =
  # Home all axes
  home_all_axes
  
  # Return state with zeroed position
  { ...state, position_x = 0millimeter, position_y = 0millimeter, position_z = 0millimeter }

# Register GCode handler for M104 (set extruder temperature)
fn handle_m104 state s =
  assert (s >= 0 && s <= 300) "Temperature out of range"
  set_extruder_temp s
  state

# Register handlers in environment
register_gcode_handler "G1" handle_g1
register_gcode_handler "G28" handle_g28
register_gcode_handler "M104" handle_m104
```

### GCode Processing

**Direct interpretation:**
```gcode
G28              ; Home all axes
M104 S200        ; Set extruder temp to 200°C
G1 X100 Y50 F3000  ; Move to (100, 50) at 3000 mm/min
```

**Or, transpile to Cadenza:**
```cadenza
# Generated from print.gcode
handle_g28 printer_state
handle_m104 printer_state 200
handle_g1 printer_state 100millimeter 50millimeter nil (3000millimeter_per_second / 60)
```

The Cadenza version can be:
1. Type-checked against handler signatures
2. Validated against config constraints
3. Compiled to efficient bytecode
4. Executed with minimal overhead

## Required Language Features

### Already Available ✅

1. **Functions** - Define handlers as first-class functions
2. **Let bindings** - Define configuration parameters
3. **Records** - Structured configuration and state
4. **Field access** - Access config parameters (`printer_config.bed_x`)
5. **Arithmetic operators** - Calculate steps, validate ranges
6. **Comparison operators** - Validate constraints
7. **Block expressions** - Multi-statement handlers
8. **Macros** - Code generation for handler registration
9. **Units of measure** - Dimensional analysis (millimeter, mm/s, etc.)
10. **Tree-walk interpreter** - Runtime execution

### In Progress 🚧

1. **Type system (Phase 2)**
   - Hindley-Milner inference for handler signatures
   - Type-check GCode arguments against handlers
   - Dimensional type checking for units
   - Required for safe parameter validation

2. **Module system (Phase 3)**
   - Load configuration as module
   - Import handler definitions
   - Namespace management
   - Required for organized code structure

### Needed for GCode Use Case 🔨

1. **Assertions**
   - Runtime validation of parameters
   - Rich error messages showing actual vs expected
   - Already planned in STATUS.md

2. **String interpolation**
   - Error messages with embedded values
   - Logging and debugging
   - Already planned in STATUS.md (requires trait system)

3. **Record merging / spread operator**
   - Update state immutably: `{ ...state, position_x = new_x }`
   - Already planned in STATUS.md

4. **Optional values / nil handling**
   - Some GCode parameters are optional (e.g., F parameter)
   - Need explicit optional type or nil semantics
   - Current: Need to design optional value handling

5. **Effect system (Phase 4)**
   - Hardware IO effects (motor control, temperature sensors)
   - Logging effects
   - Configuration context
   - Already planned in COMPILER_ARCHITECTURE.md

6. **Foreign Function Interface (FFI)**
   - Call into Rust/C for hardware control
   - Interface with existing firmware libraries
   - Open question in COMPILER_ARCHITECTURE.md

7. **Error handling**
   - Graceful error recovery during prints
   - Resume from errors if possible
   - Current: Need Result/Either types

8. **Bytecode compilation**
   - Efficient representation for GCode modules
   - Fast interpretation
   - Part of Phase 5 (Code Generation)

### Nice to Have 🎁

1. **Pattern matching**
   - Destructure command parameters
   - Handle different parameter combinations
   - Already planned in STATUS.md

2. **Enum types**
   - State machine for printer (Idle, Printing, Paused, Error)
   - Command types
   - Already planned in STATUS.md

3. **Trait system (Phase 4)**
   - Generic numeric operations
   - String conversion for logging
   - Already planned in COMPILER_ARCHITECTURE.md

4. **Streaming evaluation**
   - Process GCode incrementally
   - Don't require entire file in memory
   - New requirement

5. **Just-in-time compilation**
   - Compile hot loops in GCode to native code
   - Optimize repeated patterns
   - Beyond current scope (Phase 5+)

## Implementation Challenges

### 1. Runtime Dependencies

**Challenge**: The system needs the Cadenza parser and interpreter as runtime dependencies to process streamed GCode commands.

**Considerations**:
- Parser/interpreter adds to firmware size
- Startup time for loading and initializing runtime
- Memory footprint during execution
- Need to ship entire language runtime with printer firmware

**Mitigation**:
- Precompile configuration and handlers AOT to native code
- Only interpret GCode commands (already parsed, simpler)
- Consider embedded-friendly runtime subset
- Benchmark memory usage and optimize

### 2. Real-Time Constraints

**Challenge**: 3D printing requires predictable timing for motion control. Steps must be generated at precise intervals.

**Considerations**:
- Garbage collection pauses unacceptable
- Handler execution time must be bounded
- Interpreter overhead adds latency
- May need to meet microsecond-level deadlines

**Mitigation**:
- Compile critical paths (handlers) to native code
- Use arena allocation or region-based memory
- Pre-validate GCode to avoid runtime errors
- Separate interpretation from time-critical execution (buffering)
- Consider dual-layer: high-level planning (Cadenza) + low-level control (native)

### 3. Streaming vs Batch Processing

**Challenge**: GCode is typically streamed line-by-line, but compiling to modules requires the full file.

**Considerations**:
- Streaming: Lower memory, immediate feedback, no validation
- Batch (module): Full validation, type-checking, optimization
- Hybrid approach needed

**Solutions**:
- **Online mode**: Stream and interpret GCode directly (current Klipper model)
- **Offline mode**: Preprocess GCode file to Cadenza module, validate, then execute
- Support both modes depending on use case
- Offline validation before starting long prints

### 4. Error Handling During Prints

**Challenge**: Errors during printing require careful handling to avoid damaging hardware or ruining prints.

**Considerations**:
- Some errors are fatal (overheating, hardware fault)
- Some errors can be recovered (out of bounds, correctable parameter)
- Need to safely stop motors, heaters, etc.
- Preserve state for potential recovery

**Solutions**:
- Effect system for resource management (Phase 4)
- Explicit error types (Result<T, E>)
- Handler return values indicate success/failure
- Cleanup handlers (like Rust's Drop trait)
- Safe-stop mechanism on panic

### 5. FFI and Hardware Integration

**Challenge**: Handlers need to call low-level hardware control functions (step motors, read sensors, etc.)

**Considerations**:
- Cadenza needs FFI to Rust or C
- Type safety across FFI boundary
- Performance of FFI calls (can't be too slow)
- Hardware abstraction layer (HAL) design

**Solutions**:
- Builtin functions for hardware primitives
- FFI design (already an open question in COMPILER_ARCHITECTURE.md)
- Clear separation: Cadenza for logic, native for hardware
- Type-safe wrappers around unsafe operations

### 6. State Management

**Challenge**: Printer state (position, temperature, feedrate) must be tracked and updated atomically.

**Considerations**:
- Immutable state vs mutable state
- Functional approach: handlers return new state
- Performance of copying state on every command
- Concurrent access if multi-threaded

**Solutions**:
- Handlers take state, return new state (functional)
- Efficient state representation (small copies or COW)
- Effect system could provide state context (Phase 4)
- Single-threaded for simplicity initially

### 7. Units of Measure

**Challenge**: Motion control involves many unit conversions (mm, steps, mm/s, steps/s, etc.)

**Considerations**:
- Dimensional analysis must catch errors
- Need derived dimensions (acceleration = mm/s²)
- Unit conversions should be explicit
- Type system must support unit arithmetic

**Solutions**:
- Leverage existing unit system in Cadenza
- Extend to derived dimensions (velocity, acceleration)
- Compile-time checking of dimensional consistency
- Allow explicit conversions where needed

### 8. Toolchain Complexity

**Challenge**: Adding GCode toolchain (parser, transpiler, validator) increases maintenance burden.

**Considerations**:
- GCode parser separate from Cadenza parser
- Transpiler to generate Cadenza code
- Integration with existing build pipeline
- Testing and validation

**Solutions**:
- Start with direct interpretation (no transpiler)
- Add transpiler as optional advanced feature
- Reuse existing GCode parsing libraries if possible
- Focus on core use case first, optimize later

## Open Questions and Design Considerations

### 1. How should optional parameters be handled?

GCode commands often have optional parameters (e.g., `G1 X100` omits Y, Z, F).

**Options**:
- **Nil values**: Parameters default to `nil`, handlers check explicitly
- **Optional type**: `Optional<T>` with pattern matching
- **Multiple signatures**: Overload handlers for different parameter sets
- **Named parameters**: Handlers use record types with optional fields

**Recommendation**: Optional type with explicit handling, integrates with future type system.

### 2. Should GCode be transpiled or interpreted directly?

**Direct interpretation**:
- ✅ Simpler implementation
- ✅ Streaming support out of the box
- ✅ Lower memory usage
- ❌ No compile-time validation
- ❌ Repeated parsing overhead

**Transpile to Cadenza module**:
- ✅ Full type checking and validation
- ✅ Optimized bytecode
- ✅ Catch errors before print
- ❌ Requires full file in memory
- ❌ Additional toolchain complexity
- ❌ Two-step process

**Recommendation**: Support both. Direct interpretation for simple streaming, transpilation for validation and optimization.

### 3. How to represent GCode command structure?

**Option A: Function calls**
```cadenza
handle_g1 state 100millimeter 50millimeter nil 3000millimeter_per_second
```

**Option B: Record/struct**
```cadenza
handle_g1 state { x = 100millimeter, y = 50millimeter, f = 3000millimeter_per_second }
```

**Option C: Tagged union**
```cadenza
match command {
  G1 { x, y, z, f } -> handle_g1 state x y z f
  G28 -> handle_g28 state
  ...
}
```

**Recommendation**: Start with function calls (A), migrate to records (B) as record system matures. Enums (C) for state machines.

### 4. How much should be compiled vs interpreted?

**Spectrum**:
1. **All interpreted**: Config and handlers interpreted at runtime
2. **Config compiled, GCode interpreted**: Config/handlers AOT compiled, GCode streamed
3. **All compiled**: GCode transpiled to module, everything AOT compiled

**Trade-offs**:
- More compilation = more validation, better performance, less flexibility
- More interpretation = simpler, more flexible, validation at runtime

**Recommendation**: Hybrid approach (2) initially. Precompile configuration and handlers, interpret GCode commands. Add full compilation (3) as optional optimization.

### 5. What level of GCode compatibility is needed?

**Considerations**:
- Full GCode spec is large and complex
- Many vendor-specific extensions (Marlin, RepRap, etc.)
- Core subset sufficient for most printers

**Recommendation**: Support core GCode commands (G0, G1, G28, M104, M109, etc.) initially. Extensible handler system allows users to add vendor-specific commands. Document supported subset clearly.

### 6. How to handle GCode comments and metadata?

GCode files contain comments (`;` or inline with `()`), metadata, thumbnail images, etc.

**Options**:
- Strip comments during parsing
- Preserve as metadata
- Expose to handlers for processing

**Recommendation**: Strip comments initially, preserve metadata in structured format for advanced features (print time estimates, thumbnail preview).

### 7. Should state be mutable or immutable?

**Immutable** (functional):
```cadenza
let new_state = handle_g1 state x y z f
```

**Mutable** (imperative):
```cadenza
handle_g1 state x y z f  # Mutates state in place
```

**Trade-offs**:
- Immutable: Safer, easier to reason about, functional style
- Mutable: More familiar, potentially more efficient, imperative style

**Recommendation**: Start with immutable (functional) approach. Effect system (Phase 4) could provide mutable context if needed for performance.

### 8. How to provide debugging and visualization?

**Features**:
- Trace handler execution
- Log state changes
- Visualize toolpath
- Simulate without hardware
- Breakpoints and stepping

**Considerations**:
- Effect system for logging (Phase 4)
- Separate simulation mode
- Integration with existing tools (OctoPrint, etc.)

**Recommendation**: Design hooks for debugging from start. Effect system provides logging naturally. Simulation mode by providing mock hardware effects.

### 9. What's the performance target?

**Metrics**:
- Commands per second
- Handler execution time
- Memory footprint
- Startup time

**Targets** (compared to Klipper):
- Similar throughput (thousands of commands/sec)
- Sub-millisecond handler execution
- Comparable memory usage
- Fast firmware startup (<5 seconds)

**Recommendation**: Benchmark against Klipper. Optimize after proving correctness. Profile and iterate.

## Success Criteria

This GCode interpreter environment would be successful if it achieves:

### Core Functionality
- ✅ Load and execute a Cadenza configuration module
- ✅ Register GCode handlers as Cadenza functions
- ✅ Parse and dispatch GCode commands to handlers
- ✅ Validate parameters against configuration constraints
- ✅ Update printer state correctly
- ✅ Execute basic print jobs (home, move, extrude, temperature control)

### Type Safety
- ✅ Configuration type-checked at load time
- ✅ Handler signatures validated
- ✅ GCode arguments type-checked (if compiled to module)
- ✅ Dimensional analysis prevents unit errors
- ✅ Compile-time validation catches errors before printing

### Performance
- ✅ Comparable throughput to Klipper (thousands of commands/sec)
- ✅ Low latency for real-time motion control
- ✅ Acceptable memory footprint for embedded systems
- ✅ Fast startup time

### Developer Experience
- ✅ Clear, readable configuration syntax
- ✅ IDE support (autocomplete, inline errors)
- ✅ Good error messages with location information
- ✅ Easy to add custom handlers
- ✅ Documentation and examples

### Extensibility
- ✅ Support vendor-specific GCode commands
- ✅ Pluggable hardware abstraction layer
- ✅ Configurable without code changes
- ✅ Integrate with existing tools (slicers, monitoring software)

## Next Steps

### Phase 1: Proof of Concept (Foundational)
1. Design handler registration API
2. Implement basic GCode parser (or reuse existing)
3. Create simple configuration example
4. Prototype handler dispatch mechanism
5. Validate feasibility on embedded hardware

### Phase 2: Core Implementation (Depends on Type System)
1. Implement handler type-checking
2. Add dimensional analysis for motion parameters
3. Create configuration validation
4. Build handler execution engine
5. Test with real GCode files

### Phase 3: Advanced Features (Depends on Modules, Effects)
1. Module system for configuration
2. Effect system for hardware IO
3. FFI for low-level control
4. Error handling with recovery
5. Debugging and tracing tools

### Phase 4: Optimization (Depends on Code Generation)
1. Compile handlers to native code
2. GCode to Cadenza transpiler
3. Bytecode compilation for modules
4. Performance profiling and optimization
5. Embedded runtime optimization

### Phase 5: Production Readiness
1. Comprehensive testing (unit, integration, hardware)
2. Documentation and tutorials
3. Example configurations for popular printers
4. Tool integration (slicers, OctoPrint, etc.)
5. Community feedback and iteration

## Relationship to Existing Work

### Klipper
This design is inspired by Klipper's architecture:
- **Klipper**: Python for high-level logic, C for low-level control, config files
- **Cadenza**: Cadenza for high-level logic, native code for low-level control, Cadenza config modules

**Advantages over Klipper**:
- Type safety throughout
- Dimensional analysis for units
- Compile-time validation
- Unified language for config and handlers
- No Python runtime overhead

### Marlin
Marlin is pure C++ firmware with limited configurability.

**Advantages over Marlin**:
- Much more flexible and configurable
- Modern language features
- Safer (type-checking, validation)
- Easier to extend and customize

### RepRapFirmware
RepRapFirmware uses configuration files but handlers are hardcoded in C++.

**Advantages over RepRapFirmware**:
- Handlers defined in configuration (not firmware)
- More flexible customization
- Type-safe configuration
- Better error checking

## Conclusion

Using Cadenza as a GCode interpreter environment for 3D printing is an ambitious but achievable goal that leverages the language's unique strengths:

- **Type safety**: Catch errors at compile time
- **Dimensional analysis**: Ensure correct units in motion control
- **Functional approach**: Immutable state, pure functions, easier to reason about
- **Metaprogramming**: Macros for handler registration and code generation
- **Unified language**: Configuration, logic, and execution in one language

The main challenges are:
1. Real-time constraints and performance
2. Runtime dependencies (parser, interpreter)
3. FFI for hardware control
4. Error handling during prints

These challenges can be addressed through:
- Ahead-of-time compilation of handlers
- Efficient runtime with minimal overhead
- Careful design of FFI and effect system
- Robust error handling with recovery

**Current Status**: Most foundational language features exist (functions, records, units, interpreter). The main dependencies are:
- **Type system** (Phase 2) - For handler validation
- **Module system** (Phase 3) - For organized configuration
- **Effect system** (Phase 4) - For hardware IO
- **Code generation** (Phase 5) - For compilation and optimization

**Timeline**: This is a long-term goal (2+ phases away from current state) but worth documenting now to inform language design decisions. Features like assertions, optional values, record merging, and error handling should be designed with this use case in mind.

This document serves as a north star for ensuring Cadenza develops the capabilities needed for real-world embedded control applications.
