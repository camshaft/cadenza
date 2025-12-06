# 3D Modeling Environment

## Vision

Enable Cadenza to power a code-based 3D modeling system similar to OpenSCAD, where models are defined entirely through code rather than mouse-driven GUI operations. The system emphasizes parametric design, dimensional accuracy, and the ability to export to standard 3D formats.

Key capabilities:
1. **Parametric Modeling**: Models defined by parameters that can be adjusted
2. **Code-First Workflow**: All modeling operations expressed as code
3. **Dimensional Analysis**: Proper unit handling (millimeters, inches, etc.)
4. **Library Precompilation**: Performance-critical geometry operations compiled to native
5. **Interactive Preview**: Live updates as code changes
6. **Export to Standard Formats**: STL, OBJ, STEP, 3MF, etc.
7. **Compiler Explorer**: Web-based interactive environment for learning and experimentation
8. **Shareable Parametric Templates**: Share model code that acts as customizable templates (e.g., snowflake generators with RNG seed)

## Goals

### Primary Goals

1. **Parametric Design**
   - Define models with adjustable parameters
   - Constraints and relationships between dimensions
   - Reusable components and modules
   - Version control friendly (text-based)

2. **Type-Safe Geometry**
   - 3D vectors, points, transformations
   - Ensure dimensional consistency
   - Prevent common errors (mixing 2D and 3D, unit mismatches)
   - Compile-time validation where possible

3. **Performance Through Precompilation**
   - Core geometry library compiled to native code
   - Interpreter coordinates high-level operations
   - Efficient mesh generation
   - Fast boolean operations (union, difference, intersection)

4. **Rich Geometry Library**
   - Primitives: cube, sphere, cylinder, cone, polyhedron
   - 2D shapes: circle, square, polygon
   - Paths: splines, linear points, bezier curves
   - Extrusion and revolution
   - Boolean operations (CSG)
   - Signed Distance Functions (SDF) for advanced modeling
   - Transformations: translate, rotate, scale, mirror
   - Modifiers: fillet, chamfer, offset, shell

5. **Interactive Development**
   - Compiler Explorer web interface
   - Live preview as code changes
   - Visual feedback for errors
   - Parameter sliders for instant adjustment
   - Multiple views (front, top, side, perspective)

### Secondary Goals

- **Advanced Operations**: Sweep, loft, hull, minkowski sum
- **Mesh Analysis**: Volume, surface area, center of mass
- **Validation**: Check for manifold geometry, self-intersections
- **Optimization**: Reduce polygon count, simplify geometry
- **Import**: Load existing models (STL, OBJ) for reference or modification

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                     Model Definition                             │
│                     (model.cdz module)                           │
│                                                                  │
│  - Define parameters (dimensions, counts, etc.)                 │
│  - Create geometry using library functions                      │
│  - Apply transformations and operations                         │
│  - Mark model for export with @export attribute                 │
└─────────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────┐
│                  Compilation Phase                               │
│                                                                  │
│  1. Parse and type-check model definition                       │
│  2. Validate dimensional consistency                            │
│  3. Type-check geometry operations                              │
│  4. Link with precompiled geometry library                      │
└─────────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────┐
│                     Execution Phase                              │
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │        Interpreted Control Logic                         │   │
│  │  - Evaluate parameters                                   │   │
│  │  - Conditionals for design variations                    │   │
│  │  - Loops for pattern generation                          │   │
│  │  - Call precompiled geometry functions                   │   │
│  └──────────────────────────────────────────────────────────┘   │
│                               │                                  │
│                               ▼                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │        Geometry Library (Native Code)                    │   │
│  │  - Primitive generation (cube, sphere, etc.)            │   │
│  │  - Boolean operations (CSG)                              │   │
│  │  - Transformations                                       │   │
│  │  - Mesh generation and optimization                      │   │
│  └──────────────────────────────────────────────────────────┘   │
│                               │                                  │
│                               ▼                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │        Geometry Result                                   │   │
│  │  - Mesh representation (vertices, faces)                │   │
│  │  - Metadata (volume, bounds, etc.)                       │   │
│  │  - Ready for export or preview                           │   │
│  └──────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

### Interactive Compiler Explorer

