# A Positional Binary Wire Format — Reference Specification (prior art)

> **Reference material, not a Cadenza requirement.** This is a byte-exact
> reverse-engineering of an *external* positional binary serialization framework,
> captured as prior art to inform the value-interchange decision (should Cadenza's
> stable value form adopt or interoperate with a format of this shape?). It
> describes concrete byte layouts and cites the source it was derived from by
> bare filename only; it deliberately lives under `options/.../references/`,
> never under `spec/`. Derived 2026-07-07.

Every nontrivial claim below cites `file:line` against the source it was derived
from, and every byte layout is backed by a snapshot test (the `.snap` files
record the exact bytes produced for a known input value, so they are the ground
truth).

Snapshot byte notation: the `.snap` files render bytes as Rust byte-string
literals. `b"*"` is `0x2A`, `b"\x124Vx"` is `12 34 56 78`, `b"\0"` is `0x00`,
`b"\x7f\0\0\x01"` is `7F 00 00 01`. A snapshot value is a byte-rope of chunks;
the chunk boundaries in the snapshot (each `b"..."` line) are an artifact of how
the buffer was built and are **not** part of the format — only the concatenated
byte sequence matters.

---

## 1. Overview

The format is a compact, **schema-driven, positional binary serialization
framework** for an RPC system (`lib.rs:4-8`). Its design, as evidenced by the
code:

- **Positional, tag-free payloads.** The value bytes carry no field names, no
  field tags, and (for structs/tuples) no length or count of fields. Layout is
  entirely determined by the static type. This is the "positional" mode described
  in the comparison table at `tagged.rs:15-19` ("Per-field overhead: Zero").
- **Big-endian, fixed-width scalars by default.** Integers are written with
  `to_be_bytes` (`ser.rs:220-262`). There is an *optional* LEB128 varint type
  (`VarU64`) for fields that are small and slowly growing (`impl/varint.rs`),
  but no primitive integer uses varint encoding by default.
- **Length-prefixed variable-length data.** Sequences, strings, byte blobs and
  maps are prefixed with an 8-byte big-endian count (`ser.rs:105,124-141,152`).
- **A `Wire` trait carrying a compile-time `Type` tree.** Each type has a
  `const TYPE: Type` describing its structure (`lib.rs:190-233`). From this tree
  a 64-bit **`global_id`** is derived by SHA-256 (`lib.rs:131-153`).
- **An optional "tagged" envelope.** The top-level `serialize`/`deserialize`
  entry points (`lib.rs:31-39`) prepend the 8-byte `global_id` as a schema-hash
  guard; the receiver rejects a mismatch (`de.rs:124-134`). Untagged
  (`serialize`) methods omit it.
- **A separate self-describing TLV format, `TaggedWire`** (note: *different*
  from the tagged envelope), used for bootstrap messages that cannot rely on
  prior version negotiation (`tagged.rs:1-19`).
- **A schema-versioning system** where each `#[wire(tag = V.S)]`-annotated
  version produces a distinct `Type` tree and therefore a distinct `global_id`;
  peers negotiate by exchanging hashes (`wire/structs.rs:15-100`).

Two distinct "tag" concepts appear and must not be conflated:

| Concept | What it is | Where |
|---|---|---|
| **Tagged envelope** | 8-byte `global_id` schema-hash prefix on a positional message | `ser.rs:43-50`, `de.rs:124-134` |
| **`TaggedWire` TLV** | fully self-describing `[tag_v][tag_s][len][value]*` field stream | `tagged.rs`, `tagged_wire/*` |
| **`#[wire(tag=V.S)]`** | a *schema-version* annotation used at compile time; does **not** put a tag byte on the positional wire | `wire/common.rs:79-137` |

---

## 2. Primitive encodings

### 2.1 Varint (`VarU64`) — LEB128

`VarU64` is a `u64` newtype (`impl/varint.rs:24`) that encodes as **unsigned
LEB128** (`impl/varint.rs:1-11`). It is *not* the default encoding for any
integer type — only used where a field is explicitly declared `VarU64`.

Encoding algorithm (`impl/varint.rs:45-67`):

```
loop:
    byte = (v & 0x7F)            # low 7 bits
    v  >>= 7
    if v != 0: byte |= 0x80      # set continuation (high) bit
    emit byte
    if v == 0: break
```

- **7 data bits per byte, low-order group first (little-endian group order).**
- **High bit (`0x80`) = continuation.** Last byte has high bit clear.
- **Unsigned; no zigzag.** There is no signed varint in this format.
- Size: `0..=127` → 1 byte, `..=16383` → 2 bytes, … up to **10 bytes** for the
  full `u64` range (`impl/varint.rs:27-29`, test `impl/varint.rs:158-166`).

Decoding (`impl/varint.rs:69-96`): accumulates `((byte & 0x7F) as u64) << shift`,
`shift += 7` per continuation. **Overlong-encoding rejection**: more than 10
groups, or a 10th byte `> 0x01` (which would overflow `u64`), returns
`InvalidLength` (`impl/varint.rs:80-87`). A truncated value (continuation bit set
then EOF) returns `InvalidLength` because reading the next byte fails
(`impl/varint.rs:75-76`, test `impl/varint.rs:168-175`).

`VarU64` has `Wire` id `'v'` (`0x76`), deliberately distinct from `u64`'s `'4'`,
so swapping `u64`↔`VarU64` is a schema-breaking change (`impl/varint.rs:32-43`).

> No dedicated `VarU64` snapshot exists; the round-trip/size tests
> (`impl/varint.rs:139-166`) prove the layout: e.g. `128` → `0x80 0x01`,
> `16383` → `0xFF 0x7F`.

### 2.2 Scalar integers — fixed-width big-endian

