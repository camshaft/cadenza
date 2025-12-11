# Salsa Migration Documentation

This directory contains comprehensive documentation for migrating Cadenza to use the Salsa incremental computation framework.

## Overview

Cadenza is planning to migrate from its current direct evaluation model to using [Salsa](https://github.com/salsa-rs/salsa), a framework for incremental computation. This will enable:

- **Fast incremental compilation**: Only recompute what changes
- **Efficient LSP queries**: Near-instant hover, completion, etc.
- **Better architecture**: Cleaner separation of concerns
- **Future features**: Watch mode, parallel compilation, etc.

## Document Index

### 📋 [SALSA_MIGRATION_PLAN.md](SALSA_MIGRATION_PLAN.md)
**The comprehensive migration plan** - Start here!

Contains:
- Executive summary and background
- Detailed 7-phase migration plan
- Timeline estimates (20-28 days)
- Success metrics and risk assessment
- Key design decisions

**Target audience**: Project leads, architects, anyone needing the big picture

---

### 📊 [SALSA_COMPARISON.md](SALSA_COMPARISON.md)
**Side-by-side comparison of current vs Salsa architecture**

Contains:
- Code comparisons (before/after)
- Performance comparisons with concrete numbers
- Memory and complexity analysis
- Risk assessment
- Recommendation summary

**Target audience**: Decision makers, reviewers, anyone evaluating the migration

---

### 📖 [SALSA_ARCHITECTURE_EXAMPLE.md](SALSA_ARCHITECTURE_EXAMPLE.md)
**Concrete code examples showing the new architecture**

Contains:
- Complete examples of all Salsa patterns
- Database setup
- Input/tracked/interned types
- LSP integration patterns
- End-to-end usage examples

**Target audience**: Developers implementing the migration

---

### 🔖 [SALSA_QUICK_REFERENCE.md](SALSA_QUICK_REFERENCE.md)
**Quick reference guide for developers**

Contains:
- Core Salsa concepts
- Common patterns and idioms
- Common mistakes to avoid
- Performance tips
- Debugging techniques
- Testing patterns

**Target audience**: Developers working with Salsa after migration

---

### 📈 [../SALSA_MIGRATION_STATUS.md](../SALSA_MIGRATION_STATUS.md)
**Live status tracker for the migration**

Contains:
- Current phase status
- Task checklists
- Progress tracking
- Issues and risks
- Next steps

**Target audience**: Anyone tracking migration progress

---

## Quick Start

1. **Want to understand why?** → Read [SALSA_COMPARISON.md](SALSA_COMPARISON.md)
2. **Want to see the plan?** → Read [SALSA_MIGRATION_PLAN.md](SALSA_MIGRATION_PLAN.md)
3. **Want to implement it?** → Read [SALSA_ARCHITECTURE_EXAMPLE.md](SALSA_ARCHITECTURE_EXAMPLE.md)
4. **Want a quick reference?** → Read [SALSA_QUICK_REFERENCE.md](SALSA_QUICK_REFERENCE.md)
5. **Want to track progress?** → Read [../SALSA_MIGRATION_STATUS.md](../SALSA_MIGRATION_STATUS.md)

## Key Takeaways

### What is Salsa?

Salsa is a framework for incremental computation. You define your program as a set of **queries** (pure functions), and Salsa:
- Automatically memoizes results
- Tracks dependencies between queries
- Only recomputes what's necessary when inputs change

It's like a smart build system for your compiler.

### Why Migrate?

Current issues:
- ❌ Every LSP query requires full re-evaluation (~65ms)
- ❌ Changing one function requires recompiling everything
- ❌ No caching or memoization

With Salsa:
- ✅ LSP queries are near-instant (~0.1ms) with caching
- ✅ Incremental compilation is 10-100x faster
- ✅ Automatic memoization and dependency tracking

### How Long?

Estimated 20-28 days full-time work, broken into 7 phases:
1. Foundation (1-2 days)
2. Inputs & Interning (2-3 days)
3. Parsing (3-4 days)
4. Evaluation (5-7 days) ← Most complex
5. Type Inference (4-5 days)
6. LSP Integration (3-4 days)
7. Optimization (2-3 days)

Each phase is independently useful and can be validated before moving to the next.

### Is it Worth It?

**Yes!** The benefits are significant:
- 100-1000x faster LSP queries
- 10-100x faster incremental compilation
- Cleaner, more maintainable architecture
- Enables future features (watch mode, parallel compilation)

The costs are manageable:
- 10-20% slower initial compilation (one-time cost)
- Team learning curve (offset by good docs)
- Migration effort (phased approach reduces risk)

## Migration Approach

The migration follows a **phased, incremental approach**:

1. **Build alongside existing code**: New Salsa code path runs parallel to old code
2. **Feature flags**: Enable new code path when ready
3. **Extensive testing**: Each phase has its own test suite
4. **Rollout strategy**: Alpha → Beta → Stable → Remove old code

This minimizes risk and allows for iterative refinement.

## Key Patterns from Salsa Calc Example

The migration is based on patterns from the [Salsa calc example](https://github.com/salsa-rs/salsa/tree/master/examples/calc):

```rust
// Input - data from outside
#[salsa::input]
pub struct SourceFile {
    #[returns(ref)]
    pub text: String,
}

// Tracked function - memoized pure function
#[salsa::tracked]
pub fn parse_file(db: &dyn Db, source: SourceFile) -> ParsedFile<'_> {
    // Parse implementation
}

// Accumulator - collect diagnostics
#[salsa::accumulator]
pub struct Diagnostic {
    pub message: String,
}

// Database - ties everything together
#[salsa::db]
pub trait CadenzaDb: salsa::Database {}
```

## External Resources

- [Salsa GitHub Repository](https://github.com/salsa-rs/salsa)
- [Salsa Book](https://salsa-rs.github.io/salsa)
- [Salsa Calc Example](https://github.com/salsa-rs/salsa/tree/master/examples/calc)
- [rust-analyzer Architecture](https://github.com/rust-lang/rust-analyzer/blob/master/docs/dev/architecture.md) - Uses Salsa in production

## Questions?

For questions about the migration:
1. Check [SALSA_QUICK_REFERENCE.md](SALSA_QUICK_REFERENCE.md) for common questions
2. Review [SALSA_MIGRATION_PLAN.md](SALSA_MIGRATION_PLAN.md) for detailed explanations
3. Look at [SALSA_ARCHITECTURE_EXAMPLE.md](SALSA_ARCHITECTURE_EXAMPLE.md) for code examples

## Contributing to Migration

To contribute to the migration:
1. Review the plan in [SALSA_MIGRATION_PLAN.md](SALSA_MIGRATION_PLAN.md)
2. Check [../SALSA_MIGRATION_STATUS.md](../SALSA_MIGRATION_STATUS.md) for current status
3. Pick a task from the current phase
4. Refer to [SALSA_ARCHITECTURE_EXAMPLE.md](SALSA_ARCHITECTURE_EXAMPLE.md) for implementation guidance
5. Use [SALSA_QUICK_REFERENCE.md](SALSA_QUICK_REFERENCE.md) while coding

## Document Maintenance

These documents should be updated:
- **During migration**: Update [../SALSA_MIGRATION_STATUS.md](../SALSA_MIGRATION_STATUS.md) after completing tasks
- **After migration**: Update [SALSA_QUICK_REFERENCE.md](SALSA_QUICK_REFERENCE.md) with new patterns discovered
- **When plans change**: Update [SALSA_MIGRATION_PLAN.md](SALSA_MIGRATION_PLAN.md) with refined estimates

## Timeline

**Plan Created**: 2025-12-11  
**Implementation Start**: TBD  
**Target Completion**: TBD (20-28 days after start)  
**Status**: Planning complete, implementation not started

See [../SALSA_MIGRATION_STATUS.md](../SALSA_MIGRATION_STATUS.md) for live updates.