```
┌─────────────────────────────────────────────────────────────────┐
│                  Browser Environment                             │
│                                                                  │
│  ┌────────────────────┐  ┌──────────────────────────────────┐   │
│  │   Code Editor      │  │   3D Preview                     │   │
│  │   (Monaco/CM)      │  │   (Three.js)                     │   │
│  │                    │  │                                  │   │
│  │  let height = 20mm │  │   [Rendered 3D model]           │   │
│  │  let width = 30mm  │  │                                  │   │
│  │  ...               │  │   - Rotate/pan/zoom              │   │
│  │                    │  │   - Multiple viewports           │   │
│  └────────────────────┘  └──────────────────────────────────┘   │
│          │                              ▲                        │
│          │                              │                        │
│          ▼                              │                        │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │     Cadenza Compiler (WASM/JS)                          │    │
│  │  - Parse and type-check                                 │    │
│  │  - Evaluate to geometry                                 │    │
│  │  - Generate mesh                                        │    │
│  └─────────────────────────────────────────────────────────┘    │
│          │                                                       │
│          ▼                                                       │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │     Parameter Controls                                  │    │
│  │  - Sliders for numeric parameters                       │    │
│  │  - Checkboxes for booleans                              │    │
│  │  - Live update on change                                │    │
│  └─────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────┘
```

## Example Usage

### Basic Model Definition

```cadenza
# Define length units
measure millimeter
measure inch

# Parametric box with rounded corners
@export "rounded_box"
fn rounded_box width height depth corner_radius =
  # Input validation
  assert width > 0millimeter "Width must be positive"
  assert height > 0millimeter "Height must be positive"
  assert depth > 0millimeter "Depth must be positive"
  assert corner_radius > 0millimeter "Corner radius must be positive"
  assert corner_radius < (width / 2) "Corner radius too large for width"
  
  # Create the box
  let box = cube width height depth
  
  # Apply fillets to edges
  fillet box corner_radius

# Create an instance with specific dimensions
let my_box = rounded_box 50millimeter 30millimeter 20millimeter 3millimeter
```

### Complex Assembly

```cadenza
# Parametric enclosure with mounting holes
@export "enclosure"
fn enclosure =
  # Parameters
  let outer_width = 100millimeter
  let outer_height = 60millimeter
  let outer_depth = 40millimeter
  let wall_thickness = 2millimeter
  let corner_radius = 4millimeter
  let mount_hole_diameter = 3millimeter
  let mount_hole_inset = 5millimeter
  
  # Create outer shell
  let outer = rounded_box outer_width outer_height outer_depth corner_radius
  
  # Create inner cavity
  let inner_width = outer_width - (2 * wall_thickness)
  let inner_height = outer_height - (2 * wall_thickness)
  let inner_depth = outer_depth - wall_thickness
  let inner = cube inner_width inner_height inner_depth
    |> translate 0millimeter 0millimeter wall_thickness
  
  # Subtract inner from outer
  let shell = difference outer inner
  
  # Create mounting holes
  let hole = cylinder mount_hole_diameter (wall_thickness + 1millimeter)
  
  # Position holes at corners
  let hole1 = hole |> translate mount_hole_inset mount_hole_inset (-0.5millimeter)
  let hole2 = hole |> translate (outer_width - mount_hole_inset) mount_hole_inset (-0.5millimeter)
  let hole3 = hole |> translate mount_hole_inset (outer_height - mount_hole_inset) (-0.5millimeter)
  let hole4 = hole |> translate (outer_width - mount_hole_inset) (outer_height - mount_hole_inset) (-0.5millimeter)
  
  # Subtract holes from shell
  shell
    |> difference hole1
    |> difference hole2
    |> difference hole3
    |> difference hole4
```

### Patterns and Repetition

```cadenza
# Gear generation
@export "gear"
fn gear teeth module pressure_angle =
  # Calculate dimensions
  let pitch_diameter = teeth * module
  let base_circle = pitch_diameter * cos pressure_angle
  
  # Create tooth profile (simplified)
  let tooth = polygon [
    (0millimeter, 0millimeter),
    (module, 0millimeter),
    (module * 0.8, module * 1.5),
    (module * 0.2, module * 1.5),
  ]
  
  # Rotate and repeat to create all teeth
  let teeth_list = for i in range teeth
    let angle = (360 / teeth) * i
    tooth |> rotate_z angle
  
  # Union all teeth
  let gear_profile = union teeth_list
  
  # Extrude to create 3D gear
  extrude gear_profile 5millimeter

# Grid of objects
fn grid_array object spacing count_x count_y =
  let objects = for x in range count_x
    for y in range count_y
      let x_pos = x * spacing
      let y_pos = y * spacing
      object |> translate x_pos y_pos 0millimeter
    |> flatten
  union objects

# Create grid of cylinders
let cylinder_grid = grid_array 
  (cylinder 5millimeter 10millimeter)
  15millimeter
  4
  3
```