All default integer serialization uses `to_be_bytes` (**big-endian**),
fixed size, no length prefix (`ser.rs:220-262`):

| Type | `Wire` id (`lib.rs`) | Bytes on wire | Ser (`ser.rs`) | De (`de.rs`) |
|---|---|---|---|---|
| `u8`  | `'1'` 0x31 (359-365) | 1 | 232-236 | 354 |
| `u16` | `'2'` 0x32 (367-373) | 2 BE | 237 | 355 |
| `u32` | `'3'` 0x33 (375-381) | 4 BE | 238 | 356 |
| `u64` | `'4'` 0x34 (383-389) | 8 BE | 239 | 357 |
| `u128`| `'5'` 0x35 (391-397) | 16 BE | 240 | 358 |
| `usize`| `'6'` 0x36 (399-405) | **8 BE (as u64)** | 242-246 | 360-368 |
| `i8`  | `'a'` 0x61 (408-414) | 1 | 248-252 | 370 |
| `i16` | `'b'` 0x62 (416-422) | 2 BE | 253 | 371 |
| `i32` | `'c'` 0x63 (424-430) | 4 BE | 254 | 372 |
| `i64` | `'d'` 0x64 (432-438) | 8 BE | 255 | 373 |
| `i128`| `'e'` 0x65 (440-446) | 16 BE | 256 | 374 |
| `isize`| `'f'` 0x66 (448-454) | **8 BE (as i64)** | 258-262 | 376-384 |

Signed integers are **two's-complement big-endian** (Rust's `to_be_bytes` /
`from_be_bytes`), *not* zigzag. `i8` is written as the raw byte
(`ser.rs:248-252`) and read via `buf[0] as _` (`de.rs:370`).

`usize`/`isize` are normalized to a fixed 8-byte wire width by casting to
`u64`/`i64` before writing (`ser.rs:242-246,258-262`) and range-checked back on
read (returns `InvalidType` if a decoded `u64` exceeds the host `usize`,
`de.rs:363-367`). **This makes `usize` wire-portable across 32/64-bit hosts**,
but see the gaps section for the length-prefix `usize` subtlety.

Deserialization of every fixed-width integer first checks that at least `$size`
bytes remain, else `InvalidLength` (`de.rs:341-344`).

**Worked examples (ground truth):**

- `u8 = 42` → snapshot `u8_42.snap` = `b"*"` = **`2A`**. (42 = 0x2A.)
- `u16 = 12345` → `u16_12345.snap` = `b"09"` = **`30 39`**. (0x3039 = 12345, BE.)
- `u32 = 305419896` → `u32_305419896.snap` = `b"\x124Vx"` = **`12 34 56 78`**.
  (0x12345678 = 305419896, BE — proves big-endian byte order unambiguously.)
- `u64 = 72623859790382856` → `u64_72623859790382856.snap` =
  `b"\x01\x02\x03\x04\x05\x06\x07\x08"` = **`01 02 03 04 05 06 07 08`**.
  (0x0102030405060708, BE.)

### 2.3 `bool`

One byte: `self as u8` (`ser.rs:213-217`), i.e. `false`→`00`, `true`→`01`.
Decode: `0`→`false`, **any nonzero**→`true` (`de.rs:327-332`). `Wire` id `'B'`
(0x42) (`lib.rs:350-356`).

### 2.4 Floats

**There is no `Serialize`/`Deserialize`/`Wire` impl for `f32` or `f64` in this
format's core crate.** Float types can *appear inside* derived structs/enums (the
derive tests reference `f64` at `wire.rs:301,305`), but only because the deriving
crate would need a `Wire` impl in scope; the core does not provide one. See gaps.

### 2.5 The unit type `()`

Serializes to **zero bytes** (`ser.rs:279-281`); deserializes consuming nothing
(`de.rs:400-403`). `Wire` id `'0'` (0x30), name `"nil"` (`lib.rs:486-492`).

- Worked example: `()` → `unit.snap` = `[]` (empty).

---

## 3. Composite encodings

Unless noted, composites write their parts back-to-back with no separators.

### 3.1 Length prefix convention

Variable-length containers (`Vec<T>`, `VecDeque<T>`, `BTreeMap`, `String`,
`Bytes`, `ByteVec`, `Arc<str>`) prefix their element/byte **count** by calling
`self.len().serialize(out)` where `len()` is a `usize` — therefore an **8-byte
big-endian** length (see §2.2 `usize`) (`ser.rs:105,111,124-159,168-173`).

On decode the length is read as `usize` (`u64` → `usize`, `de.rs:360-368`).

> Note the count is a *usize serialized as u64*: always 8 bytes on the wire.

### 3.2 `String` (and `Arc<str>`)

`Wire` id `'u'` (0x75), name `"String"` (`lib.rs:334-340`). Layout =
**[u64 BE byte-length][UTF-8 bytes]**. `String::serialize` delegates to
`self.into_bytes().serialize` — i.e. `Vec<u8>` (`ser.rs:162-166`). Decode reads a
`Vec<u8>` then validates UTF-8, returning `InvalidUtf8` on failure
(`de.rs:263-268`). `Arc<str>` uses the identical layout (`ser.rs:168-173`,
`de.rs:270-275`, `Wire` at `lib.rs:342-348`).

- Worked example: `"hello"` → `string_hello.snap` =
  `b"\0\0\0\0\0\0\0\x05"` + `b"hello"` = **`00 00 00 00 00 00 00 05  68 65 6C 6C 6F`**.
  (length 5 as u64 BE, then ASCII "hello".)

### 3.3 Byte blobs — `Vec<u8>`, `Bytes`, `ByteVec`, `Tagged<O>`

All share the same layout: **[u64 BE byte-length][raw bytes]**.

