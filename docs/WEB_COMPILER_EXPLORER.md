# Web-Based Compiler Explorer Environment

## Vision

Provide an interactive, browser-based environment for learning Cadenza, experimenting with code, and sharing examples. Similar to the Rust Playground or TypeScript Playground, but with live visualization of results tailored to each use case (3D models, audio waveforms, calculations, etc.).

Key capabilities:
1. **Zero Installation**: Run entirely in browser via WASM
2. **Live Preview**: See results as code changes
3. **Examples Library**: Curated examples for learning
4. **Shareable Links**: Share code via URL
5. **Multiple Output Modes**: Text, 3D visualization, audio, plots
6. **Educational Focus**: Lower barrier to entry for new users

## Goals

### Primary Goals

1. **Instant Access**
   - No signup or installation required
   - Load in <3 seconds
   - Works on desktop and mobile
   - Offline support (PWA)

2. **Live Interactive Feedback**
   - Compile and run on every change (debounced)
   - Show errors inline with code
   - Visual output for applicable results
   - Real-time type information on hover

3. **Learning-Oriented**
   - Structured examples by topic
   - Progressive complexity
   - Inline documentation
   - Guided tutorials

4. **Code Sharing**
   - Generate shareable URLs
   - Embed code snippets in documentation
   - Export code to local files
   - Import from gists/URLs

### Secondary Goals

- **Multiple Themes**: Light/dark mode, various color schemes
- **Keyboard Shortcuts**: Vim/Emacs modes
- **Accessibility**: Screen reader support, keyboard navigation
- **Mobile Experience**: Touch-friendly interface
- **Collaboration**: Real-time co-editing (future)

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                        Browser                                   │
│                                                                  │
│  ┌─────────────────┐  ┌────────────────────────────────────┐    │
│  │  Code Editor    │  │  Output Panel                      │    │
│  │  (Monaco)       │  │                                    │    │
│  │                 │  │  [Result visualization]            │    │
│  │  let x = 1 + 1  │  │  - Text output                     │    │
│  │                 │  │  - 3D preview (Three.js)          │    │
│  │  [Type info]    │  │  - Waveform (for audio)           │    │
│  │  [Errors]       │  │  - Plots and graphs                │    │
│  └─────────────────┘  └────────────────────────────────────┘    │
│         │                          ▲                             │
│         │                          │                             │
│         ▼                          │                             │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │     Cadenza Compiler (WASM)                              │   │
│  │  - Lexer/Parser (cadenza-syntax)                         │   │
│  │  - Evaluator (cadenza-eval)                              │   │
│  │  - Type checker                                          │   │
│  │  - Error formatter                                       │   │
│  └──────────────────────────────────────────────────────────┘   │
│         │                                                        │
│         ▼                                                        │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │     UI State Management                                  │   │
│  │  - Examples library                                      │   │
│  │  - URL state persistence                                 │   │
│  │  - Local storage for preferences                         │   │
│  │  - Theme management                                      │   │
│  └──────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

## Example User Flows

### Learning Flow

```
User lands on playground
  → Sees "Welcome" example with explanation
  → Clicks "Examples" dropdown
  → Selects "Arithmetic with Units"
  → Modifies values, sees live updates
  → Clicks "Next: Variables and Functions"
  → Progresses through guided tutorial
```

### Experimentation Flow

```
User has an idea for a calculation
  → Opens blank editor
  → Types code with autocomplete
  → Sees type errors inline
  → Fixes errors
  → Sees result
  → Clicks "Share" to get URL
  → Pastes URL in Discord/forum
```

### 3D Modeling Flow

```
User wants to design a part
  → Selects "3D Modeling" from examples
  → Modifies parameters (width, height, radius)
  → Sees 3D preview update in real-time
  → Rotates/zooms 3D view
  → Exports STL when satisfied
```

## Output Visualization Modes

### Text Output
```cadenza
# For simple calculations
> 1 + 1
2

> "Hello " ++ "World"
"Hello World"
```

### 3D Visualization (Three.js)
```cadenza
# For geometry
@export "box"
cube 10millimeter 20millimeter 30millimeter

# Renders interactive 3D view with orbit controls
```

### Audio Visualization
```cadenza
# For music/audio (future)
@export "tone"
sine_wave 440hertz
  |> duration 1second

# Shows waveform, play button, download WAV
```

### Plot/Graph (future)
```cadenza
# For mathematical functions
@export "plot"
plot sin 0 (2 * pi)

# Renders 2D graph with axes
```

## Implementation Plan

### Phase 1: Basic Editor and Compilation
- [ ] Set up Monaco editor with Cadenza syntax highlighting
- [ ] Compile cadenza-syntax and cadenza-eval to WASM
- [ ] Wire editor changes to compiler
- [ ] Display text results
- [ ] Show compiler errors inline

### Phase 2: Examples and UI
- [ ] Create examples library (matches test-data examples)
- [ ] Implement examples dropdown/navigation
- [ ] Add "Share" button (URL encoding)
- [ ] Add "Download" button (save to .cdz file)
- [ ] Implement theme switcher

