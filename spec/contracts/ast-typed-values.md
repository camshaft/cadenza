# Frozen Contract - AST Typed Values

> **FROZEN CONTRACT.** This document pins the TYPED MAPPING: the one binary AST tree that each typed
> value is written as, and how a decoder reads that tree back. A typed value is a boolean, an integer,
> a binary floating-point value, a character, a text, a byte sequence, the unit, an optional value, a
> list, a tuple, a map, a record, a wrapper or an enumeration variant. [ast-binary-format.md](./ast-binary-format.md)
> pins the bytes of every tree; this document pins which tree a typed value is, so that a program
> outside Cadenza can write and read typed values as binary ASTs from the two documents alone and get
> the same bytes as the reference implementation. It is versioned and changed only by the coordinated
> act the constitution requires of a frozen contract.
>
> RFC-2119 key words are normative. Each row below states the tree for one kind of typed value and
> holds a worked byte example; the reference implementation's tests check each example.

## Purpose And Scope

[ast-binary-format.md](./ast-binary-format.md) defines every tree's bytes but not which tree stands
for a record, a list or an optional value, which left the reference implementation as the only
definition of the mapping. This document is that mapping: one row for each kind of typed value, the
rules a decoder follows, and the leaf kinds the mapping writes.

## Conventions

### Tree Notation

A tree is written in parentheses: `(a b c)` is a list whose children are `a`, `b` and `c` in order,
and `()` is the list with no children. Inside a tree, `<list>`, `<tuple>`, `<record>` and `<map>` are
atoms of the list, tuple, record and map constructor heads (kinds `20`, `21`, `22` and `23`), `<=>` is
an atom of the field-pair head (kind `25`), a quoted text such as `"x"` is an atom of a string leaf
(kind `7`), a bare word such as `Circle` is an atom of a name leaf (kind `10`), and a number is an
atom of a decimal integer leaf.

### Worked Examples Are Whole Binary ASTs

Each worked example is the whole binary AST an encoder writes for one value, as hexadecimal bytes:
`H` for the eight-byte header `63 64 7A 61 73 74 00 01`, then the leaf pool, the structure pool and
the root, in the canonical order of [ast-binary-format.md](./ast-binary-format.md). A long example
puts the leaf pool on its first line and the structure pool and the root on the lines after it.

### The Shortest Decimal Of A Binary Floating-Point Value

The shortest decimal of a finite binary floating-point value is the decimal digit string with the
fewest digits that reads back to that value when rounded to the nearest value of its format, taking,
where two such strings of that length exist, the one nearer the exact value. Its significand is that
digit string read as an integer, and its exponent is the power of ten of the string's last digit, so
that `1.5` is the significand `15` with the exponent `-1`, `0.1` is `1` with `-1`, and `300` is `3`
with `2`. The shortest decimal of zero is the empty significand with the exponent `0`.

### The Kinds Of Typed Value

A list is a sequence of elements of one type. A tuple is a fixed-length sequence of positional
parts, and a value of a named type declared with a list of positional parts is a tuple too, whether
the list holds none of them or two or more. A map is a set of entries, each a key and a value. A
record is a value of a named type declared with named fields, whatever count of them it declares,
none among them. A wrapper is a value of a named type whose list of positional parts holds exactly
one. A named unit is the one value of a named type declared with neither named fields nor a list of
positional parts, by its name alone. An enumeration variant is one alternative of a type whose values
are named alternatives, with no payload, one payload value, positional parts or named fields. A byte
sequence is a value its type declares as raw bytes; a list of small integers is a list, under the
list row.

## The Typed Values

### A Boolean Is Its Boolean Leaf

A boolean MUST be written as an atom of the boolean leaf of its value, kind `8` for false and kind `9`
for true.

Examples: true is `H 01 09 01 00 00 00`, and false is `H 01 08 01 00 00 00`.

### An Integer Is A Decimal Integer Leaf

An integer of any width MUST be written as an atom of a decimal integer leaf whose magnitude is the
integer's absolute value, kind `0` for zero and every positive integer and kind `3` for every negative
integer.

Examples: 300 is `H 01 00 02 01 2C 01 00 00 00`, -1 is `H 01 03 01 01 01 00 00 00`, and 0 is
`H 01 00 00 01 00 00 00`.

### A Whole 64-Bit Binary Floating-Point Value Is Its Integer Decimal