### Conditional Design

```cadenza
# Bracket with optional features
@export "bracket"
fn bracket with_fillet with_holes =
  # Base shape
  let base = cube 50millimeter 30millimeter 5millimeter
  
  # Add fillet if requested
  let base = if with_fillet
    fillet base 3millimeter
  else
    base
  
  # Add mounting holes if requested
  let base = if with_holes
    let hole = cylinder 3millimeter 6millimeter
    let hole1 = hole |> translate 10millimeter 15millimeter (-0.5millimeter)
    let hole2 = hole |> translate 40millimeter 15millimeter (-0.5millimeter)
    base
      |> difference hole1
      |> difference hole2
  else
    base
  
  base

# Generate variations
let bracket_plain = bracket false false
let bracket_filleted = bracket true false
let bracket_complete = bracket true true
```

## Required Language Features

### Already Available ✅

1. **Functions** - Define reusable geometry operations
2. **Let bindings** - Parameters and intermediate results
3. **Records** - Store points, vectors, configuration
4. **Arithmetic** - Calculate dimensions
5. **Comparison** - Validate constraints
6. **Units of measure** - Dimensional analysis (millimeter, inch, etc.)
7. **Assertions** - Runtime validation
8. **Block expressions** - Multi-step modeling operations

### In Progress 🚧

1. **Type system** - Geometry type safety (Phase 2)
2. **Module system** - Organize geometry libraries (Phase 3)

### Needed for 3D Modeling 🔨

