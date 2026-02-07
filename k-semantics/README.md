# Cadenza K Framework Implementation

This directory contains the formal semantics of the Cadenza language implemented in the K framework.

## About K Framework

[K](https://kframework.org/) is a rewrite-based executable semantic framework where programming languages, calculi, and formal models can be defined in a formal and executable way. K generates parsers, interpreters, compilers, semantic-based debuggers, state-space explorers, and model checkers from a single formal definition.

## Installation

### Installing K Framework

To use this K definition, you need to have K installed. Follow the [official installation guide](https://kframework.org/docs/user_manual/#installation).

Quick install on Ubuntu/Debian:
```bash
bash <(curl -s https://kframework.org/install)
```

Or build from source:
```bash
git clone https://github.com/runtimeverification/k
cd k
mvn package
export PATH="$(pwd)/k-distribution/target/release/k/bin:$PATH"
```

## Structure

- `cadenza.k` - Main K definition combining all modules
- `syntax.k` - Syntax definitions for parenthesized AST input
- `configuration.k` - Runtime configuration (cells for state)
- `semantics/` - Semantic modules organized by feature
  - `literals.k` - Literal evaluation
  - `variables.k` - Variable bindings and scope
  - `operators.k` - Operator semantics
  - `functions.k` - Function definition and application
  - `control-flow.k` - Match expressions and control flow
- `tests/` - Test programs in parenthesized AST format

## Usage

### Kompiling the Definition

Compile the K definition:
```bash
cd k-semantics
kompile cadenza.k
```

This generates an interpreter from the formal semantics.

### Running Programs

Run a program written in parenthesized AST format:
```bash
krun program.cdz
```

Example program file (`hello.cdz`):
```k
'Literal'('String'("hello"))
```

### Testing

Run the test suite:
```bash
make test
```

## Input Format: Parenthesized AST

Since K doesn't support Pratt parsing (which Cadenza uses for its concrete syntax), programs must be provided in parenthesized AST format. Each syntactic construct is represented as a K term.

### Examples

#### Literals
```k
# Integer
'Literal'('Integer'(42))

# Float
'Literal'('Float'(3.14))

# String
'Literal'('String'("hello"))

# Boolean
'Literal'('Bool'(true))
```

#### Variables
```k
# Let binding
'Let'('x', 'Literal'('Integer'(42)))

# Variable reference
'Ident'('x')
```

#### Operations
```k
# Addition
'Apply'('Op'('+'), 'Literal'('Integer'(1)), 'Literal'('Integer'(2)))

# Comparison
'Apply'('Op'('>'), 'Literal'('Integer'(5)), 'Literal'('Integer'(3)))
```

#### Functions
```k
# Function definition
'Fn'('double', ['x'], 'Apply'('Op'('*'), 'Ident'('x'), 'Literal'('Integer'(2))))

# Function call
'Apply'('Ident'('double'), 'Literal'('Integer'(5)))
```

## Converting from Cadenza Concrete Syntax

To convert from Cadenza's concrete syntax to parenthesized AST format, you can use the `cadenza ast` command (to be implemented):

```bash
cadenza ast input.cdz -o output.k
```

Or use the language server to extract the AST programmatically.

## Documentation

For detailed semantics documentation, see the [docs/semantics](../docs/semantics) directory, which contains the executable specification that this K implementation is based on.

## Development

### Adding New Language Features

1. Add syntax definition to `syntax.k`
2. Add semantic rules to the appropriate file in `semantics/`
3. Add test cases to `tests/`
4. Re-kompile and test

### Debugging

Use K's debugging features:
```bash
krun --debug program.cdz
```

Or use the search command to explore all possible execution paths:
```bash
krun --search program.cdz
```

## References

- [K Framework Documentation](https://kframework.org/docs/)
- [K Tutorial](https://kframework.org/k-distribution/k-tutorial/)
- [Cadenza Semantics](../docs/semantics/)