### Phase 3: Advanced Editor Features
- [ ] LSP integration for autocomplete
- [ ] Hover tooltips with type information
- [ ] Go to definition (within same file)
- [ ] Format code button
- [ ] Keyboard shortcuts help

### Phase 4: 3D Visualization
- [ ] Three.js integration
- [ ] Parse geometry results from evaluator
- [ ] Convert to Three.js mesh
- [ ] Orbit controls for camera
- [ ] Multiple viewport modes (top/front/side)

### Phase 5: Polish and Performance
- [ ] Optimize WASM size and loading
- [ ] Add loading indicators
- [ ] Implement PWA for offline use
- [ ] Mobile-responsive layout
- [ ] Accessibility improvements

## Technical Considerations

### WASM Compilation Strategy

**For Browser Target**:
- Compile Rust crates to WASM using `wasm-pack`
- Use `wasm-bindgen` for JS interop
- Keep WASM binary small (<1MB ideally)
- Lazy load additional features

**Optimization**:
- Strip symbols and debug info for production
- Use `wasm-opt` for size reduction
- Consider dynamic linking for large libraries
- Cache WASM in service worker

### URL Encoding

Encode code in URL for sharing:
```
https://play.cadenza.dev/?code=<base64-encoded-code>
```

**Considerations**:
- URL length limits (2048 chars typical)
- Compression for longer examples (use LZ-string)
- Fallback to local storage for very long code
- Server-side storage for large examples (gist-style)

### Performance Targets

- **Initial load**: <3s on 3G connection
- **Compile time**: <500ms for typical examples
- **Render update**: <100ms after code change
- **Memory usage**: <50MB for basic examples

### Mobile Experience

**Challenges**:
- Small screen space
- Touch keyboard
- No hover for tooltips

**Solutions**:
- Collapsible panels
- Tab bar for mobile views
- Touch-friendly buttons
- Long-press for hover info

## Example Library Structure

### Beginner Examples
1. `example-01-welcome.cdz` - Introduction and basic arithmetic
2. `example-02-literals.cdz` - Numbers, strings, booleans
3. `example-03-arithmetic.cdz` - Math operators
4. `example-04-comparison.cdz` - Comparison operators
5. `example-05-variables.cdz` - Let bindings and assignment

### Intermediate Examples
6. `example-06-functions.cdz` - Function definitions and calls
7. `example-07-measures.cdz` - Units of measure and dimensional analysis
8. `example-08-records.cdz` - Record creation and field access
9. `example-09-lists.cdz` - List operations
10. `example-10-conditionals.cdz` - If/else expressions

### Advanced Examples
11. `example-11-higher-order.cdz` - Map, filter, fold
12. `example-12-modules.cdz` - Imports and exports
13. `example-13-types.cdz` - Type annotations and inference
14. `example-14-3d-modeling.cdz` - Basic 3D shapes (if geometry library exists)
15. `example-15-physics.cdz` - Physics calculations with units

## Required Language Features

### Already Available ✅
1. **Parser and evaluator** - Core compilation
2. **Error reporting** - Diagnostics with spans
3. **Examples** - test-data directory has examples

### In Progress 🚧
1. **WASM compilation** - cadenza-web crate exists
2. **Web UI** - Basic structure in place

### Needed for Compiler Explorer 🔨
1. **TypeScript Bindings**
   - WASM wrapper with clean JS API
   - Type definitions (.d.ts files)
   - Error handling across boundary

2. **Syntax Highlighting**
   - Monaco language definition
   - TextMate grammar for Cadenza
   - Semantic token provider

3. **URL Encoding/Decoding**
   - Compress code for URL
   - Handle URL parameters
   - Local storage fallback

4. **Examples Loader**
   - Fetch examples from bundled data
   - Parse metadata (title, description, category)
   - Navigation UI

## Success Criteria

- ✅ Load and run in <3 seconds
- ✅ Support all basic Cadenza examples
- ✅ Clear error messages with source locations
- ✅ Shareable URLs work reliably
- ✅ Responsive on desktop and mobile
- ✅ Works offline after first load
- ✅ Examples progress from simple to complex
- ✅ 3D preview for geometry results (when implemented)

## Next Steps

1. **Immediate (Phase 1)**:
   - Ensure cadenza-web builds to WASM
   - Basic Monaco integration
   - Display evaluation results

2. **Short-term (Phase 2)**:
   - Load examples from test-data
   - Implement URL sharing
   - Add theme switcher

3. **Medium-term (Phase 3-4)**:
   - LSP integration for better editing
   - Three.js for 3D visualization
   - Mobile optimization

4. **Long-term (Phase 5+)**:
   - Audio visualization
   - Collaborative editing
   - Backend for larger examples

## Conclusion

The Compiler Explorer serves as Cadenza's public face:
- **Zero friction** for trying the language
- **Educational** for learning
- **Shareable** for community building
- **Showcase** for unique features

Priority: **HIGH** - This is how people discover Cadenza.

Current status: Basic infrastructure exists in cadenza-web, needs UI polish and examples integration.

Timeline: Basic version feasible in Phase 2 (concurrent with type system), full features in Phase 3-4.