1. **Vector and Point Types**
   - 2D and 3D vectors
   - Point vs vector distinction
   - Vector operations (add, subtract, dot, cross)
   - Type safety (can't add point to point)

2. **Geometry Primitive Types**
   - Solid (3D geometry)
   - Surface (2D in 3D space)
   - Curve (1D in 3D space)
   - Mesh (triangulated representation)
   - Type-safe operations per geometry type

3. **Transformations**
   - Translate, rotate, scale, mirror
   - Matrix representation
   - Composition of transformations
   - Type safety (ensure dimensions match)

4. **Boolean Operations**
   - Union (join solids)
   - Difference (subtract)
   - Intersection
   - Efficient CSG implementation
   - FFI to native geometry libraries

5. **List Operations**
   - Map, filter, fold for geometry lists
   - Union/difference of lists
   - Pattern repetition helpers

6. **For Loops**
   - Generate repeated patterns
   - Iterate over ranges
   - List comprehensions

7. **Conditionals**
   - Design variations based on parameters
   - Optional features
   - Guard clauses for validation

8. **@export Attribute**
   - Mark models for export
   - Name the exported artifact
   - Specify export format/options

9. **FFI for Geometry Libraries**
   - Interface with existing geometry kernels (CGAL, OpenCASCADE)
   - Performance-critical operations in native code
   - Type-safe wrapper around unsafe operations

### Nice to Have 🎁

1. **Advanced Geometry Operations**
   - Fillet and chamfer with variable radius
   - Offset (shell, expand/shrink)
   - Sweep along path
   - Loft between profiles
   - Hull (convex hull)
   - Minkowski sum

2. **Mesh Operations**
   - Subdivide mesh
   - Simplify mesh
   - Smooth normals
   - UV mapping

3. **Analysis Functions**
   - Calculate volume
   - Calculate surface area
   - Find center of mass
   - Bounding box
   - Check if manifold

4. **Import Functions**
   - Load STL files
   - Load OBJ files
   - Import SVG as 2D paths (represented same as native 2D path functions)
   - Reference external models

## Implementation Challenges

### 1. Geometry Kernel Integration

**Challenge**: Need robust, efficient geometry operations (CSG, mesh generation).

**Considerations**:
- Implementing CSG from scratch is complex
- Existing libraries (CGAL, OpenCASCADE) are C++
- FFI overhead vs reimplementation
- License compatibility

**Solutions**:
- FFI to proven geometry libraries
- Wrap in type-safe Cadenza API
- Consider lighter-weight alternatives (manifold, libigl)
- Build minimal subset initially, expand later

### 2. Browser Performance

**Challenge**: Running 3D operations in browser needs good performance.

**Considerations**:
- WASM compilation overhead
- Memory constraints
- JavaScript interop for preview
- Large meshes can be slow

**Solutions**:
- Compile geometry library to WASM
- Incremental preview updates
- Level-of-detail for preview
- Offload heavy computation to Web Workers
- Consider server-side rendering for complex models

### 3. Mesh Generation and Optimization

**Challenge**: Converting CSG representation to efficient mesh.

**Considerations**:
- Triangle count affects export size and processing
- Need smooth surfaces (sufficient tessellation)
- Balance quality vs performance
- User control over mesh density

**Solutions**:
- Adaptive mesh density based on curvature
- User parameters for quality settings
- Post-processing optimization (simplification)
- Multiple export options (coarse for preview, fine for final)

### 4. Type Safety for Geometry

**Challenge**: Prevent invalid geometry operations at compile-time.

**Considerations**:
- 2D vs 3D operations should not mix
- Units must be consistent
- Some operations only valid on certain geometry types
- Transformation matrices must match dimensionality

**Solutions**:
- Strong typing for geometry types (Solid2D, Solid3D, Surface, etc.)
- Dimensional analysis for all length measurements
- Type-level tracking of coordinate spaces
- Compiler errors for invalid operations

### 5. Interactive Preview in Compiler Explorer

**Challenge**: Real-time updates as code changes without full recompilation.

**Considerations**:
- Every keystroke triggers recompilation
- Large models take time to generate
- Need smooth user experience
- Memory usage for multiple versions

**Solutions**:
- Incremental compilation where possible
- Debounce updates (wait for pause in typing)
- Cache intermediate results
- Fast path for parameter-only changes
- Progressive rendering (show partial results)

### 6. Parameter Extraction for UI

**Challenge**: Automatically generate UI controls for parameters.

**Considerations**:
- Need to identify top-level parameters
- Extract type information (number, boolean, enum)
- Determine reasonable ranges for sliders
- Handle parameter dependencies

**Solutions**:
- Type introspection at compile-time
- Annotations for UI hints (range, step, default)
- Generate JSON schema from types
- Effect system provides "parameter" context

### 7. Export to Multiple Formats

**Challenge**: Support various 3D file formats with different capabilities.

**Considerations**:
- STL: Only triangulated mesh, widely supported
- OBJ: Mesh with textures, materials
- STEP: Solid model with parametric data
- 3MF: Modern format with color, metadata
- Different formats have different requirements

**Solutions**:
- Start with STL (simplest, most universal)
- Plugin architecture for exporters
- Format-specific validation
- User-selectable export options (quality, units, etc.)

## Open Questions and Design Considerations

### 1. How to represent coordinate systems?

3D modeling involves multiple coordinate systems (world, local, object).

**Options**:
- Implicit global coordinate system
- Explicit coordinate system objects
- Type-level tracking of coordinate spaces

**Recommendation**: Start with implicit global system, add explicit coordinate systems if needed.

### 2. Should operations be mutable or immutable?

**Immutable** (functional):
```cadenza
let box1 = cube 10millimeter 10millimeter 10millimeter
let box2 = translate box1 5millimeter 0millimeter 0millimeter
```

**Mutable** (imperative):
```cadenza
let box = cube 10millimeter 10millimeter 10millimeter
translate! box 5millimeter 0millimeter 0millimeter
```

**Recommendation**: Immutable (functional) style. Fits language design, easier to reason about, enables caching.

### 3. How to handle large model hierarchies?

Complex models may have hundreds of components.

**Options**:
- Flat list of operations
- Hierarchical tree structure
- Named components with references

**Recommendation**: Start simple (flat), add hierarchy when needed. Module system will help organize.

### 4. What level of geometry validation?

**Options**:
- No validation (fast but can produce invalid geometry)
- Basic validation (check for NaN, infinite values)
- Full validation (check manifold, self-intersections)

**Recommendation**: Basic validation by default, optional full validation. Make validation errors informative.

### 5. How to handle units in different contexts?

Some users prefer metric (mm), others imperial (inches).

**Options**:
- Force one unit system
- Allow mixing with conversions
- Unit-agnostic with explicit conversions

**Recommendation**: Allow mixing with automatic conversion. Type system ensures consistency.

### 6. Should we support 2D modeling?

Many designs start in 2D, then extrude/revolve to 3D.

**Recommendation**: Yes, support 2D primitives and operations. Distinct types from 3D to prevent mixing.

### 7. How to integrate with external tools?

**Integrations**:
- CAD software (FreeCAD, Fusion 360)
- Slicers (Cura, PrusaSlicer) for 3D printing
- Renderers (Blender, Cycles) for visualization

**Recommendation**: Export to standard formats (STL, STEP). Let external tools consume exports.

## Success Criteria

This 3D modeling environment would be successful if it achieves:

### Core Functionality
- ✅ Define parametric models in code
- ✅ Basic primitives (cube, sphere, cylinder, cone)
- ✅ Boolean operations (union, difference, intersection)
- ✅ Transformations (translate, rotate, scale)
- ✅ Export to STL format
- ✅ Interactive preview in Compiler Explorer

### Type Safety
- ✅ Dimensional analysis prevents unit errors
- ✅ Geometry types prevent invalid operations
- ✅ Compile-time validation catches errors early
- ✅ Helpful error messages with suggestions

### Performance
- ✅ Reasonable compile times for typical models (<5 seconds)
- ✅ Interactive preview updates (<1 second for parameter changes)
- ✅ Export generation comparable to OpenSCAD
- ✅ Browser performance acceptable for learning/experimentation

### Developer Experience
- ✅ Clear, intuitive syntax for modeling operations
- ✅ Good documentation with examples
- ✅ Compiler Explorer for learning
- ✅ Parameter controls for experimentation
- ✅ Helpful error messages

## Next Steps

### Phase 1: Foundation
1. Define geometry types (Solid, Surface, Mesh, Vector, Point)
2. Implement basic primitives (cube, sphere, cylinder)
3. Implement transformations (translate, rotate, scale)
4. Design FFI for native geometry library
5. Create simple examples and tests

### Phase 2: CSG Operations
1. Implement union operation
2. Implement difference operation
3. Implement intersection operation
4. Mesh generation from CSG tree
5. Test with complex models

### Phase 3: Web Preview
1. Compile to WASM for browser
2. Three.js integration for 3D rendering
3. Interactive editor with live preview
4. Parameter extraction and UI generation
5. Deploy Compiler Explorer

### Phase 4: Export and Polish
1. STL export implementation
2. Export validation and testing
3. Advanced operations (fillet, chamfer, offset)
4. Performance optimization
5. Documentation and tutorials

### Phase 5: Advanced Features
1. 2D primitives and operations
2. Sweep and loft operations
3. Mesh analysis functions
4. Import STL/OBJ files
5. Additional export formats (OBJ, STEP, 3MF)

## Relationship to Existing Work

### OpenSCAD
Text-based 3D CAD modeler.
- **Cadenza advantage**: Modern syntax, type safety, better error messages, faster compilation

### CadQuery (Python)
Parametric CAD in Python.
- **Cadenza advantage**: Static typing, dimensional analysis, better performance via native compilation

### Grasshopper (Rhino)
Visual parametric modeling.
- **Cadenza advantage**: Text-based (version control), type checking, reproducibility

### ImplicitCAD
Haskell-based CAD.
- **Cadenza advantage**: More accessible syntax, better tooling, web-based explorer

## Conclusion

Using Cadenza as a 3D modeling environment leverages its unique strengths:

- **Type safety**: Catch geometric errors at compile-time
- **Dimensional analysis**: Ensure unit consistency throughout model
- **Parametric by design**: Everything is code, easily version controlled
- **Interactive exploration**: Compiler Explorer lowers barrier to entry
- **Performance**: Native geometry library for production use

The main challenges are:
1. Integration with geometry kernel (FFI)
2. Browser performance for interactive preview
3. Mesh generation and optimization
4. Type safety for diverse geometry operations

These challenges can be addressed through:
- Proven geometry libraries via FFI
- WASM compilation for browser
- Configurable quality settings
- Strong type system with geometry-specific types

**Current Status**: Most foundational language features exist. Main dependencies are:
- **Vector/Point types** - New, needs design
- **Geometry types** - New, needs design
- **FFI for geometry library** - Depends on FFI design (Phase 4+)
- **@export attribute** - Planned for module system (Phase 3)

**Timeline**: This is a medium-term goal (2-3 phases away) but the Compiler Explorer can start with simpler evaluation results (numbers, strings) and evolve to 3D previews as geometry features are added.

This document serves as a north star for ensuring Cadenza develops into a powerful tool for parametric 3D modeling.
