# cadenza-tree Status

## Completed Features

- ✅ Two-layer architecture (green tree + red tree)
- ✅ String interning with O(1) comparisons
- ✅ Token caching and deduplication
- ✅ Node caching for structural sharing
- ✅ Optimized text() method (avoids allocation for single-token nodes)
- ✅ Source file tracking infrastructure
- ✅ Line number computation support
- ✅ Metadata system foundation

## Known Issues

### Critical: Checkpoint Implementation (25 test failures)

**Problem**: The `start_node_at` implementation doesn't correctly handle all cases of left-associative parsing.

**Current Status**: 358/383 tests passing (93.5%)

**Symptoms**:
- Tests fail with different CST output than expected
- Related to complex parsing scenarios (nested applications, parentheses, multiple operators)

**Root Cause**:
The checkpoint mechanism has a mismatch between when checkpoints are taken and when they're used:
1. A checkpoint is taken with N children at depth D
2. Parser continues, may push/pop nodes, finish nodes
3. When `start_node_at` is called, current node may have different number of children
4. The `children_count` from checkpoint no longer matches reality

**Current Implementation**:
```rust
pub struct Checkpoint {
    children_count: usize,  // Number of children when checkpoint was taken
    stack_depth: usize,     // For debugging/validation
}

pub fn start_node_at(&mut self, checkpoint: Checkpoint, kind: SyntaxKind) {
    // Works on current stack top
    let (_, current_children) = self.stack.last_mut().expect("no current node");
    // Clamp to prevent panics
    let safe_position = checkpoint.children_count.min(current_children.len());
    let children_to_move = current_children.split_off(safe_position);
    self.stack.push((kind, children_to_move));
}
```

**Workaround Applied**:
- Clamp `children_count` to actual children length to prevent panics
- This allows parsing to complete but may produce incorrect tree structure in edge cases
- 25 tests still fail (down from 26 originally)

**Failing Tests**:
- `ws_paren`, `ap_nested` - Complex application with whitespace
- `lit_multi_line` - Multi-line literals
- `index_*` - Index operator tests  
- `invalid_parse::error_recovery_*` - Error recovery scenarios
- `op_field_after_call`, `op_try_with_paren` - Operator combinations
- `ws_leading_newline` - Whitespace handling

**Next Steps for Full Fix**:
1. Deep investigation of Rowan 0.16 source code for checkpoint semantics
2. Possibly need event-based or position-based approach instead of children-count-based
3. May need to refactor parser to ensure checkpoint usage guarantees
4. Consider adding checkpoint validation/debugging during development

**Why This Is Hard**:
- Rowan's API is designed for incremental parsing
- Checkpoints allow parser to be non-linear (backtrack/rewrap)
- The interaction between stack depth changes and checkpoints is subtle
- Without Rowan source access, reverse-engineering correct semantics is challenging

## Future Architectural Improvements

### Slot Map Architecture (Deferred)

**Current**: Nodes use `Arc<SyntaxNode>` for parent pointers
**Proposed**: Use slot map with node IDs

**Benefits**:
- Smaller nodes (just an ID + Arc to shared state)
- Single Arc for entire tree's metadata
- More cache-friendly

**Tradeoffs**:
- Adds indirection on every node access
- Requires redesigning parent/child relationships
- All traversal code needs updates

**Status**: Deferred until after checkpoint bug is fixed

### Synthetic/Virtual Tokens

**Status**: Structure in place but not fully implemented

**TODO**:
- Add zero-width token support
- Ensure they don't affect text ranges
- Add tests for synthetic tokens

### AnyMap for Node Metadata

**Status**: Infrastructure exists but not exposed in API

**TODO**:
- Add `NodeMetadata::set_data/get_data` methods
- Implement AnyMap storage
- Add examples and tests

## Testing Status

- cadenza-tree: 24/24 tests pass ✅
- cadenza-syntax: 358/383 tests pass (93.5%) ⚠️
  - 25 failures due to checkpoint implementation producing incorrect tree structure in edge cases
  - All tests complete without panics
  - Failures are in complex parsing scenarios (nested applications, operators, whitespace handling)

## Performance Notes

- String interning makes token comparisons O(1)
- Single-token text() avoids allocation (common case)
- Token cache prevents duplicate allocations
- Node cache enables structural sharing

## Migration from Rowan

- ✅ API compatibility maintained
- ✅ GreenNodeBuilder API similar to Rowan
- ✅ SyntaxNode/SyntaxToken types match Rowan patterns
- ⚠️ Checkpoint semantics differ (causing test failures)

## Dependencies

- rustc-hash: For fast hash maps
- No external tree library dependencies (Rowan removed)