- `Vec<T>` writes `len` then, **specialized when `T == u8`**, copies the raw
  bytes in one shot (`ser.rs:124-141`); the `Bytes`/`ByteVec` impls do the same
  (`ser.rs:103-115`). `Bytes` and `ByteVec` both have `TYPE = <Vec<u8>>::TYPE`
  (`lib.rs:322-332`), so on the wire they are indistinguishable from `Vec<u8>`.
- `Debug`-rendering the `Type` tree shows `[u8]` as the special name `"Bytes"`
  (`lib.rs:92-97`), but this affects only debug output, not the bytes.

Worked examples (all identical layout):

- `vec![1u8,2,3]` → `vec_u8_1_2_3.snap` =
  `00 00 00 00 00 00 00 03  01 02 03`.
- `Bytes::from(vec![1,2,3])` → `bytes_1_2_3.snap` = **same** `00…03 01 02 03`.
- `ByteVec::from(vec![1,2,3])` → `bytevec_1_2_3.snap` = **same**.
- `[u8; 4]` fixed array `[1,2,3,4]` → `u8_array_4.snap` = `01 02 03 04`
  (**no length prefix** — see §3.9).

### 3.4 `Vec<T>` / `VecDeque<T>` of non-byte `T`

Layout = **[u64 BE element-count][elem₀][elem₁]…]**, each element serialized by
its own `Serialize` (`ser.rs:135-139,143-150`). Decode reads count, then that
many elements (`de.rs:208-237,239-248`). `Wire` id `'['` (0x5B), name `"list"`,
one child = `T::TYPE` (`lib.rs:288-308`).

- Worked example: `vec!["hello","world"]` → `vec_string_hello_world.snap` =
  `00 00 00 00 00 00 00 02` (count 2)
  `00 00 00 00 00 00 00 05` `hello`
  `00 00 00 00 00 00 00 05` `world`.

**Vec length sanity guard (decode):** before allocating, `Vec<T>::deserialize`
rejects `len * size_of::<T>()` if it overflows or exceeds `u32::MAX`
(`0xFFFF_FFFF`) bytes → `InvalidLength` (`de.rs:211-216`). For `T=u8` it also
requires the buffer to actually hold `len` bytes (`de.rs:221-223`).

### 3.5 `Option<T>`

**Enum with a 1-byte discriminant, then payload for `Some`.**
`None`→`00`; `Some(v)`→`01` followed by `v` (`ser.rs:59-73`). Decode: tag `0`→
`None`, `1`→`Some(T)`, **any other byte → `InvalidVariant`** (`de.rs:157-168`).
`Wire`: `id = Type::ENUM = 'E'` (0x45); children are `None{id:0}` and
`Some{id:1, child = T::TYPE}` (`lib.rs:259-279`).

- `None::<u8>` → `option_none_u8.snap` = **`00`**.
- `Some(42u8)` → `option_some_u8_42.snap` = `b"\x01*"` = **`01 2A`**.

### 3.6 `Result<T, E>`

Same shape as `Option`: 1-byte discriminant then payload.
**`Ok(v)` → `00` + `v`; `Err(e)` → `01` + `e`** (`ser.rs:180-193`). Decode: `0`→
`Ok`, `1`→`Err`, other → `InvalidVariant` (`de.rs:282-298`). `Wire`:
`id = ENUM`, children `Ok{id:0,child=T}`, `Err{id:1,child=E}` (`lib.rs:456-473`).

- `Ok::<_,u8>(42u8)` → `result_ok_u8_42.snap` = `b"\0*"` = **`00 2A`**.
- `Err::<u8,_>(42u8)` → `result_err_u8_42.snap` = `b"\x01*"` = **`01 2A`**.

> Note the discriminant convention: **`Option::None` = 0, `Option::Some` = 1**;
> **`Result::Ok` = 0, `Result::Err` = 1**. (`Ok` is the *zero* value.)

### 3.7 Tuples (and derived structs)

Tuples up to 16 elements serialize each element in order **with no count and no
length prefix** (`ser.rs:276-301`); decode reads each element in order
(`de.rs:397-424`). `Wire`: `id = Type::STRUCT = 's'` (0x73), name `"tuple"`,
children = each element type (`lib.rs:483-511`). Derived structs behave
identically: fields written in declaration order, no framing
(`wire/structs.rs:173-185`).

- `(42u32, "hello")` → `tuple_u32_string.snap` =
  `00 00 00 2A` (u32 42, BE)
  `00 00 00 00 00 00 00 05` `hello` (String).
  = `b"\0\0\0*\0\0\0\0\0\0\0\x05"` + `b"hello"`. Note the tuple itself adds **no**
  bytes; it is exactly `u32` layout followed by `String` layout.

### 3.8 `Box<T>`

**Fully transparent** — encodes exactly as `T`, adding nothing
(`ser.rs:89-101`, `de.rs:137-149`); `Box<T>::TYPE = T::TYPE` (`lib.rs:281-286`).

- `Box::new(42u32)` → `box_u32_42.snap` = `b"\0\0\0*"` = **`00 00 00 2A`**
  (identical to a bare `u32=42`).
- `Box::new("hello")` → `box_string_hello.snap` = identical to bare `String`.

### 3.9 Fixed-size arrays `[T; N]`

`Wire`: `id = STRUCT`, name `"array"`, `N` children of `T::TYPE`
(`lib.rs:475-481`). The only `Serialize`/`Deserialize` impls provided are for
**`[u8; N]`**: raw `N` bytes, **no length prefix** (`ser.rs:264-268`,
`de.rs:386-395`; requires ≥ N bytes else `InvalidLength`).

- `[1u8,2,3,4]` → `u8_array_4.snap` = **`01 02 03 04`** (contrast with `Vec<u8>`,
  which prefixes an 8-byte length).

