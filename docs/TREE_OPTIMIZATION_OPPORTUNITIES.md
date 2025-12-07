# Tree Optimization Opportunities

This document explores potential optimizations for the cadenza-tree crate, particularly around memory allocation and Arc usage.

## Current Architecture

### Green Tree (Immutable CST)
- Each `GreenNode` has its own `Arc<GreenNodeData>`
- Each `GreenToken` has its own `Arc<GreenTokenData>`
- Interning via cache deduplicates identical subtrees
- Hash-based lookup using hashbrown with FxHasher

### Red Tree (Typed Navigation)
- Each `SyntaxNode<L>` contains:
  - `green: GreenNode` (cloning increments Arc refcount)
  - `parent: Option<Arc<SyntaxNode<L>>>` (separate Arc for navigation)
  - Position/offset metadata

## Completed Optimizations

### ✅ String Interning Elimination
**Status**: Completed in this PR

**Changes**:
- Added `SyntaxText::interned()` method to extract underlying `InternedString`
- Eliminated ~15 allocation sites where `.to_string().as_str().into()` pattern was used
- Now using direct `text.interned()` for identifiers, operators, etc.

**Impact**:
- Eliminates double interning (text was already interned in SyntaxText)
- Reduces string allocations in hot paths (eval, type inference, IR generation)
- No API changes required - leveraged existing structure

**Files Modified**:
- `crates/cadenza-eval/src/eval.rs`: 4 allocation sites
- `crates/cadenza-eval/src/typeinfer.rs`: 2 allocation sites  
- `crates/cadenza-eval/src/ir/generator.rs`: 6 allocation sites
- `crates/cadenza-eval/src/special_form/*.rs`: 3 allocation sites

**Note**: String literals still require allocation when creating `Value::String(String)` - this is unavoidable without changing the Value enum.

## Future Optimization Opportunities

### 1. Slotmap-Based Arena (Deferred)

**Status**: Documented in STATUS.md, deferred for now

**Proposal**: Replace individual Arc allocations with a slotmap arena.

**Current Memory Pattern**:
```rust
// Each node/token gets its own Arc
struct GreenNode {
    inner: Arc<GreenNodeData>  // 16 bytes overhead per node
}
struct GreenToken {
    inner: Arc<GreenTokenData> // 16 bytes overhead per token
}
```

**Proposed Pattern**:
```rust
// All nodes stored in arena, referenced by ID
struct GreenNode {
    id: NodeId  // 4-8 bytes
}

struct TreeArena {
    data: Arc<TreeData>  // Single Arc for entire tree
}

struct TreeData {
    nodes: SlotMap<NodeId, GreenNodeData>,
    tokens: SlotMap<TokenId, GreenTokenData>,
}
```

**Benefits**:
1. **Smaller clones**: Cloning a tree just increments one Arc refcount instead of many
2. **Better cache locality**: Nodes stored contiguously in arena
3. **Reduced memory overhead**: One Arc overhead per tree instead of per node
4. **Generational indices**: SlotMap provides safe IDs with generation checking

**Tradeoffs**:
1. **Indirection cost**: Every node access requires arena lookup
2. **Lifetime complexity**: Arena must outlive all node references
3. **API changes**: Nodes need access to arena for all operations
4. **Breaking change**: Significant API redesign required

**Implementation Considerations**:
- Need to thread arena reference through all tree operations
- Interning cache would need to deduplicate based on arena contents
- Red tree parent pointers would still need separate Arc or arena IDs
- Consider hybrid approach: arena for green tree, Arc for red tree navigation

**When to Consider**:
- If profiling shows significant time in Arc cloning
- If memory usage from Arc overhead becomes problematic
- After other, simpler optimizations are exhausted

### 2. Reduced Arc Allocations in Red Tree

**Observation**: Red tree creates `Arc<SyntaxNode<L>>` for every parent pointer.

**Alternative Approaches**:
1. **Cursor-based navigation**: Store only current position, recompute parents
2. **Path-based**: Store Vec of indices from root instead of parent Arc
3. **Arena for red nodes too**: Extend slotmap to red tree

**Tradeoffs**: Each alternative trades memory for CPU or complexity.

### 3. Optimize Text Collection

**Current**: `SyntaxNode::text()` has fast path for single tokens, slow path allocates String.

**Opportunities**:
- Could use SmallVec for common small text cases
- Consider rope-like structure for very large nodes
- Profile to see if this is actually a bottleneck

### 4. Hash Computation Optimization

**Current**: Hashes computed recursively during node construction.

**Potential Improvements**:
- Incremental hash computation using rolling hash
- Skip hashing for very large nodes (already done for >3 children)
- Consider alternative hash algorithms (XXH3, CityHash)

## Benchmarking Recommendations

Before implementing any major optimizations:

1. **Profile first**: Use flamegraph/perf to identify actual bottlenecks
2. **Measure Arc overhead**: Count Arc clone operations in real workloads
3. **Memory profiling**: Measure actual memory usage patterns
4. **Microbenchmarks**: Create focused benchmarks for:
   - Tree cloning
   - Node navigation
   - Text extraction
   - Interning operations

## Decision Framework

When considering an optimization:

1. **Is it a measured bottleneck?** Profile data should drive decisions
2. **What's the complexity cost?** Simple optimizations first
3. **Breaking changes?** Prefer compatible optimizations
4. **Maintenance burden?** Consider long-term cost

## References

- [Rust Analyzer's Green Tree](https://github.com/rust-lang/rust-analyzer/blob/master/docs/dev/syntax.md)
- [Rowan Library](https://github.com/rust-analyzer/rowan) - Original inspiration
- [SlotMap Documentation](https://docs.rs/slotmap/latest/slotmap/)
- STATUS.md - Current architectural decisions
