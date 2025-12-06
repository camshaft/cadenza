# Music Composition Environment

## Vision

Enable Cadenza to power an algorithmic music composition system that combines the best of live coding, offline rendering, and precise musical control. The system integrates with the [bach](https://github.com/camshaft/bach) discrete event simulator and [euphony](https://github.com/camshaft/euphony-rs) project to create a comprehensive music programming environment.

Key capabilities:
1. **Live Performance Mode**: Real-time execution responding to MIDI/UI events
2. **Offline Rendering**: Non-realtime parallel rendering as fast as possible
3. **DSP Precompilation**: Performance-critical audio processing compiled to native code
4. **Interpreted Control**: High-level composition logic interpreted for flexibility
5. **Autonomous Tasks**: Multiple concurrent tasks coordinating via discrete event simulation
6. **Interactive REPL**: Live experimentation and parameter adjustment

## Goals

### Primary Goals

1. **Dual Execution Modes**
   - **Online (Live)**: Real-time event handling with predictable latency
   - **Offline (Render)**: Fast parallel rendering without real-time constraints
   - Switch between modes seamlessly
   - Share composition code between both modes

2. **Hybrid Compilation Strategy**
   - Precompile DSP algorithms to native code for performance
   - Interpret control logic for flexibility and live updates
   - JIT compilation for hot paths when beneficial
   - Zero-cost abstractions where possible

3. **Discrete Event Simulation**
   - Integration with bach library for event scheduling
   - Autonomous tasks emitting commands and MIDI events
   - Precise timing control with musical time units
   - Task coordination and synchronization

4. **Interactive Development**
   - REPL for live coding and parameter tweaking
   - Hot-reload of composition changes
   - Visual feedback (waveforms, spectrograms, parameter graphs)
   - Inspection of any value or state for better understanding
   - Undo/redo for experimentation

5. **Musical Units and Types**
   - Time: beats, measures, seconds, samples
   - Pitch: Hz, MIDI notes, note names (C4, A440), modes, intervals
   - Progressive refinement: interval → mode → tonic → tuning → frequency
   - Amplitude: dB, linear gain, MIDI velocity
   - Bitwidth integer types for MIDI (7-bit, 14-bit values)
   - Type safety for musical operations with dimensional analysis

6. **Advanced Audio Routing**
   - Logical groups for DSP outputs (e.g., "lead", "drums")
   - Per-group DSP chains
   - Spatial audio: place signals in polar coordinates
   - Virtual space rendering (implementation-dependent positioning)
   - Subrenderings: cache complex waveforms for reuse as wavetables

### Secondary Goals

- **MIDI Integration**: Send/receive MIDI messages, CC, program changes
- **Audio I/O**: Multi-channel input/output
- **Plugin Hosting**: VST/AU/CLAP plugin support
- **Score Export**: MusicXML, MIDI file export
- **Visualization**: Real-time spectrum, waveform, piano roll

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                   Composition Module                             │
│                   (composition.cdz)                              │
│                                                                  │
│  - Define instruments (DSP graphs)                              │
│  - Define patterns and sequences                                │
│  - Define event generators and transformers                     │
│  - Set up task coordination                                     │
└─────────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────┐
│                  Compilation Phase                               │
│                                                                  │
│  1. Parse and type-check composition                            │
│  2. Identify DSP components for precompilation                  │
│  3. Compile DSP to native code (Rust → native/WASM)            │
│  4. Generate IR for control logic                               │
│  5. Create event scheduler setup                                │
└─────────────────────────────────────────────────────────────────┘
                                │
                ┌───────────────┴───────────────┐
                ▼                               ▼
┌─────────────────────────┐   ┌─────────────────────────────┐
│   Online Mode           │   │   Offline Mode              │
│   (Real-time)           │   │   (Render)                  │
└─────────────────────────┘   └─────────────────────────────┘
                │                               │
                ▼                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                     Runtime Environment                          │
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │          Bach Discrete Event Scheduler                   │   │
│  │  - Schedule musical events with precise timing          │   │
│  │  - Run autonomous tasks                                  │   │
│  │  - Coordinate task communication                         │   │
│  └──────────────────────────────────────────────────────────┘   │
│                               │                                  │
│                               ▼                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │            Control Logic (Interpreted)                   │   │
│  │  - Pattern sequencing                                    │   │
│  │  - Event generation and transformation                   │   │
│  │  - Parameter automation                                  │   │
│  │  - MIDI message handling                                 │   │
│  └──────────────────────────────────────────────────────────┘   │
│                               │                                  │
│                               ▼                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │            DSP Engine (Native Code)                      │   │
│  │  - Audio synthesis and processing                        │   │
│  │  - Effect chains                                         │   │
│  │  - Mixing and routing                                    │   │
│  │  - Buffer management                                     │   │
│  └──────────────────────────────────────────────────────────┘   │
│                               │                                  │
│                               ▼                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │            Audio Output                                  │   │
│  │  - Online: Real-time audio callback                     │   │
│  │  - Offline: Bounce to file (WAV, FLAC, etc.)           │   │
│  └──────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

## Example Usage

### Defining an Instrument

```cadenza
# Define musical units
measure beat
measure hertz
measure decibel

# Simple synthesizer instrument
let synth = instrument
  # Parameters with units
  let frequency = parameter 440hertz
  let gain = parameter -6decibel
  let attack = parameter 0.01beat
  let release = parameter 0.1beat
  
  # DSP graph (compiled to native)
  dsp
    let osc = sine_wave frequency
    let env = adsr attack 0.0beat release 0.1beat
    osc * env * db_to_linear gain
```

### Defining Patterns and Sequences

```cadenza
# Define a melody pattern
let melody = pattern
  # Pattern in beats, with note names and durations
  notes [
    (C4, 1beat),
    (E4, 1beat),
    (G4, 1beat),
    (C5, 1beat),
  ]

# Define a rhythm pattern
let kick_pattern = pattern
  # Trigger on beats: 0, 1, 2, 3
  triggers [0beat, 1beat, 2beat, 3beat]
  instrument kick_drum

# Sequence patterns over time
let arrangement = sequence
  at 0beat -> play melody with synth
  at 4beat -> play melody with synth transpose 7
  at 0beat -> loop kick_pattern every 4beat for 16beat
```

### Event Generation with Tasks

```cadenza
# Autonomous task that generates events
task melodic_generator =
  # Initialize state
  let pitch = 60  # Middle C
  let direction = 1
  
  # Loop forever, emitting events
  loop
    # Emit note-on event
    emit note_on pitch 100
    
    # Wait for duration
    wait 0.5beat
    
    # Emit note-off event
    emit note_off pitch
    
    # Update state for next iteration
    pitch = pitch + direction
    if pitch > 72 || pitch < 48
      direction = -direction
    
    # Wait before next note
    wait 0.5beat

# Task that responds to MIDI input
task midi_responder =
  loop
    # Wait for MIDI event
    let event = await midi_event
    
    match event
      NoteOn { note, velocity } ->
        # Trigger synth with incoming note
        emit play synth note velocity
      
      ControlChange { cc, value } ->
        # Map CC to parameter
        if cc == 1  # Mod wheel
          set_parameter synth.cutoff (value * 100hertz)
```

### Live Mode Example

```cadenza
# Online execution - responds to events in real-time
mode live =
  # Start audio engine
  start_audio
    sample_rate = 48000hertz
    buffer_size = 128
    channels = 2
  
  # Start MIDI input
  start_midi "USB MIDI Interface"
  
  # Launch tasks
  spawn melodic_generator
  spawn midi_responder
  
  # Interactive REPL available
  # User can call functions, query state, adjust parameters
```

### Offline Rendering Example

```cadenza
# Offline execution - render as fast as possible
mode render =
  # Set up render context
  render_audio
    output = "composition.wav"
    sample_rate = 48000hertz
    duration = 60beat  # Render 60 beats
    parallel = true    # Use all CPU cores
  
  # Play arrangement
  play arrangement
  
  # Wait for render to complete
  wait_for_completion
```

## Required Language Features

### Already Available ✅

1. **Functions** - Define instruments, patterns, generators
2. **Let bindings** - Parameter definitions, state
3. **Records** - Configuration, event data
4. **Arithmetic** - Musical calculations
5. **Units of measure** - Time, frequency, amplitude
6. **Macros** - Domain-specific syntax (instrument, pattern, task)
7. **Block expressions** - Multi-step definitions

### In Progress 🚧

1. **Type system** - Musical type safety (Phase 2)
2. **Module system** - Organize instruments and patterns (Phase 3)
3. **Effect system** - Audio I/O, MIDI, state (Phase 4)

### Needed for Music Use Case 🔨

1. **Async/Task System**
   - Spawn concurrent tasks
   - Event loop integration
   - await for events
   - Task communication channels
   - Integration with bach scheduler

2. **Loop Constructs**
   - Infinite loops for tasks
   - Conditional loops
   - Loop with state updates
   - Break/continue semantics

3. **Pattern Matching**
   - Match on MIDI events
   - Destructure event data
   - Guard clauses for conditions

4. **Mutable State in Tasks**
   - Task-local mutable variables
   - Atomic updates
   - State machines for sequencing

5. **Time and Tempo**
   - Musical time units (beats, measures, seconds)
   - Tempo conversion (BPM)
   - Synchronization primitives
   - Swing and groove quantization

6. **Audio Sample Type**
   - Sample rate aware types
   - Buffer operations
   - Channel layouts (mono, stereo, surround)

7. **DSP Primitives**
   - Oscillators (sine, saw, square, noise)
   - Filters (low-pass, high-pass, band-pass)
   - Envelopes (ADSR, custom)
   - Effects (reverb, delay, distortion)
   - FFI to native audio libraries

8. **MIDI Types**
   - Note on/off events
   - Control change messages
   - Program changes
   - Pitch bend, aftertouch
   - System messages

### Nice to Have 🎁

1. **Score Representation**
   - Musical notation types
   - Export to MusicXML/MIDI
   - Import existing scores

2. **Tuning Systems**
   - 12-TET (standard)
   - Just intonation
   - Microtonal scales
   - Custom tuning tables

3. **Audio Analysis**
   - FFT/spectral analysis
   - Pitch detection
   - Beat detection
   - Onset detection

4. **Plugin Integration**
   - Load VST/AU/CLAP plugins
   - Parameter automation
   - Preset management

## Implementation Challenges

### 1. Real-Time vs Non-Real-Time Execution

**Challenge**: Same composition code must work in both real-time (live) and non-real-time (render) modes.

**Considerations**:
- Real-time: Strict latency requirements, no blocking, no GC pauses
- Offline: Can use blocking operations, optimize for throughput
- Need abstraction over time: logical time vs wall-clock time

**Mitigation**:
- Abstract time representation (bach's discrete event simulation)
- Compile DSP to native code (no GC in audio thread)
- Separate control logic (interpreted, can be slower) from DSP (native, must be fast)
- Use fixed-size buffers, pre-allocate memory

### 2. DSP Performance Requirements

**Challenge**: Audio processing requires consistent performance to avoid clicks and dropouts.

**Considerations**:
- Audio callback typically every 2-10ms (buffer size 128-512 samples)
- Must complete processing within deadline
- No allocations in audio thread
- Predictable execution time

**Solutions**:
- Precompile all DSP code to native
- Use arena/pool allocation for fixed-size buffers
- Lock-free communication between threads
- Profile and benchmark critical paths
- Consider SIMD optimization for DSP

### 3. Task Coordination and Event Scheduling

**Challenge**: Multiple autonomous tasks must coordinate and schedule events precisely.

**Considerations**:
- Tasks may produce events at arbitrary future times
- Events must be ordered and dispatched correctly
- Need priority handling for real-time events
- Concurrent task execution

**Solutions**:
- Use bach's discrete event scheduler
- Priority queue for event ordering
- Lock-free event queues where possible
- Task isolation (no shared mutable state except via channels)

### 4. Live Coding and Hot Reload

**Challenge**: Users want to modify code while music is playing without interruption.

**Considerations**:
- Can't stop audio engine (would cause glitches)
- Need smooth transitions between old and new code
- State preservation across reloads
- Type safety during hot reload

**Solutions**:
- Keep DSP engine running, swap control logic
- Fade between old and new DSP graphs
- Serialize/deserialize task state
- Version compatibility checking
- Gradual migration approach

### 5. MIDI and Audio I/O Integration

**Challenge**: Need low-level access to audio hardware and MIDI devices.

**Considerations**:
- Platform differences (ALSA, CoreAudio, ASIO, WASAPI)
- Device enumeration and selection
- Sample rate and buffer size negotiation
- Error handling for device disconnection

**Solutions**:
- FFI to Rust audio libraries (cpal, midir)
- Abstract audio backend (similar to euphony)
- Effect system for I/O capabilities
- Graceful degradation on errors

### 6. Musical Time Representation

**Challenge**: Music operates in beats/measures, but audio operates in samples/seconds.

**Considerations**:
- Tempo changes over time
- Time signature changes
- Sample-accurate timing for events
- Conversion between units

**Solutions**:
- Dimensional analysis for time units
- Tempo map data structure
- Event scheduler handles conversions
- Type system prevents mixing incompatible time units

### 7. Memory Management in Real-Time Context

**Challenge**: Real-time audio processing cannot tolerate GC pauses or allocations.

**Considerations**:
- Audio callback runs every few milliseconds
- Any pause causes audible glitches
- Need bounded execution time

**Solutions**:
- Compile DSP to native code (no GC)
- Pre-allocate all buffers
- Use fixed-size data structures
- Lock-free algorithms for communication
- Keep interpreted control logic out of audio thread

## Open Questions and Design Considerations

### 1. How to express DSP graphs?

**Options**:
- **Functional**: Chain functions (`osc |> filter |> env |> output`)
- **Graph syntax**: Explicit node/edge definitions
- **Macro DSL**: Custom syntax for audio routing

**Recommendation**: Start with functional composition, add graph syntax for complex routings.

### 2. How to handle state in DSP?

Audio processing often needs state (filter coefficients, envelope stage, etc.).

**Options**:
- Implicit state (hidden in DSP nodes)
- Explicit state parameters
- State monad or similar pattern

**Recommendation**: Implicit state for DSP primitives, explicit for user-defined state machines.

### 3. What level of MIDI support?

**Considerations**:
- MIDI 1.0 (note on/off, CC, etc.) - essential
- MIDI 2.0 (high resolution, per-note controllers) - nice to have
- MPE (multidimensional polyphonic expression) - nice to have

**Recommendation**: Start with MIDI 1.0, design to allow future extensions.

### 4. How to represent polyphony?

Multiple notes playing simultaneously.

**Options**:
- Voice stealing and voice allocation
- Unlimited polyphony (spawn voice per note)
- User-specified voice pool

**Recommendation**: Provide voice allocation helpers, allow user control.

### 5. Integration with existing Rust audio ecosystem?

**Considerations**:
- Many excellent Rust audio libraries exist
- FFI overhead vs reimplementation
- Type safety across FFI boundary

**Recommendation**: Use FFI for primitives, wrap in safe Cadenza API. Build on cpal, dasp, fundsp, etc.

### 6. How to visualize audio in REPL?

**Features**:
- Waveform display
- Spectrum analyzer
- Parameter graphs over time
- Piano roll view

**Recommendation**: Effect system provides "visualization" context, REPL requests visual data.

### 7. Export and interoperability?

**Formats**:
- Audio: WAV, FLAC, MP3, OGG
- MIDI: Standard MIDI File (.mid)
- Score: MusicXML, LilyPond

**Recommendation**: Support audio export from day one. MIDI and score export later.

## Success Criteria

This music composition environment would be successful if it achieves:

### Core Functionality
- ✅ Define instruments with DSP graphs
- ✅ Create patterns and sequences
- ✅ Run autonomous tasks generating events
- ✅ Live performance mode with real-time audio
- ✅ Offline rendering mode
- ✅ REPL for live experimentation

### Performance
- ✅ Real-time audio with <10ms latency
- ✅ No dropouts or glitches during performance
- ✅ Offline rendering faster than real-time
- ✅ Efficient CPU usage (comparable to native DAW)

### Developer Experience
- ✅ Clear, expressive syntax for musical concepts
- ✅ Type safety for musical operations
- ✅ Helpful error messages with musical context
- ✅ Fast iteration (hot reload, REPL)
- ✅ Good documentation and examples

### Musical Capabilities
- ✅ Flexible sequencing and arrangement
- ✅ Expressive instrument design
- ✅ MIDI input and output
- ✅ Parameter automation
- ✅ Multiple time signatures and tempo changes

## Next Steps

### Phase 1: Foundation
1. Design task/async primitives for bach integration
2. Define DSP primitive types and operations
3. Create musical time unit system
4. Prototype simple oscillator and envelope
5. Test real-time audio callback integration

### Phase 2: Core DSP
1. Implement basic oscillators (sine, saw, square, noise)
2. Implement ADSR envelope
3. Implement filters (low-pass, high-pass)
4. Create DSP graph composition
5. Benchmark performance in real-time context

### Phase 3: Sequencing
1. Design pattern representation
2. Implement bach event scheduler integration
3. Create autonomous task primitives
4. Build pattern playback engine
5. Test with simple compositions

### Phase 4: Live Performance
1. Real-time audio engine integration (cpal)
2. MIDI input handling (midir)
3. Parameter automation system
4. Hot reload mechanism
5. Test with live performance scenarios

### Phase 5: Production Features
1. Offline rendering to file
2. Parallel rendering optimization
3. Effect library (reverb, delay, etc.)
4. Advanced modulation sources
5. Score export (MIDI, MusicXML)

## Relationship to Existing Work

### Sonic Pi
Live coding environment for music.
- **Cadenza advantage**: Type safety, dimensional analysis, native compilation

### TidalCycles
Pattern-based live coding.
- **Cadenza advantage**: Unified language, better tooling (LSP), native performance

### SuperCollider
Audio synthesis language and server.
- **Cadenza advantage**: Modern syntax, type system, better error messages

### Overtone (Clojure)
Clojure interface to SuperCollider.
- **Cadenza advantage**: Static typing, better performance, integrated DSP compilation

### Max/MSP, Pure Data
Visual programming for audio.
- **Cadenza advantage**: Text-based (version control friendly), type checking, reproducibility

## Conclusion

Using Cadenza as a music composition environment leverages its unique strengths:

- **Type safety**: Catch errors before they become audible glitches
- **Dimensional analysis**: Ensure time, frequency, and amplitude units are used correctly
- **Hybrid compilation**: Fast DSP, flexible control logic
- **Discrete event simulation**: Precise timing and task coordination
- **Modern tooling**: LSP, REPL, hot reload

The main challenges are:
1. Real-time performance constraints
2. Task coordination and event scheduling
3. FFI for audio/MIDI I/O
4. State management in DSP context

These challenges can be addressed through:
- Native DSP compilation
- Integration with bach for event scheduling
- Effect system for I/O
- Careful state management design

**Current Status**: Most foundational language features exist. Main dependencies are:
- **Task/async system** - New, needs design
- **DSP primitives** - New, needs FFI design
- **Musical time units** - Extension of existing unit system
- **Effect system** (Phase 4) - For audio I/O

**Timeline**: This is a medium-term goal (3-4 phases away) but worth documenting now to inform language design decisions around concurrency, effects, and FFI.

This document serves as a north star for ensuring Cadenza develops the capabilities needed for professional music composition and performance.
