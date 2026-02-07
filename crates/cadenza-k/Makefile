# Makefile for Cadenza K Framework Implementation

# K Framework tools
KOMPILE := kompile
KRUN := krun

# Directories
ROOT_DIR := $(shell git rev-parse --show-toplevel)
SEMANTICS_DIR := $(ROOT_DIR)/docs/semantics
TEST_DATA_DIR := $(ROOT_DIR)/crates/cadenza-compiler/test-data/semantics
BUILD_DIR := cadenza-kompiled

# Cadenza CLI for converting .cdz to AST
CADENZA := $(ROOT_DIR)/target/debug/cadenza

.PHONY: all kompile test clean help

# Default target
all: kompile

# Compile the K definition
kompile: cadenza.k
	@echo "Compiling Cadenza K definition..."
	$(KOMPILE) cadenza.k --backend llvm

# Run a single file (usage: make run FILE=path/to/file.ast)
run: kompile
	@if [ -z "$(FILE)" ]; then \
		echo "Error: FILE not specified. Usage: make run FILE=path/to/file.ast"; \
		exit 1; \
	fi
	$(KRUN) $(FILE)

# Extract semantics tests
extract-tests:
	@echo "Extracting tests from semantics documentation..."
	cd $(ROOT_DIR) && cargo xtask semantics extract

# Run all tests
test: kompile extract-tests
	@echo "Running K framework tests..."
	@./tests/run-tests.sh

# Build the Cadenza binary first (if needed)
build-cadenza:
	@echo "Building Cadenza CLI..."
	cd $(ROOT_DIR) && cargo build --bin cadenza

# Clean compiled artifacts
clean:
	rm -rf $(BUILD_DIR)
	rm -rf tests/output

# Help target
help:
	@echo "Cadenza K Framework Makefile"
	@echo ""
	@echo "Targets:"
	@echo "  all              - Compile K definition (default)"
	@echo "  kompile          - Compile K definition"
	@echo "  run FILE=<path>  - Run a single AST file"
	@echo "  extract-tests    - Extract test cases from semantics docs"
	@echo "  test             - Run all semantics tests"
	@echo "  build-cadenza    - Build the Cadenza CLI tool"
	@echo "  clean            - Remove compiled artifacts"
	@echo "  help             - Show this help message"
