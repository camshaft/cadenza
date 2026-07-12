/// Offset conversion between CodeMirror and the compiler.
///
/// CodeMirror document offsets count UTF-16 code units; the Cadenza compiler's spans are UTF-8 byte
/// offsets. They coincide for ASCII but diverge on any non-ASCII identifier, string, or comment, so
/// we convert at every boundary: byte→UTF-16 to render a diagnostic/hover range, UTF-16→byte to send
/// the hover cursor to the compiler. Both loops are surrogate-safe (a lone-surrogate `TextEncoder`
/// would miscount).

/// UTF-16 code-unit offset (CodeMirror) → UTF-8 byte offset (compiler).
export function utf16ToByte(str: string, utf16Offset: number): number {
  let bytes = 0;
  for (let i = 0; i < utf16Offset && i < str.length; ) {
    const cp = str.codePointAt(i)!;
    if (cp <= 0x7f) bytes += 1;
    else if (cp <= 0x7ff) bytes += 2;
    else if (cp <= 0xffff) bytes += 3;
    else {
      bytes += 4;
      i++; // a surrogate pair is two UTF-16 units
    }
    i++;
  }
  return bytes;
}

/// UTF-8 byte offset (compiler) → UTF-16 code-unit offset (CodeMirror).
export function byteToUtf16(str: string, byteOffset: number): number {
  let bytes = 0;
  for (let i = 0; i < str.length; ) {
    if (bytes >= byteOffset) return i;
    const cp = str.codePointAt(i)!;
    if (cp <= 0x7f) bytes += 1;
    else if (cp <= 0x7ff) bytes += 2;
    else if (cp <= 0xffff) bytes += 3;
    else {
      bytes += 4;
      i++;
    }
    i++;
  }
  return str.length;
}
