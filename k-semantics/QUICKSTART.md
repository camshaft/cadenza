# Quick Start Guide - Cadenza K Framework

This guide will get you up and running with the Cadenza K implementation in 5 minutes.

## Prerequisites

You need the K framework installed. If you don't have it:

```bash
# Quick install (Ubuntu/Debian)
bash <(curl -s https://kframework.org/install)
```

Or follow the [official installation guide](https://kframework.org/docs/user_manual/#installation).

## Step 1: Compile the K Definition

```bash
cd k-semantics
make kompile
```

This compiles the Cadenza semantics into an executable interpreter. It takes about 30-60 seconds.

## Step 2: Run Your First Program

Try the "Hello World" example:

```bash
make run FILE=tests/01-literal-integer.cdz
```

You should see output like:
```
<k>
  'Integer ( 42 )
</k>
```

## Step 3: Try More Examples

Run all the test programs:

```bash
make test
```

This will run 15 test programs demonstrating various language features.

## Step 4: Write Your Own Program

Create a new file `my-program.cdz`:

```k
'Let(x, 'Integer(10)) ;
'Let(y, 'Integer(20)) ;
'Apply('Op(+), 'Ident(x), 'Ident(y))
```

Run it:

```bash
make run FILE=my-program.cdz
```

Expected output:
```
<k>
  'Integer ( 30 )
</k>
```

## Understanding the Output

K shows the final state of the program. The `<k>` cell contains the result value.

- `'Integer(42)` = integer value 42
- `'String("hello")` = string value "hello"
- `'Bool(true)` = boolean value true
- `'Rational(3, 2)` = rational value 3/2

## Common Program Patterns

### Variable Binding
```k
'Let(name, 'String("Alice")) ;
'Ident(name)
```

### Arithmetic
```k
'Apply('Op(+), 'Integer(2), 'Integer(3))
```

### Function Definition
```k
'Fn(double, (x, .Ids), 
    'Apply('Op(*), 'Ident(x), 'Integer(2)))
```

### Function Call
```k
'Apply('Ident(double), ('Integer(5), .Exprs))
```

### Complete Example: Factorial
```k
'Fn(factorial, (n, .Ids),
  'Match('Ident(n),
    ('MatchArm('Integer(0), 'Integer(1)),
     'MatchArm(n, 
       'Apply('Op(*), 
              'Ident(n),
              'Apply('Ident(factorial),
                     ('Apply('Op(-), 'Ident(n), 'Integer(1)), 
                      .Exprs)))),
     .MatchArms))) ;
'Apply('Ident(factorial), ('Integer(5), .Exprs))
```

## Next Steps

- Read [CONVERSION.md](CONVERSION.md) to learn the parenthesized AST format
- Read [GUIDE.md](GUIDE.md) for detailed semantics documentation
- Check [STATUS.md](STATUS.md) to see what's implemented
- Look at test files in `tests/` for more examples

## Debugging Tips

### View Detailed Execution

```bash
krun --debug my-program.cdz
```

### Search All Execution Paths

```bash
krun --search my-program.cdz
```

### Limit Execution Depth

```bash
krun --depth 10 my-program.cdz
```

## Troubleshooting

### "kompile: command not found"
K is not installed or not in PATH. Install K and ensure it's in your PATH.

### Compilation errors
Check that all `.k` files have correct syntax. Run `kompile cadenza.k` manually to see detailed errors.

### Runtime errors
The program likely has incorrect parenthesization or missing list terminators. Check that:
- Every `(` has a matching `)`
- Lists end with `.Ids`, `.Exprs`, etc.
- Single quotes around constructors: `'Integer` not `Integer`

### Unexpected output
K shows the final configuration state. If the result isn't what you expect:
1. Check your program's syntax
2. Add intermediate let bindings to see values at each step
3. Use `--debug` flag to see step-by-step execution

## Help and Resources

- **K Framework docs**: https://kframework.org/docs/
- **K Tutorial**: https://kframework.org/k-distribution/k-tutorial/
- **Issues**: Report bugs in the Cadenza repository
- **Community**: Join the K framework community for help

## Converting from Cadenza Concrete Syntax

If you have Cadenza code in concrete syntax (e.g., `let x = 42`), you need to convert it to parenthesized AST format. See [CONVERSION.md](CONVERSION.md) for a complete conversion guide.

A future tool will automate this:
```bash
# Coming soon
cadenza ast input.cdz -o output.k
```

## Contributing

Want to add features to the K implementation? See:
- [GUIDE.md](GUIDE.md) - Implementation guide
- [STATUS.md](STATUS.md) - What needs to be done
- [Contributing guidelines](../AGENTS.md) - General project guidelines

Have fun exploring formal semantics with K! 🎵
