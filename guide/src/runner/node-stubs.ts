/// Browser stubs for the Node built-ins that `@bytecodealliance/jco-transpile` imports at module
/// top-level. The guide only ever transpiles WITHOUT optimize/minify/asm.js/writeFiles, so the code
/// paths that would actually touch the filesystem, child processes, or os are never executed — these
/// stubs exist only so the named imports RESOLVE in a browser bundle. Any stub that is actually
/// invoked throws loudly, which would signal a misconfigured transpile call.
///
/// Vite aliases `node:process`, `node:os`, `node:child_process`, `node:fs/promises`, and `node:util`
/// to this file (see vite.config.ts). `node:path`, `node:buffer`, and `node:url` have real browser
/// shims Vite/Rollup provide, so they are left alone.

function unavailable(name: string): never {
  throw new Error(
    `${name} is not available in the browser build — the guide transpiles without any Node-only ` +
      `code path (optimize/minify/asm.js/writeFiles are all off).`,
  );
}

// --- node:process ---------------------------------------------------------------------------------
export const platform = "browser";
export const argv0 = "browser";
export const env: Record<string, string | undefined> = {};

// --- node:os --------------------------------------------------------------------------------------
export function tmpdir(): string {
  return "/tmp";
}

// --- node:child_process ---------------------------------------------------------------------------
export function spawn(): never {
  unavailable("spawn");
}

// --- node:fs/promises -----------------------------------------------------------------------------
export function readFile(): never {
  unavailable("fs.readFile");
}
export function writeFile(): never {
  unavailable("fs.writeFile");
}
export function rm(): never {
  unavailable("fs.rm");
}
export function mkdtemp(): never {
  unavailable("fs.mkdtemp");
}
export function mkdir(): never {
  unavailable("fs.mkdir");
}

// --- node:util ------------------------------------------------------------------------------------
export const promisify = <T>(fn: T): T => fn;
export const TextEncoder = globalThis.TextEncoder;
export const TextDecoder = globalThis.TextDecoder;
// `styleText(color, s)` colorizes terminal text; in the browser just return the text unchanged.
export function styleText(_style: unknown, text: string): string {
  return text;
}

// --- node:path ------------------------------------------------------------------------------------
// Minimal POSIX-ish path helpers. Only string manipulation — safe and used only for naming, never
// for real filesystem access (which is stubbed above).
export const sep = "/";
export function join(...parts: string[]): string {
  return parts.filter(Boolean).join("/").replace(/\/+/g, "/");
}
export function normalize(p: string): string {
  return p.replace(/\/+/g, "/");
}
export function resolve(...parts: string[]): string {
  return join(...parts);
}
export function dirname(p: string): string {
  const i = p.replace(/\/+$/, "").lastIndexOf("/");
  return i <= 0 ? "/" : p.slice(0, i);
}
export function basename(p: string, ext?: string): string {
  const b = p.slice(p.lastIndexOf("/") + 1);
  return ext && b.endsWith(ext) ? b.slice(0, -ext.length) : b;
}
export function extname(p: string): string {
  const b = p.slice(p.lastIndexOf("/") + 1);
  const i = b.lastIndexOf(".");
  return i > 0 ? b.slice(i) : "";
}

// --- node:url -------------------------------------------------------------------------------------
export function fileURLToPath(u: string | URL): string {
  return String(u).replace(/^file:\/\//, "");
}

// --- node:buffer ----------------------------------------------------------------------------------
// jco-transpile uses `Buffer.from(...).toString('utf8')` and `Buffer.from(str)` on the minify/asm
// paths only (never taken here). Provide a tiny shim over TextEncoder/Decoder so the name resolves.
export const Buffer = {
  from(input: string | Uint8Array | ArrayBuffer): Uint8Array & { toString(enc?: string): string } {
    const bytes =
      typeof input === "string"
        ? new globalThis.TextEncoder().encode(input)
        : input instanceof Uint8Array
          ? input
          : new Uint8Array(input);
    const out = bytes as Uint8Array & { toString(enc?: string): string };
    out.toString = () => new globalThis.TextDecoder().decode(bytes);
    return out;
  },
};

// A permissive default export so `import x from 'node:*'` / `import * as x` style access finds members.
export default {
  platform,
  argv0,
  env,
  tmpdir,
  spawn,
  readFile,
  writeFile,
  rm,
  mkdtemp,
  mkdir,
  promisify,
  TextEncoder,
  TextDecoder,
  styleText,
  sep,
  join,
  normalize,
  resolve,
  dirname,
  basename,
  extname,
  fileURLToPath,
  Buffer,
};