### 3.10 `BTreeMap<K, V>`

Layout = **[u64 BE entry-count][k₀][v₀][k₁][v₁]…]** (`ser.rs:152-159`). Because
`BTreeMap` iterates in **sorted key order**, entries are emitted **sorted by key**
(ascending, by `K`'s `Ord`). Decode reads count, then that many `(K,V)` pairs,
inserting into a fresh `BTreeMap` (`de.rs:250-261`). `Wire`: `id = 'm'` (0x6D),
name `"map"`, two children `[K::TYPE, V::TYPE]` (`lib.rs:310-320`).

- Worked example: `{1u32:"one", 2u32:"two"}` → `btreemap_u32_string.snap`:
  `00 00 00 00 00 00 00 02` (count 2)
  `00 00 00 01` `00 00 00 00 00 00 00 03` `one` (key 1 as u32, then "one")
  `00 00 00 02` `00 00 00 00 00 00 00 03` `two`.
  Keys appear in ascending order 1 then 2, confirming sorted emission.

- Nested `BTreeMap<u32, BTreeMap<String, Vec<u8>>>` → `btreemap_nested.snap`
  `{1:{"a":[1,2,3]}, 2:{"b":[4,5,6]}}`:
  `00 00 00 00 00 00 00 02` (outer count 2)
  `00 00 00 01` (key 1) `00 00 00 00 00 00 00 01` (inner count 1)
    `00 00 00 00 00 00 00 01` `a` (inner key "a")
    `00 00 00 00 00 00 00 03` `01 02 03` (Vec<u8> [1,2,3])
  `00 00 00 02` (key 2) `00 00 00 00 00 00 00 01` (inner count 1)
    `00 00 00 00 00 00 00 01` `b`
    `00 00 00 00 00 00 00 03` `04 05 06`.
  This confirms recursive nesting with no extra framing at any level.

### 3.11 Net types (`impl/net.rs`)

- **`Ipv4Addr`**: 4 raw octets (`[u8;4]`, no prefix) (`impl/net.rs:48-52`).
  `Wire` id `STRUCT`, name `"std::net::IpV4Addr"`, child `[u8;4]`
  (`impl/net.rs:54-60`).
  - `Ipv4Addr::new(127,0,0,1)` → `ipv4_localhost.snap` = `b"\x7f\0\0\x01"` =
    **`7F 00 00 01`**.
- **`Ipv6Addr`**: 16 raw octets (`[u8;16]`) (`impl/net.rs:69-73`).
  - `Ipv6Addr::…::1` → `ipv6_localhost.snap` =
    `00…00 01` (16 bytes, last = `01`).
- **`IpAddr`**: **1-byte discriminant `4` (V4) or `6` (V6)** then the address
  (`impl/net.rs:18-31`). Decode: tag `4`/`6`, other → `InvalidVariant`
  (`impl/net.rs:4-16`). `Wire` id `ENUM` (`impl/net.rs:33-39`). *(The
  discriminants are the literal integers 4 and 6, not 0/1.)*
  - `IpAddr::V4(127.0.0.1)` → `ipaddr_v4.snap` = `b"\x04\x7f\0\0\x01"` =
    **`04  7F 00 00 01`**.
  - `IpAddr::V6(::1)` → `ipaddr_v6.snap` = `06` + 16 bytes (`…01`).
- **`SocketAddr`**: `IpAddr` (tagged as above) then **`u16` port (2 bytes BE)**
  (`impl/net.rs:91-96`); decode reads `IpAddr` then `u16` (`impl/net.rs:83-89`).
  `Wire` id `STRUCT`, children `[IpAddr, u16]` (`impl/net.rs:98-104`).
  - `SocketAddr V4 127.0.0.1:8080` → `socketaddr_v4.snap` =
    `b"\x04\x7f\0\0\x01\x1f\x90"` = **`04  7F 00 00 01  1F 90`**.
    (`0x1F90` = 8080, BE.)

### 3.12 Time (`impl/time.rs`)

`Duration` = **`u64` seconds (8 BE)** then **`u32` subsec-nanos (4 BE)**
(`impl/time.rs:20-25`); decode reads `u64` then `u32` and calls
`Duration::new` (`impl/time.rs:12-18`). `Wire` id `STRUCT`, children
`[u64, u32]`, name `"std::time::Duration"` (`impl/time.rs:4-10`). No `SystemTime`
or `Instant` impl exists.

### 3.13 Ranges / Bound (`impl/ops.rs`)

- `Range<T>` / `RangeInclusive<T>`: `start` then `end`, back-to-back, no framing
  (`impl/ops.rs:12-17,35-41`). `Wire` id `STRUCT`, children `[T, T]`.
- `Bound<T>`: **1-byte discriminant** — `Unbounded`→`0`, `Excluded(v)`→`1`+`v`,
  `Included(v)`→`2`+`v` (`impl/ops.rs:68-82`); decode maps `0/1/2`, other →
  `InvalidVariant` (`impl/ops.rs:51-66`). `Wire` id `ENUM`, three variants with
  ids 0/1/2 (`impl/ops.rs:84-106`).

### 3.14 `std::io::Error` / `io::ErrorKind` (`impl/io.rs`)

- `io::ErrorKind`: **1-byte discriminant** mapping to a fixed table of 37 kinds
  `0..=36`; **any unrecognized kind serializes as `255`** and any unknown byte
  deserializes to `io::ErrorKind::Other` (`impl/io.rs:73-117` ser,
  `impl/io.rs:12-56` de). `Wire` id `ENUM`, one child `u8`
  (`impl/io.rs:58-64`).
- `io::Error`: `ErrorKind` (1 byte) then the **message as a `String`**
  (`[u64 len][utf8]`) via `self.to_string()` (`impl/io.rs:66-71`). Decode reads
  kind then a `String` (`impl/io.rs:4-10`). `Wire` id `STRUCT`, children
  `[ErrorKind, String]` (`impl/io.rs:119-125`).

### 3.15 `de::Error` / `de::ErrorKind`

`de::ErrorKind` is a derived `Wire` enum (`de.rs:69-82`), so it serializes as a
**1-byte discriminant = declaration index** (`wire/enums.rs:92-129`):

| Kind | Byte | Snapshot |
|---|---|---|
| `InvalidType` | `00` | `error_invalid_type.snap` = `b"\0"` |
| `InvalidLength` | `01` | `error_invalid_length.snap` = `b"\x01"` |
| `InvalidUtf8` | `02` | `error_invalid_utf8.snap` = `b"\x02"` |
| `InvalidBytes` | `03` | `error_invalid_bytes.snap` = `b"\x03"` |
| `InvalidVariant` | `04` | `error_invalid_variant.snap` = `b"\x04"` |
| `InvalidList` | `05` | `error_invalid_list.snap` = `b"\x05"` |

`de::Error` (`Serialize`) writes only its `kind` — the `ty` pointer is not on the
wire (`ser.rs:270-274`); decode reconstructs `kind` and a placeholder `ty`
(`de.rs:49-59`). (`InvalidBytes` and `InvalidList` are defined in the enum but no
decode path currently *raises* them; see §5.)

### 3.16 `Infallible`

`Wire` id `'!'` (0x21), name `"never"` (`lib.rs:251-257`). Serialize is
`unreachable!` (cannot construct) (`ser.rs:53-57`); deserialize always returns
`InvalidType` (`de.rs:151-155`).

---

## 4. The tagged envelope (schema-hash guard)

### 4.1 Two entry points: tagged vs untagged

The format exposes two layers per direction:

- **Untagged**: `Serialize::serialize` / `Deserialize::deserialize`
  — emit/consume only the positional value bytes described in §§2–3.
- **Tagged**: `Serialize::serialize_tagged` / `Deserialize::deserialize_tagged`
  (`ser.rs:43-51`, `de.rs:124-135`), used by the top-level free functions
  `serialize()` / `deserialize()` (`lib.rs:31-39`).

**On-wire difference**: a tagged message is exactly an **8-byte prefix followed
by the untagged bytes**:

```
tagged   = [ global_id : u64 big-endian ] [ untagged value bytes … ]
untagged = [ untagged value bytes … ]
```

The prefix is produced by `const { Self::TYPE.global_id() }.serialize(out)` then
`self.serialize(out)` (`ser.rs:43-50`). Since `global_id()` is a `u64`, it is
written as **8 bytes big-endian** (§2.2). The optionality mechanism is purely
**which method you call** — there is no leading flag byte and no wrapper type;
`serialize`/`deserialize` produce untagged bytes, `serialize_tagged`/
`deserialize_tagged` produce/expect the prefixed form. (All the §§2–3 snapshots
were produced by the untagged `value.serialize(&mut builder)` path —
`tests.rs:69-77` — so they contain **no** hash prefix.)

### 4.2 Receiver behavior — verify-and-reject

`deserialize_tagged` reads a `u64` and compares it to the *local*
`Self::TYPE.global_id()`; **mismatch → `InvalidType` error, no dispatch**
(`de.rs:124-134`):

```rust
let id = u64::deserialize(bytes)?;
if id != const { Self::TYPE.global_id() } {
    return Err(Error::new(ErrorKind::InvalidType, &Self::TYPE));
}
Self::deserialize(bytes)
```

So the tagged prefix is a **schema/version guard**, not a dispatch key at this
layer. (Higher layers — the service codegen — build a
`global_id → (method, version)` map to dispatch; that is out of the core crate,
but described at `wire/structs.rs:15-100`.)

### 4.3 How `global_id` is computed

`global_id` is derived from the type's `Type` tree by **SHA-256, truncated to the
first 8 bytes, read big-endian into a `u64`** (`lib.rs:131-153`). Two steps:

**(a) `serialize_for_hash` — canonicalize the `Type` tree to bytes**
(`lib.rs:155-171`). Recursively, for each node:

```
buffer[cursor++] = self.id          // 1 byte: the type's id
buffer[cursor++] = 1                // literal 0x01 "open" marker
for child in children: recurse
buffer[cursor++] = 0                // literal 0x00 "close" marker
```

Node `id` bytes are the ASCII/`u8` ids from the `Wire` impls (e.g. `u8`=`'1'`,
`String`=`'u'`, `Vec`=`'['`, map=`'m'`, struct/tuple=`'s'`, enum=`'E'`; enum
variant ids are their small integer discriminants such as `0`/`1`). The scheme is
**`id, 0x01, <children…>, 0x00`** per node — the `0x01`/`0x00` frame each node so
distinct tree shapes hash differently.

**(b) `global_id` — hash the name + canonical bytes** (`lib.rs:131-153`):

```
buffer[0] = 0                        // sentinel: guarantees a 0 between name and subtree
cursor = 1
root.serialize_for_hash(buffer, &mut cursor)   // fills buffer[1..cursor]
h = SHA256( name.as_bytes() ++ buffer[0..cursor] )   // note: includes sentinel byte 0
global_id = u64::from_be_bytes(h[0..8])
```

- **Hash function**: SHA-256, via a `const fn` SHA-256 implementation so the
  whole thing evaluates at compile time (`sha2-const/lib.rs:71-87`,
  `lib.rs:148-151`).
- **Hashed input**: the type's `name` string bytes, then a `0x00` sentinel, then
  the `serialize_for_hash` canonical bytes of the tree (starting at buffer[1]).
  Because the root's own `serialize_for_hash` starts at `buffer[1]`, the layout
  is `name ++ [0x00] ++ [root.id, 0x01, …children…, 0x00]`.
- **Truncation**: first **8 bytes** of the 32-byte digest (`lib.rs:152`).
- **Byte order on the wire**: `from_be_bytes` builds the `u64` from `h[0..8]`
  **big-endian** (`lib.rs:152`), and that `u64` is then serialized big-endian
  (§2.2). Net effect: **the 8 prefix bytes on the wire are `SHA256(...)[0..8]` in
  digest order** (h[0] first).

### 4.4 Worked example — tagged vs untagged `u8 = 42`

`u8`: `name = "u8"`, `id = '1' = 0x31`, no children (`lib.rs:359-365`).

`serialize_for_hash` into `buffer` with sentinel:
`buffer = [00, 31, 01, 00]`, `cursor = 4`.
Hash input = `"u8"` (`75 38`) ++ `buffer[0..4]` (`00 31 01 00`) =
`75 38 00 31 01 00`.

Computed:
```
SHA256("u8" ++ 00 31 01 00) = d60b8225485a389b155546aa67d361fb…
global_id (first 8 bytes, BE u64) = d60b8225485a389b
```

Therefore:

- **untagged** `42u8` = `2A`  (this is exactly `u8_42.snap`).
- **tagged**  `42u8` = `D6 0B 82 25 48 5A 38 9B  2A`
  (8-byte `global_id` prefix, then the value byte).

(For cross-checking other types with the same algorithm: `bool` → prefix
`FF FE 1F D6 DD 33 AC F7`; `u32` → prefix `03 16 AF 8F B5 E6 56 0F`.)

> There is **no snapshot** of a tagged message (all snapshots use the untagged
> path), so the exact 8 prefix bytes above are computed from the documented
> algorithm rather than pinned by a `.snap`. The algorithm itself is fully
> determined by the cited code.

### 4.5 Schema versioning and `global_id`

`#[wire(tag = V.S)]` annotations make a type *versioned*: each retained version
produces a distinct `Type` tree (`VERSION_TYPES`, `lib.rs:182-198`) and thus a
distinct `global_id` (tests `tests.rs:335-343,576-582`). `Wire::TYPE` equals the
**latest** version's tree (`wire/structs.rs:883`). The versioned wire *payload*
is still positional — versioning changes *which* fields are present, not the
framing — and the version is conveyed out-of-band via the negotiated `global_id`,
not by a byte in the payload. It is largely orthogonal to the byte-level format
and is not needed to encode/decode a single fixed-schema value.

---

## 5. Error / decode-failure model

`de::Error` = `{ kind: ErrorKind, ty: &'static Type }` (`de.rs:15-33`); only
`kind` is ever serialized (§3.15). `ErrorKind` variants and their triggers:

| `ErrorKind` | Byte | Raised when (cite) |
|---|---|---|
| `InvalidType` (0) | 00 | tagged-hash mismatch (`de.rs:130-132`); `usize`/`isize` value out of host range (`de.rs:365,381`); `Infallible::deserialize` (`de.rs:153`) |
| `InvalidLength` (1) | 01 | fixed-int/array/`ByteVec` truncation — fewer than N bytes remain (`de.rs:192-194,342-344,388-390`); `Vec` length overflow / `len*size_of>u32::MAX` / insufficient bytes for `Vec<u8>` (`de.rs:211-223`); `VarU64` truncated or overlong (`impl/varint.rs:80-87`); `deserialize_with_len_prefix` trailing bytes remain (`de.rs:118-120`) |
| `InvalidUtf8` (2) | 02 | `String`/`Arc<str>` bytes not valid UTF-8 (`de.rs:266`) |
| `InvalidBytes` (3) | 03 | **defined but not raised** by any decode path (only exists as a variant / for external use) |
| `InvalidVariant` (4) | 04 | `Option` tag ∉ {0,1} (`de.rs:166`); `Result` tag ∉ {0,1} (`de.rs:296`); `IpAddr` tag ∉ {4,6} (`impl/net.rs:11`); `Bound` tag ∉ {0,1,2} (`impl/ops.rs:59`); derived enum unknown discriminant (`wire/enums.rs:187-189,531-535`) |
| `InvalidList` (5) | 05 | **defined but not raised** by any decode path |

Notable semantics:

- **No trailing-bytes check by default.** `deserialize` / `deserialize_tagged`
  do not require the buffer to be fully consumed; leftover bytes are ignored
  unless the caller uses `deserialize_with_len_prefix`, which *does* enforce
  emptiness (`de.rs:115-122`).
- Decode reads are **length-checked before copy** for fixed-width types
  (`de.rs:341-348`), so truncation is a clean `InvalidLength`, never a panic.
- `Vec<T>::deserialize` (non-`u8` `T`) does not pre-validate the buffer against
  the element count beyond the `u32::MAX` byte-budget guard, so an oversized
  count is caught by the inner element decode failing (`de.rs:230-234`).

---

## 6. Trait / impl surface

### 6.1 The three traits

- **`Wire`** (`lib.rs:190-233`): carries `const TYPE: Type` (structure
  descriptor) plus versioning consts (`VERSION_TYPES`, `VERSION_XPRODUCT_COUNT`,
  `__XPRODUCT_HASH_ENTRIES`). `Type` = `{ name: &'static str, id: u8, children:
  &'static [Type] }` (`lib.rs:45-50`). `id`s are single bytes:
  `STRUCT='s'`, `ENUM='E'`, `RECURSIVE='@'`, scalars `'0'..'6'`/`'a'..'f'`,
  `bool='B'`, `String='u'`, `Vec='['`, map='m', `VarU64='v'`, never='!'
  (`lib.rs:122-129`, scalar impls throughout `lib.rs`).
- **`Serialize`** (`ser.rs:13-51`): `fn serialize(self, &mut Builder)` consumes
  the value and appends bytes; plus `serialize_at(_, version_idx)` (default
  ignores the index; versioned/container types override to route by version) and
  `serialize_tagged` (§4). Serialization **takes `self` by value** and writes
  into a byte-rope builder.
- **`Deserialize: Sized + Wire`** (`de.rs:99-135`): `fn deserialize(&mut ByteVec)
  -> Result<Self, Error>`, plus `deserialize_at`, `deserialize_with_len_prefix`,
  `deserialize_tagged`. Reading advances/consumes the byte-rope in place.

### 6.2 The buffer types

- The writing target is a byte-rope `Builder`; the reading source is a byte-rope
  `ByteVec` (`bytevec/lib.rs:17-19`). `ByteVec` is a rope of `Bytes` chunks;
  `serialize` appends chunks and `Builder::finish()` concatenates the head into
  the chunk list (`byte_vec/builder.rs:245-251`).
- `Builder::write_with_len_prefix(f)` runs `f`, measures the bytes it wrote, and
  **inserts an 8-byte big-endian `u64` length in front of them**
  (`byte_vec/builder.rs:253-285`; the `debug_assert_eq!(len_chunk.len(), 8)` at
  line 283 pins the width at 8). This is the primitive behind
  `serialize_with_len_prefix` (`ser.rs:29-41`) and the TaggedWire per-field/whole
  framing. **Important**: this is a `u64` length, contradicting the prose in
  `tagged.rs:9` which says `[length: u32]`; see §7.

### 6.3 Deriving `Wire` for custom types

`#[derive(Wire)]` (`wire.rs:25-62`) generates `Serialize`, `Deserialize`, and
`Wire` for structs/enums:

- **Non-versioned struct**: `TYPE.id = STRUCT`, `children` = each non-`skip`
  field's `<FieldTy>::TYPE` (`wire/structs.rs:205-209`, `common.rs:197-210`).
  Serialize writes fields in declaration order (`wire/structs.rs:173-185`);
  `#[wire(skip)]` fields are omitted on the wire and filled with
  `Default::default()` on decode (`wire/structs.rs:216-223`).
- **Non-versioned enum**: `TYPE.id = ENUM`; each variant becomes a child `Type`
  whose `id` is its **declaration index (u8)** and whose children are the
  variant's field types (`wire/enums.rs:164-180`). Serialize writes the **1-byte
  variant index** then the fields (`wire/enums.rs:92-129`); decode reads the u8
  and matches, unknown → `InvalidVariant` (`wire/enums.rs:184-190`). Max 255
  variants (`wire/enums.rs:49,176-178`).
- `#[wire(schema = OtherTy)]` overrides the child type used for hashing while
  keeping the derived (de)serialization — used for newtype wrappers over e.g.
  `Vec<u8>` (`common.rs:292-296`, `wire.rs:147-155`).
- `#[wire(recurse)]` uses `Type::RECURSIVE` (`id='@'`) as the child to break
  infinite type trees for recursive types (`common.rs:205-209`, `lib.rs:125-129`).
- `#[wire(tag = V.S [, default=/fallback= expr])]` opts a type into schema
  versioning (§4.5); `#[wire(min_version = N)]` prunes retained versions
  (`common.rs:47-52,118-137`).

### 6.4 `TaggedWire` — the self-describing TLV format (distinct from §4)

`#[derive(TaggedWire)]` (`tagged_wire/*`) generates a *different* encoding used
for bootstrap messages (`tagged.rs:1-19`). It is **not** the tagged envelope of
§4. Layout: each field is emitted as

```
[ tag_v : u8 ] [ tag_s : u8 ] [ length : u64 big-endian ] [ value bytes … ]
```

where `tag_v.tag_s` come from `#[wire(tag = V.S)]`, `length` is produced by
`write_with_len_prefix` (**8-byte u64**, `tagged_wire/structs.rs:63-65`), and the
value is the field's ordinary positional `Serialize`. Fields are concatenated
(`tagged_wire/structs.rs:54-68`). A whole `TaggedWire` value, when used through
the *plain* `Serialize` impl, is itself wrapped once more in an 8-byte length
prefix (`tagged_wire/structs.rs:155-161`).

Decode (`tagged_wire/structs.rs:119-145`): loop while bytes remain — read
`tag_v`(u8), `tag_s`(u8), `len`(**u64**), then either decode the matching field
from a `len`-byte sub-buffer or **skip `len` bytes for unknown tags** (forward
compatibility). Missing fields get `#[wire(default = expr)]` or
`Default::default()` (`tagged_wire/structs.rs:100-110`). Enums encode one
`(tag_v, tag_s, len, payload)` for the active variant; an unknown tag falls back
to a `#[wire(tag = …, fallback)]` variant or errors `InvalidVariant`
(`tagged_wire/enums.rs:171-236`).

> **Discrepancy to flag**: the module doc `tagged.rs:9,42-44` and the helper
> `read_tag_header` (`tagged.rs:90-102`) describe/read the length as **`u32`**
> (6-byte-per-field overhead), but the actual derived code
> (`tagged_wire/structs.rs:63,126`, `tagged_wire/enums.rs:71,258`) writes and
> reads a **`u64`** (10-byte-per-field overhead) because it uses
> `write_with_len_prefix`, which is hard-coded to 8 bytes. The free helpers in
> `tagged.rs:42-102` appear to be an earlier/unused API (`write_field` is a
> no-op stub). The **derive-macro output is authoritative**: **u64 length**.

---

## 7. Ambiguities / gaps / host-dependent notes

1. **TaggedWire length width — doc vs code.** `tagged.rs` prose and its
   `read_tag_header` say the field length is a `u32` (`tagged.rs:9,99`), but the
   `#[derive(TaggedWire)]` codegen writes/reads a **`u64`**
   (`tagged_wire/structs.rs:63,126`; `tagged_wire/enums.rs:71,258`) because it
   uses `write_with_len_prefix`, hard-coded to 8 bytes
   (`byte_vec/builder.rs:274-283`). A port must use **u64** to match real
   traffic.

2. **`usize`/`isize` wire width.** Serialized as `u64`/`i64` (8 bytes, §2.2), so
   the *format* is host-independent, but decode **fails with `InvalidType`** if a
   value exceeds the host's `usize`/`isize` range (`de.rs:363-367,377-383`).
   Since all length prefixes go through `usize`, a length ≥ 2⁶³ (isize) or a
   32-bit host receiving a length ≥ 2³² would fail to decode. A non-Rust port
   should treat all length prefixes as **unsigned 64-bit big-endian counts**.

3. **No float support in the core.** No `f32`/`f64` `Serialize`/`Deserialize`/
   `Wire` impls exist. Derived types containing floats rely on an impl from
   elsewhere; its byte layout (IEEE-754? endianness? `Wire` id?) **cannot be
   determined from this source**. A port cannot assume a float encoding here.
   **This is the single biggest open question for Cadenza interop** — Cadenza's
   determinism law mandates a specific canonical float/NaN/−0.0 form.

4. **`SystemTime`/`Instant` absent.** Only `Duration` (`u64` secs + `u32` nanos,
   §3.12) is implemented. A foreign encoder must keep nanos `< 1_000_000_000` to
   round-trip identically (`Duration::new` normalizes).

5. **Fixed arrays only implemented for `[u8; N]`.** `[T; N]` has a `Wire` `TYPE`
   for any `T` (`lib.rs:475-481`) but `Serialize`/`Deserialize` exist **only for
   `[u8; N]`** (`ser.rs:264-268`, `de.rs:386-395`).

6. **The tagged-envelope prefix bytes are computed, not snapshot-pinned.** No
   `.snap` exercises `serialize_tagged`, so the exact 8 prefix bytes in §4.4 come
   from executing the documented SHA-256 algorithm (`lib.rs:131-171`), not from a
   recorded golden file. A port should reproduce §4.3 and diff against a live
   `serialize(value)` output.

7. **`Type.id` value space is informal.** The `id: u8` values are ASCII letters
   /digits for built-ins but small integers (`0,1,2,4,6,…`) for enum
   discriminants and variant ids. Safe only because `serialize_for_hash` frames
   each node with `0x01`/`0x00` and the id is position-dependent. A port that
   recomputes `global_id` must replicate the **exact id bytes** from each `Wire`
   impl or hashes diverge. Note `Ipv4Addr`'s hashed name is `"IpV4Addr"`
   (`impl/net.rs:56`) and `Ipv6Addr` `"IpV6Addr"` (`impl/net.rs:77`).

8. **`InvalidBytes` (3) and `InvalidList` (5) are never raised** by any decode
   path. A decoder need not generate them but must still accept/round-trip the
   discriminant bytes `03`/`05`.

9. **Endianness is fixed to big-endian throughout** (`to_be_bytes`/`from_be_bytes`
   everywhere). No host-endianness dependence in the value bytes. The only place
   a `VarU64` differs is group order (little-endian *groups*) — §2.1.

10. **No explicit maximum message size / recursion-depth limit** beyond the
    per-`Vec` `u32::MAX`-byte budget guard (`de.rs:211-216`). The
    `serialize_for_hash` buffer is a fixed 4096 bytes (`lib.rs:188`), so
    **schemas whose canonical hash bytes exceed ~4 KiB overflow at compile time**
    — a schema-size bound, not a message bound.

11. **Chunk boundaries in snapshots are not semantic** — they reflect byte-rope
    chunking, not delimiters. Only the concatenation is the wire format.

---

### Appendix A — Quick byte-layout cheat sheet

```
()                : (nothing)
bool              : 1 byte (00/01; decode: 0=false else true)
u8/i8             : 1 byte
u16..u128/i16..   : 2/4/8/16 bytes, big-endian, two's-complement for signed
usize/isize       : 8 bytes BE (u64/i64), range-checked on decode
VarU64            : LEB128, 1..10 bytes, 7 bits/byte, high bit=continue
String/Arc<str>   : [u64 BE len][utf8]
Vec<u8>/Bytes/    : [u64 BE len][raw bytes]
  ByteVec/Tagged
[u8; N]           : N raw bytes (NO length prefix)
Vec<T>/VecDeque<T>: [u64 BE count][elem…]
Option<T>         : 00 | 01 <T>
Result<T,E>       : 00 <T> | 01 <E>
Box<T>            : <T>  (transparent)
(A,B,…)/struct    : <A><B>…  (positional, NO count/prefix)
BTreeMap<K,V>     : [u64 BE count][<K><V>…]  (keys ASCENDING sorted)
Duration          : [u64 BE secs][u32 BE subsec_nanos]
Ipv4Addr          : 4 raw octets
Ipv6Addr          : 16 raw octets
IpAddr            : 04 <Ipv4> | 06 <Ipv6>
SocketAddr        : <IpAddr><u16 BE port>
Range/RangeIncl.  : <start><end>
Bound<T>          : 00 | 01 <T> | 02 <T>
io::ErrorKind     : 1 byte (0..36; unknown→255 out / Other in)
io::Error         : <ErrorKind><String message>
de::ErrorKind     : 1 byte (declaration index 0..5)
tagged envelope   : [u64 BE global_id = SHA256(name ++ 00 ++ canon)[0..8]] <value>
TaggedWire field  : [tag_v u8][tag_s u8][u64 BE len][value]   (repeated)
```
