#!/bin/bash
# Test runner for Cadenza K Framework implementation
# Runs all semantics tests through the K interpreter and tracks status

set -euo pipefail

# Get repository root
REPO_ROOT=$(git rev-parse --show-toplevel)
K_DIR="$REPO_ROOT/crates/cadenza-k"
TEST_DATA_DIR="$REPO_ROOT/crates/cadenza-compiler/test-data/semantics"
OUTPUT_DIR="$K_DIR/tests/output"
CADENZA="$REPO_ROOT/target/debug/cadenza"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Counters
TOTAL=0
PASSED=0
FAILED=0
ERROR=0

# Ensure output directory exists
mkdir -p "$OUTPUT_DIR"

# Check if K framework is installed
if ! command -v krun &> /dev/null; then
    echo -e "${RED}Error: K framework not found. Please install K framework first.${NC}"
    echo "See README.md for installation instructions."
    exit 1
fi

# Check if Cadenza binary exists
if [ ! -f "$CADENZA" ]; then
    echo -e "${YELLOW}Cadenza binary not found. Building...${NC}"
    cd "$REPO_ROOT" && cargo build --bin cadenza
fi

# Check if K definition is compiled
if [ ! -d "$K_DIR/cadenza-kompiled" ]; then
    echo -e "${YELLOW}K definition not compiled. Compiling...${NC}"
    cd "$K_DIR" && make kompile
fi

echo "Running Cadenza K Framework Tests"
echo "=================================="
echo ""

# Iterate through all test files
for cdz_file in "$TEST_DATA_DIR"/*.cdz; do
    [ -e "$cdz_file" ] || continue
    
    TOTAL=$((TOTAL + 1))
    basename=$(basename "$cdz_file" .cdz)
    expected_file="${cdz_file%.cdz}.expected"
    ast_file="$OUTPUT_DIR/${basename}.ast"
    output_file="$OUTPUT_DIR/${basename}.out"
    
    # Skip if no expected file
    if [ ! -f "$expected_file" ]; then
        echo -e "${YELLOW}⊘${NC} $basename (no expected file)"
        ERROR=$((ERROR + 1))
        continue
    fi
    
    # Convert .cdz to AST
    if ! "$CADENZA" ast "$cdz_file" > "$ast_file" 2>&1; then
        echo -e "${RED}✗${NC} $basename (AST conversion failed)"
        FAILED=$((FAILED + 1))
        continue
    fi
    
    # Run through K interpreter
    if ! krun "$ast_file" --directory "$K_DIR" > "$output_file" 2>&1; then
        echo -e "${RED}✗${NC} $basename (K execution failed)"
        FAILED=$((FAILED + 1))
        continue
    fi
    
    # For now, just mark as error since we're setting up infrastructure
    # In a full implementation, we'd compare output with expected
    echo -e "${YELLOW}⊘${NC} $basename (not yet implemented)"
    ERROR=$((ERROR + 1))
done

echo ""
echo "=================================="
echo "Test Results:"
echo "  Total:  $TOTAL"
echo -e "  ${GREEN}Passed: $PASSED${NC}"
echo -e "  ${RED}Failed: $FAILED${NC}"
echo -e "  ${YELLOW}Not Implemented: $ERROR${NC}"
echo ""

# Exit with success for now since we're just setting up
exit 0