A finite 64-bit binary floating-point value that is a whole number MUST be written as an atom of a
float leaf whose exponent is zero and whose significand is the value's absolute value.

Example: the 64-bit value 300 is `H 01 06 00 00 00 00 00 00 00 00 00 02 01 2C 01 00 00 00`.

### A Fractional 64-Bit Binary Floating-Point Value Is Its Shortest Decimal

A finite 64-bit binary floating-point value that is not a whole number MUST be written as an atom of
a float leaf whose significand and exponent are those of the value's shortest decimal.

Examples: the 64-bit value 1.5 is `H 01 06 00 FF FF FF FF FF FF FF FF 01 0F 01 00 00 00`, and -0.25
is `H 01 06 01 FF FF FF FF FF FF FF FE 01 19 01 00 00 00`.

### A 32-Bit Binary Floating-Point Value Is Its Shortest Decimal

A finite 32-bit binary floating-point value MUST be written as an atom of a float leaf whose
significand and exponent are those of the value's shortest decimal, whether or not it is a whole
number.

Examples: the 32-bit value 0.1 is `H 01 06 00 FF FF FF FF FF FF FF FF 01 01 01 00 00 00`, and the
32-bit value 300 is `H 01 06 00 00 00 00 00 00 00 00 02 01 03 01 00 00 00`, where the 64-bit value 300
has the significand 300 and the exponent 0.

### The Sign Of A Float Leaf Is The Value's Sign Bit

The negative flag of the float leaf written for a finite binary floating-point value MUST be set
exactly when the value's sign bit is set, so that negative zero is a negative float leaf with an
empty significand.

Example: the 64-bit negative zero is `H 01 06 01 00 00 00 00 00 00 00 00 00 01 00 00 00`.

### A Not-A-Number Value Is The Not-A-Number Leaf

A not-a-number value of either binary floating-point format MUST be written as an atom of the
not-a-number leaf, kind `17`, whatever its sign and payload.

Example: `H 01 11 01 00 00 00`.

### An Infinity Is Its Infinity Leaf

A positive infinity of either binary floating-point format MUST be written as an atom of the
positive-infinity leaf, kind `18`.

A negative infinity of either binary floating-point format MUST be written as an atom of the
negative-infinity leaf, kind `19`.

Examples: positive infinity is `H 01 12 01 00 00 00`, and negative infinity is `H 01 13 01 00 00 00`.

### A Character Is A Character Leaf

A character, one Unicode scalar value, MUST be written as an atom of a character leaf, kind `13`,
holding that scalar.

Example: the character U+00E9 is `H 01 0D 02 C3 A9 01 00 00 00`.

### A Text Is A String Leaf

A text MUST be written as an atom of a string leaf, kind `7`, holding the text's UTF-8 bytes as given,
without normalization.

Examples: the text `hi` is `H 01 07 02 68 69 01 00 00 00`, and the text of U+0065 followed by U+0301 is
`H 01 07 03 65 CC 81 01 00 00 00`, not its normalized form.

### A Byte Sequence Is A Bytes Leaf

A byte sequence MUST be written as an atom of a bytes leaf, kind `11`, holding its bytes.

Example: the bytes `00 FF` are `H 01 0B 02 00 FF 01 00 00 00`.

### A Unit Is The Empty List

A unit, the unit value or a named unit, MUST be written as the empty list `()`.

Example: `H 00 01 01 00 00`.

### An Absent Optional Value Is The Empty List

An absent optional value MUST be written as the empty list `()`.

Example: `H 00 01 01 00 00`, the same bytes as the unit.

### A Present Optional Value Is A List Of One

A present optional value MUST be written as a list of one child, the tree of the value it holds.

Example: the optional value holding 5 is `(5)`, `H 01 00 01 05 02 00 00 01 01 00 01`.

### A List Is The List Constructor Head Then Its Elements

A list MUST be written as a list whose first child is an atom of the list constructor head, kind `20`,
followed by the tree of each element in order.

Example: the list of 1 and 2 is `(<list> 1 2)`,
`H 03 14 00 01 01 00 01 02 04 00 00 00 01 00 02 01 03 00 01 02 03`.

### A Tuple Is The Tuple Constructor Head Then Its Parts

A tuple MUST be written as a list whose first child is an atom of the tuple constructor head, kind
`21`, followed by the tree of each part in order.

