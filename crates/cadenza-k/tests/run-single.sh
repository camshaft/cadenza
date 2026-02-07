#!/bin/bash
# Run a single .cdz file through the K framework interpreter
# Usage: ./run-single.sh path/to/file.cdz

set -euo pipefail

if [ $# -ne 1 ]; then
    echo "Usage: $0 <file.cdz>"
    exit 1
fi

CDZ_FILE="$1"
REPO_ROOT=$(git rev-parse --show-toplevel)
K_DIR="$REPO_ROOT/crates/cadenza-k"
CADENZA="$REPO_ROOT/target/debug/cadenza"

# Check if K framework is installed
if ! command -v krun &> /dev/null; then
    echo "Error: K framework not found. Please install K framework first."
    echo "See README.md for installation instructions."
    exit 1
fi

# Check if Cadenza binary exists
if [ ! -f "$CADENZA" ]; then
    echo "Cadenza binary not found. Building..."
    cd "$REPO_ROOT" && cargo build --bin cadenza
fi

# Check if K definition is compiled
if [ ! -d "$K_DIR/cadenza-kompiled" ]; then
    echo "K definition not compiled. Compiling..."
    cd "$K_DIR" && make kompile
fi

# Convert .cdz to AST
AST_FILE=$(mktemp --suffix=.ast)
trap "rm -f $AST_FILE" EXIT

echo "Converting $CDZ_FILE to AST..."
"$CADENZA" ast "$CDZ_FILE" > "$AST_FILE"

echo "AST:"
cat "$AST_FILE"
echo ""

echo "Running through K interpreter..."
krun "$AST_FILE" --directory "$K_DIR"
