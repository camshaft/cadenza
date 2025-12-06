# Visual Art and Generative Graphics Environment

## Vision

Enable Cadenza as a platform for creating generative art, interactive graphics, and visual compositions. Combines functional programming with immediate visual feedback, similar to Processing or p5.js but with type safety and dimensional analysis.

## Goals

1. **Canvas-Based Drawing**: 2D graphics primitives (shapes, paths, colors)
2. **Animation**: Time-based updates and transitions
3. **Interactivity**: Mouse, keyboard, touch input
4. **Generative Systems**: Algorithmic pattern generation
5. **Export**: Save as images (PNG, SVG) or video

## Example Usage

```cadenza
# Simple sketch
@export "circles"
fn draw =
  # Canvas size
  let width = 800
  let height = 600
  
  # Create background
  fill "#ffffff"
  rect 0 0 width height
  
  # Draw circles in a grid
  for x in range 0 width 50
    for y in range 0 height 50
      let size = noise x y * 30  # Perlin noise for variation
      fill "#ff0000"
      circle x y size

# Animated sketch
@export "animation"
fn draw frame_time =
  let radius = 100 + (50 * sin frame_time)
  circle 400 300 radius
```

## Architecture

Similar to Processing/p5.js model:
- Setup function runs once
- Draw function called every frame
- Event handlers for interaction
- Canvas-based rendering (HTML5 Canvas or WebGL)

## Implementation Considerations

### Browser Target
- Compile to JavaScript/WASM
- Render to HTML5 Canvas or WebGL
- Use requestAnimationFrame for smooth animation
- Touch/mouse event handling

### Native Target  
- Use graphics libraries (skia, cairo, or wgpu)
- Window management
- Export to image files or video

### Graphics Primitives
- **2D Shapes**: circle, rect, line, arc, polygon
- **Paths**: bezier curves, custom shapes
- **Colors**: RGB, HSL, hex codes, alpha channel
- **Transforms**: translate, rotate, scale
- **Compositing**: blend modes, clipping
- **Text**: Font rendering, text layout

### Required Features
- **Effect system** for canvas state (current color, transform, etc.)
- **Time type** for animations (seconds, frames)
- **Event system** for user input
- **Image type** for loading/saving graphics
- **FFI** to canvas/graphics libraries

## Success Criteria

- ✅ Draw 2D shapes with colors and transforms
- ✅ Animate graphics at 60fps
- ✅ Respond to user input (mouse, keyboard)
- ✅ Export to PNG/SVG
- ✅ Generate algorithmic patterns
- ✅ Live preview in browser

## Relationship to Existing Tools

- **Processing**: Visual art programming (Java-based)
- **p5.js**: JavaScript graphics library
- **openFrameworks**: C++ creative coding toolkit
- **Cadenza advantage**: Type safety, dimensional analysis, better error messages

## Timeline

Medium-term goal (Phase 4-5):
- Depends on effect system for canvas state
- Needs FFI for graphics libraries
- Builds on REPL and Compiler Explorer infrastructure

## Next Steps

1. Define graphics primitive types (Color, Shape, Path)
2. Design canvas/drawing effect system
3. Implement basic shapes and rendering
4. Add time-based animation support
5. Integrate with Compiler Explorer for live preview

---

# Interactive Books and Educational Content Environment

## Vision

Enable Cadenza to power interactive educational content where code examples are executable, modifiable, and visualized inline. Think Jupyter notebooks, Observable, or interactive textbooks, but with Cadenza's type safety and dimensional analysis.

## Goals

1. **Executable Textbooks**: Code runs within documentation
2. **Live Exploration**: Readers can modify and re-run examples
3. **Visual Explanations**: Graphs, diagrams, animations inline
4. **Scientific Computing**: Math notation, formulas, plots
5. **Reproducible Research**: Version-controlled, reproducible results

## Example Usage

```markdown
# Physics Tutorial: Projectile Motion

The range of a projectile is given by:

$$R = \frac{v^2 \sin(2\theta)}{g}$$

Let's calculate this in Cadenza:

```cadenza
measure meter
measure meter_per_second
measure degree

fn projectile_range velocity angle =
  let g = 9.8 meter_per_second^2
  let theta_rad = angle * (pi / 180)
  (velocity^2 * sin(2 * theta_rad)) / g

# Try different velocities and angles
let range_45 = projectile_range 20meter_per_second 45degree
# Output: 40.8 meters