Example: the tuple of 1 and the text `a` is `(<tuple> 1 "a")`,
`H 03 15 00 01 01 07 01 61 04 00 00 00 01 00 02 01 03 00 01 02 03`, and a value of a named type
declared with an empty list of positional parts is `(<tuple>)`, `H 01 15 02 00 00 01 01 00 01`.

### A Field Pair Is The Field-Pair Head, A Key And A Value

A field pair MUST be written as a list of three children: an atom of the field-pair head, kind `25`,
then the tree of the key, then the tree of the value.

The map and record examples below each hold field pairs.

### A Map Is The Map Constructor Head Then One Field Pair Per Entry

A map MUST be written as a list whose first child is an atom of the map constructor head, kind `23`,
followed by one field pair for each entry, holding the entry's key and value, in the order the map
presents its entries.

Example: the map from the text `a` to 1 is `(<map> (<=> "a" 1))`:

```text
H 04 17 19 07 01 61 00 01 01
  06 00 00 00 01 00 02 00 03 01 03 01 02 03 01 02 00 04 05
```

### A Record Is The Record Constructor Head Then One Field Pair Per Field

A record MUST be written as a list whose first child is an atom of the record constructor head, kind
`22`, followed by one field pair for each of its fields, in the order the record's type declares them.

Example: a value of a record type declared with no fields is `(<record>)`,
`H 01 16 02 00 00 01 01 00 01`, and the record with the field `x` 1 and the field `y` 2 is
`(<record> (<=> "x" 1) (<=> "y" 2))`, in which both field pairs use the one field-pair head leaf:

```text
H 06 16 19 07 01 78 00 01 01 07 01 79 00 01 02
  0A 00 00 00 01 00 02 00 03 01 03 01 02 03 00 01 00 04 00 05 01 03 05 06 07 01 03 00 04 08 09
```

### A Record Field's Key Is A String Leaf Of Its Name

The key of a record field's field pair MUST be written as an atom of a string leaf holding the
field's name as the record's type declares it for writing.

Example: the keys of the record example above are `07 01 78` and `07 01 79`, the string leaves `"x"`
and `"y"`.

### A Wrapper Is The Tree Of The Value It Wraps

A wrapper MUST be written as the tree of its one positional part, with nothing added.

Example: a wrapper of 5 is `H 01 00 01 05 01 00 00 00`, the same bytes as 5.

### An Enumeration Variant Is Headed By A Name Leaf Of Its Name

The first child of the list an enumeration variant is written as MUST be an atom of a name leaf, kind
`10`, holding the variant's name as its type declares it for writing.

The name leaf's text is in Unicode normalization form C, as the canonical byte form of
[ast-binary-format.md](./ast-binary-format.md) has it.

### A Variant Without A Payload Is A List Of Its Name

A variant without a payload MUST be written as a list of one child, the atom of its name.

A variant declared with an empty positional payload is written the same way.

Example: the variant `Empty` is `(Empty)`, `H 01 0A 05 45 6D 70 74 79 02 00 00 01 01 00 01`.

### A Variant With One Payload Value Is Its Name Then That Value

A variant whose payload is one value MUST be written as a list of two children: the atom of its name,
then the tree of the payload.

Example: the variant `Circle` holding 5 is `(Circle 5)`,
`H 02 0A 06 43 69 72 63 6C 65 00 01 05 03 00 00 00 01 01 02 00 01 02`.

### A Variant With Positional Parts Is Its Name Then Each Part

A variant whose payload is two or more positional parts MUST be written as a list of the atom of its
name followed by the tree of each part in order.

Example: the variant `Pair` holding 1 and 2 is `(Pair 1 2)`,
`H 03 0A 04 50 61 69 72 00 01 01 00 01 02 04 00 00 00 01 00 02 01 03 00 01 02 03`.

### A Variant With Named Fields Is Its Name Then One Record

A variant whose payload is named fields MUST be written as a list of two children: the atom of its
name, then the tree of a record holding those fields.

Example: the variant `Rect` with the field `w` 1 and the field `h` 2 is
`(Rect (<record> (<=> "w" 1) (<=> "h" 2)))`:

```text
H 07 0A 04 52 65 63 74 16 19 07 01 77 00 01 01 07 01 68 00 01 02
  0C 00 00 00 01 00 02 00 03 00 04 01 03 02 03 04
  00 02 00 05 00 06 01 03 06 07 08 01 03 01 05 09 01 02 00 0A 0B
```

### The Leaf Kinds The Typed Mapping Writes

