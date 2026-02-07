# Makefile for Cadenza K Semantics

# K Framework commands
KOMPILE := kompile
KRUN := krun
KPROVE := kprove

# Main definition file
MAIN_DEF := cadenza.k

# Output directory for compiled definition
KOMPILED_DIR := cadenza-kompiled

# Test directory
TEST_DIR := tests

# Find all test files
TEST_FILES := $(wildcard $(TEST_DIR)/*.cdz)

.PHONY: all clean test kompile check-k help

# Default target
all: kompile

# Check if K is installed
check-k:
	@which $(KOMPILE) > /dev/null || (echo "Error: K framework not found. Please install K from https://kframework.org/" && exit 1)
	@echo "K framework found: $$(which $(KOMPILE))"

# Compile the K definition
kompile: check-k
	@echo "Compiling Cadenza K definition..."
	$(KOMPILE) $(MAIN_DEF) --syntax-module CADENZA-SYNTAX
	@echo "Compilation complete. Output in $(KOMPILED_DIR)/"

# Run all tests
test: kompile
	@echo "Running Cadenza K tests..."
	@for test in $(TEST_FILES); do \
		echo ""; \
		echo "=== Running $$test ==="; \
		$(KRUN) $$test || echo "Test failed: $$test"; \
	done
	@echo ""
	@echo "All tests completed."

# Run a specific test
run: kompile
	@if [ -z "$(FILE)" ]; then \
		echo "Usage: make run FILE=path/to/file.cdz"; \
		exit 1; \
	fi
	$(KRUN) $(FILE)

# Clean compiled artifacts
clean:
	rm -rf $(KOMPILED_DIR)
	@echo "Cleaned compiled artifacts."

# Show help
help:
	@echo "Cadenza K Semantics Makefile"
	@echo ""
	@echo "Targets:"
	@echo "  all       - Compile the K definition (default)"
	@echo "  kompile   - Compile the K definition"
	@echo "  test      - Run all test files"
	@echo "  run       - Run a specific file (usage: make run FILE=path/to/file.cdz)"
	@echo "  clean     - Remove compiled artifacts"
	@echo "  check-k   - Check if K framework is installed"
	@echo "  help      - Show this help message"
	@echo ""
	@echo "Examples:"
	@echo "  make kompile"
	@echo "  make test"
	@echo "  make run FILE=tests/01-literal-integer.cdz"
