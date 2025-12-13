# Salsa Migration Status

This document tracks the progress of the Salsa migration across all phases. See `SALSA_MIGRATION.md` for the complete migration plan.

## Phase 1: Foundation ✅ COMPLETE

**Duration**: 1-2 days  
**Status**: ✅ Complete

### Tasks

- [x] Add `salsa = "0.24"` to Cargo.toml
- [x] Create `CadenzaDb` trait
- [x] Create `CadenzaDbImpl` for CLI/testing
- [x] Document thread-safety considerations for LSP

### Implementation

- Database infrastructure in `crates/cadenza-eval/src/db.rs`
- Database trait properly annotated with `#[salsa::db]`
- Implementation struct includes `salsa::Storage<Self>`
- Tests verify database creation and trait implementation

## Phase 2: Source Tracking ✅ COMPLETE

**Duration**: 2-3 days  
**Status**: ✅ Complete  
**Completed**: 2024-12-13

### Tasks

- [x] Define `SourceFile` input struct
- [x] Define `Identifier` interned type
- [x] Add comprehensive tests for SourceFile operations
- [x] Add comprehensive tests for Identifier interning
- [x] Update documentation

### Implementation

- `SourceFile`: Salsa input with `path` and `text` fields
- `Identifier`: Salsa interned type for deduplicated identifiers
- Both types exported from `crates/cadenza-eval/src/db.rs`
- Full test coverage including mutation tests for SourceFile
- Doc examples demonstrate usage with proper trait imports

### Key Learnings

- Salsa's `Setter` trait must be imported to use `.to()` method on input setters
- Interned types don't automatically derive `Debug`, so tests should use `assert!(a == b)` instead of `assert_eq!`
- Doc examples need explicit `use salsa::Setter;` import

## Phase 3: Parsing (NOT STARTED)

**Duration**: 3-4 days  
**Status**: ⏳ Not Started

### Tasks

- [ ] Define `ParsedFile` tracked struct
- [ ] Create `parse_file` tracked function
- [ ] Setup diagnostic accumulator
- [ ] Integrate with existing parser from `cadenza-syntax`
- [ ] Tests for parsing queries

## Phase 4: Evaluation (NOT STARTED)

**Duration**: 5-7 days  
**Status**: ⏳ Not Started

### Tasks

- [ ] Define `CompiledModule` tracked struct
- [ ] Create `evaluate_module` tracked function
- [ ] Handle macro expansion as separate query
- [ ] Migrate `Env` to persistent data structure
- [ ] Make evaluation pure (no mutations)

### Challenges

- Current evaluator mutates state
- Need to convert to pure functions using persistent data structures
- Consider using `im` crate for efficient immutable collections

## Phase 5: Type Inference (NOT STARTED)

**Duration**: 4-5 days  
**Status**: ⏳ Not Started

### Tasks

- [ ] Define `infer_function_type` query
- [ ] Implement `type_at_position` query for LSP
- [ ] Use accumulators for type errors
- [ ] Wire existing TypeInferencer into Salsa queries

## Phase 6: LSP Integration (NOT STARTED)

**Duration**: 3-4 days  
**Status**: ⏳ Not Started

### Tasks

- [ ] Create thread-safe `LspDatabase` wrapper with Mutex
- [ ] Implement LSP handlers as database queries
- [ ] Handle file change notifications
- [ ] Wire up hover, completion, diagnostics

## Phase 7: Optimization (NOT STARTED)

**Duration**: 2-3 days  
**Status**: ⏳ Not Started

### Tasks

- [ ] Configure durability for rarely-changing inputs
- [ ] Optimize query granularity
- [ ] Add performance logging
- [ ] Profile and identify bottlenecks

## Total Progress

**Completed Phases**: 2 / 7  
**Estimated Total Duration**: 20-28 days  
**Time Spent**: ~2-3 days

## Next Steps

1. Begin Phase 3: Parsing
   - Study `cadenza-syntax` parser API
   - Design `ParsedFile` struct to hold CST
   - Implement `parse_file` tracked function
   - Create diagnostic accumulator for parse errors
