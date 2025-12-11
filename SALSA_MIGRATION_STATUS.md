# Salsa Migration Status

This document tracks the progress of migrating Cadenza to use Salsa for incremental compilation.

**Last Updated**: 2025-12-11

## Overview

See [docs/SALSA_MIGRATION_PLAN.md](docs/SALSA_MIGRATION_PLAN.md) for the complete migration plan.

## Current Status: Planning Complete

The migration plan has been created and reviewed. Implementation has not yet started.

## Phase Status

### Phase 1: Foundation and Database Setup ⏳ Not Started

**Goal**: Add Salsa as a dependency and create the core database infrastructure.

**Status**: Not started

**Tasks**:
- [ ] Add Salsa 0.24 dependency to Cargo.toml
- [ ] Create `CadenzaDb` trait in `crates/cadenza-eval/src/db.rs`
- [ ] Create `CadenzaDbImpl` struct for CLI/testing
- [ ] Create thread-safe database wrapper for LSP
- [ ] Write basic unit tests for database creation
- [ ] Document database architecture

**Estimated Effort**: 1-2 days

---

### Phase 2: Source Tracking and Interner ⏳ Not Started

**Goal**: Migrate source text tracking and string interning to Salsa.

**Status**: Not started

**Prerequisites**: Phase 1 complete

**Tasks**:
- [ ] Define `SourceFile` input type
- [ ] Define `Identifier` interned type
- [ ] Migrate `InternedString` to use Salsa interning
- [ ] Update syntax nodes to reference SourceFile
- [ ] Add tests for interning behavior
- [ ] Verify existing tests still pass with adapter layer

**Estimated Effort**: 2-3 days

---

### Phase 3: Parsing as Tracked Functions ⏳ Not Started

**Goal**: Make parsing incremental - only re-parse files that changed.

**Status**: Not started

**Prerequisites**: Phase 2 complete

**Tasks**:
- [ ] Define `ParsedFile` tracked struct
- [ ] Create `parse_file` tracked function
- [ ] Define `ParseDiagnostic` accumulator
- [ ] Emit diagnostics during parsing
- [ ] Add tests for parse memoization
- [ ] Verify diagnostic accumulation works
- [ ] Benchmark parsing performance

**Estimated Effort**: 3-4 days

---

### Phase 4: Evaluation and Compiler State ⏳ Not Started

**Goal**: Convert the evaluator to use Salsa tracked functions.

**Status**: Not started

**Prerequisites**: Phase 3 complete

**Tasks**:
- [ ] Define `CompiledModule` tracked struct
- [ ] Create `evaluate_module` tracked function
- [ ] Handle macro expansion (decide on strategy)
- [ ] Migrate `Env` to persistent data structure
- [ ] Add tests for module evaluation
- [ ] Verify macro expansion works incrementally
- [ ] Test definition dependency tracking

**Estimated Effort**: 5-7 days

---

### Phase 5: Type Inference with Salsa ⏳ Not Started

**Goal**: Make type checking incremental.

**Status**: Not started

**Prerequisites**: Phase 4 complete

**Tasks**:
- [ ] Define `infer_function_type` query
- [ ] Implement `type_at_position` query for LSP
- [ ] Create `collect_constraints` query
- [ ] Define `TypeError` accumulator
- [ ] Add tests for incremental type checking
- [ ] Verify type errors accumulate correctly
- [ ] Benchmark type checking performance

**Estimated Effort**: 4-5 days

---

### Phase 6: LSP Integration ⏳ Not Started

**Goal**: Wire the LSP server to query the Salsa database.

**Status**: Not started

**Prerequisites**: Phase 5 complete

**Tasks**:
- [ ] Create `LspDatabase` wrapper with Mutex
- [ ] Implement LSP hover using queries
- [ ] Implement LSP completion using queries
- [ ] Handle `did_change` notifications
- [ ] Implement `did_open`/`did_close` handlers
- [ ] Add LSP integration tests
- [ ] Benchmark LSP query latency

**Estimated Effort**: 3-4 days

---

### Phase 7: Optimization and Tuning ⏳ Not Started

**Goal**: Fine-tune Salsa configuration for optimal performance.

**Status**: Not started

**Prerequisites**: Phase 6 complete

**Tasks**:
- [ ] Configure durability for stable inputs
- [ ] Optimize query granularity based on profiling
- [ ] Implement query groups for organization
- [ ] Add performance logging
- [ ] Benchmark against current implementation
- [ ] Document performance characteristics
- [ ] Write optimization guide

**Estimated Effort**: 2-3 days

---

## Total Progress

- **Phases Complete**: 0 / 7
- **Total Tasks Complete**: 0 / 58
- **Estimated Remaining Effort**: 20-28 days

## Key Decisions Made

1. **Target Salsa Version**: 0.24.0 (latest stable)
2. **Migration Strategy**: Incremental, phase-by-phase
3. **Feature Flag Strategy**: Build new code path alongside existing
4. **Macro Expansion**: Separate tracked query (Option B)
5. **Environment**: Migrate to persistent data structures (im-rc)

## Key Decisions Pending

1. Should we keep the old evaluation path as a fallback?
2. How to handle WASM builds with Salsa?
3. What's the priority: LSP performance or incremental compilation?
4. Should we wait for any upcoming Salsa releases?

## Risks and Issues

### Current Risks

1. **Performance Overhead**: Salsa adds tracking overhead
   - **Mitigation**: Profile and optimize critical paths
   - **Status**: Will monitor during implementation

2. **Macro Expansion Complexity**: Current macro system mutates state
   - **Mitigation**: Prototype early, get feedback
   - **Status**: Will address in Phase 4

3. **Learning Curve**: Team needs to learn Salsa
   - **Mitigation**: Good documentation, examples
   - **Status**: Documentation in progress

### Issues

None currently.

## Resources

- [Salsa Migration Plan](docs/SALSA_MIGRATION_PLAN.md)
- [Salsa Repository](https://github.com/salsa-rs/salsa)
- [Salsa Book](https://salsa-rs.github.io/salsa)
- [Salsa Calc Example](https://github.com/salsa-rs/salsa/tree/master/examples/calc)

## Notes

- The plan was created by analyzing the Salsa calc example and current Cadenza architecture
- Each phase builds on the previous and can be validated independently
- The migration preserves all existing functionality while adding incrementality
- LSP integration in Phase 6 is where the biggest user-visible improvements will appear

## Next Steps

1. Review this plan with the team
2. Address any questions or concerns
3. Get approval to proceed
4. Create feature branch: `feature/salsa-migration`
5. Start Phase 1: Foundation and Database Setup