An encoder MUST write a typed value with leaves of kinds `0`, `3`, `6`, `7`, `8`, `9`, `10`, `11`,
`13`, `17`, `18`, `19`, `20`, `21`, `22`, `23` and `25` alone.

So a decoder that defines the kinds `0` to `26` decodes the tree of every typed value, and the tree
of a typed value never holds a kind above `26`.

## Reading Typed Values

### A Decoder Reads Each Typed Value From The Tree Its Row Writes

A decoder MUST read each typed value from the tree that the row above for its kind writes.

### A Decoder Refuses An Integer Its Target Can't Hold

A decoder MUST refuse an integer leaf whose value the target integer type can't hold.

Example: the bytes of 300 above, read into an integer type whose largest value is 255, are refused,
and so is the integer 2^64 + 1, `H 01 00 09 01 00 00 00 00 00 00 00 01 01 00 00 00`, read into any
integer type of 64 bits or fewer.

### A Decoder Rounds A Float Leaf To Its Target's Nearest Value

A decoder MUST read a float leaf into a 64-bit binary floating-point target as the 64-bit value nearest
the leaf's exact decimal, ties to even.

A decoder MUST read a float leaf into a 32-bit binary floating-point target as the 32-bit value nearest
the 64-bit value that the rule for a 64-bit target returns, ties to even.

A decoder MUST read the not-a-number leaf and each infinity leaf into a binary floating-point target as
the value that leaf names.

Example: the bytes of the 32-bit value 0.1 above, the decimal `1 x 10^-1`, read into a 64-bit target
as the 64-bit value 0.1 and into a 32-bit target as the 32-bit value 0.1.

### A Decoder Skips A Record Field Its Type Doesn't Declare

A decoder reading a record, or the record of a variant with named fields, MUST skip each field pair
whose key names a field the target type doesn't declare, whatever tree the pair's value holds.

Example: the record `(<record> (<=> "x" 1) (<=> "extra" (<record> (<=> "items" (<list> 2)))))`, read
into a record type that declares the field `x` alone, is the record with the field `x` 1:

```text
H 08 16 19 07 01 78 00 01 01 07 05 65 78 74 72 61 07 05 69 74 65 6D 73 14 00 01 02
  11 00 00 00 01 00 02 00 03 01 03 01 02 03 00 01 00 04 00 00 00 01 00 05 00 06 00 07
  01 02 0A 0B 01 03 08 09 0C 01 02 07 0D 01 03 05 06 0E 01 03 00 04 0F 10
```

### A Type That Refuses Unknown Fields Refuses Them

A decoder reading a record into a type marked to refuse unknown fields MUST refuse a record that holds
a field pair whose key names a field the type doesn't declare.

Example: the bytes of the record example above, read into a record type that declares the field `x`
alone and is marked to refuse unknown fields, are refused for the field `extra`.

### A Decoder May Accept A Name Head In Place Of A Constructor Head

A decoder MAY accept a list, a tuple, a map or a record whose first child is an atom of a name leaf
spelling `list`, `tuple`, `map` or `record` in place of the constructor head of the kind it spells.

An encoder MUST NOT write a name leaf in place of a constructor head.

Example: `(record (<=> "x" 1))`, with the name `record` as its head, is the record with the field `x`
1 for a decoder that accepts the name head:

```text
H 04 0A 06 72 65 63 6F 72 64 19 07 01 78 00 01 01
  06 00 00 00 01 00 02 00 03 01 03 01 02 03 01 02 00 04 05
```

## Additive Evolution

### A Field Added To A Record Type Is Optional

A field added to a record type whose earlier values decoders already read MUST be optional, so that a
decoder of the newer type reads the earlier values and a decoder of the earlier type skips the field.

Example: `(<record> (<=> "x" 1) (<=> "note" ("hi")))`, written by a record type with the optional
field `note`, read into the earlier record type that declares the field `x` alone, is the record with
the field `x` 1:

```text
H 06 16 19 07 01 78 00 01 01 07 04 6E 6F 74 65 07 02 68 69
  0B 00 00 00 01 00 02 00 03 01 03 01 02 03 00 01 00 04
  00 05 01 01 07 01 03 05 06 08 01 03 00 04 09 0A
```

### Additive Evolution Of This Contract

The addition of a row for a kind of typed value that no row maps MUST NOT change the tree of any typed
value an existing row maps.

A change to the tree that an existing row writes MUST carry a container version increment and a
stated migration path.