# Interactive slider: velocity = [0-50] m/s
# Interactive slider: angle = [0-90] degrees
# [Live plot of trajectory]
\```

Change the velocity and angle above to see how it affects the range!
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     Markdown Document                            │
│                                                                  │
│  # Title                                                         │
│                                                                  │
│  Explanation text...                                             │
│                                                                  │
│  ```cadenza                                                      │
│  [Executable code block]                                         │
│  ```                                                             │
│                                                                  │
│  [Rendered output/visualization]                                 │
│                                                                  │
│  More explanation...                                             │
└─────────────────────────────────────────────────────────────────┘
                        │
                        ▼
┌─────────────────────────────────────────────────────────────────┐
│              Rendering Pipeline                                  │
│                                                                  │
│  1. Parse Markdown + extract code blocks                        │
│  2. Compile Cadenza code blocks                                 │
│  3. Execute and capture results                                 │
│  4. Render visualizations                                       │
│  5. Inject interactive controls                                 │
│  6. Generate final HTML/PDF                                     │
└─────────────────────────────────────────────────────────────────┘
```

## Implementation Considerations

### Document Format
- **Markdown front-end syntax**: Similar to specialized parsers, translate Markdown to Cadenza CST/AST
- **Macro-based processing**: Macros in eval context process markdown into executable
- **Mode switching**: Switch parsing modes mid-document for Cadenza syntax
- **Code fence arguments**: Pass arguments to control handling (editable, hidden, output format)
- **Math notation**: Use Cadenza for formulas instead of MathJax (type-safe math expressions)
- **Custom directives** for interactive elements

### Execution Model
- Each code block evaluated in sequence
- State carries forward between blocks
- Errors shown inline, don't break document
- Results rendered based on type (table, plot, 3D, etc.)

**Code Fence Options** (via arguments):
- Execute in own environment and make editable
- Don't render in document (execute to define variables only)
- Return value rendered as markdown, SVG, or interactive widget
- Control whether output is shown, cached, or hidden

### Interactivity
- **Sliders** for numeric parameters
- **Dropdowns** for options
- **Buttons** to trigger actions
- **Reactive** - changes propagate automatically

### Output Types
- **Text**: Simple values, formatted output
- **Tables**: Dataframes, matrices
- **Plots**: Line graphs, scatter plots, histograms
- **3D**: Embedded Three.js visualizations
- **Audio**: Waveforms, play buttons
- **Custom**: Extensible visualization system

### Required Features
- **Module system** to organize teaching materials
- **Effect system** for interactive I/O
- **Plotting library** or integration
- **Documentation generation** from code
- **Notebook format** (like Jupyter .ipynb)

## Use Cases

### Queueing Theory Book (Real-World Example)
- Interactive book at https://camshaft.github.io/kew/
- Currently uses JavaScript + WASM binary (separate and not cohesive)
- Goal: Tight integration where everything is Cadenza
- Simulations, visualizations, and explanations in one unified system

### Physics Textbook
- Interactive simulations
- Unit-aware calculations
- Dimensional analysis examples
- Experiment with parameters

### Computer Science Course
- Algorithm visualization
- Data structure manipulation
- Type system examples
- Performance analysis

### Engineering Calculations
- Load calculations with units
- Safety factor analysis
- Material selection
- Design optimization

### Mathematics
- Function plotting
- Calculus visualization
- Linear algebra operations
- Statistical analysis

## Success Criteria

- ✅ Execute code blocks sequentially
- ✅ Display results with appropriate visualization
- ✅ Interactive controls update downstream results
- ✅ Export to static HTML or PDF
- ✅ Fast iteration for authors
- ✅ Works offline (e.g., in textbook apps)

## Relationship to Existing Tools

- **Jupyter Notebooks**: Python-based notebooks
- **Observable**: JavaScript notebooks
- **R Markdown**: R-based documents
- **Mathematica Notebooks**: Wolfram language
- **mdBook**: Rust documentation (static)
- **Cadenza advantage**: Type safety, units, consistent language across use cases

## Timeline

Medium-term goal (Phase 4-5):
- Depends on module system and effect system
- Needs visualization primitives
- Builds on Compiler Explorer
- Requires rich output type system

## Next Steps

1. Design notebook file format (.cdz.md or similar)
2. Build Markdown parser with code fence extraction
3. Implement sequential evaluation with state
4. Create output rendering system
5. Add interactive control generation
6. Integrate with documentation site

---

## Summary: Visual Art & Interactive Books

Both of these environments share:
- **Browser-first**: Web-based, interactive
- **Visual output**: Graphics, plots, animations
- **Effect system**: Canvas/plotting/I/O effects
- **Compiler Explorer foundation**: Build on existing web infrastructure

**Priority**: Medium (Phase 4-5)
- Requires effect system to be well-designed
- Benefits from completed module system
- Natural extension of Compiler Explorer

**Recommendation**: Start with simple 2D graphics support in Compiler Explorer (canvas output mode), then expand to full-featured visual art tools. Interactive books can build on the same infrastructure once visualization is mature.
