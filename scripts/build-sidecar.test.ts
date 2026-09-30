// Run with `bun run test:scripts` (scoped to this folder: a bare `bun test`
// would also pick up the Angular specs, which run under Vitest instead).
import { describe, expect, test } from 'bun:test';

import { hostTriple, sidecarFileName } from './build-sidecar';

const RUSTC_VV = `rustc 1.91.1 (ed61e7d7e 2025-11-07)
binary: rustc
commit-hash: ed61e7d7e242494fb7057f2657300d9e77bb4fcb
commit-date: 2025-11-07
host: x86_64-pc-windows-msvc
release: 1.91.1
LLVM version: 21.1.2
`;

describe('hostTriple', () => {
  test('reads the host line of `rustc -vV`', () => {
    expect(hostTriple(RUSTC_VV)).toBe('x86_64-pc-windows-msvc');
  });

  test('tolerates CRLF line endings', () => {
    expect(hostTriple(RUSTC_VV.replaceAll('\n', '\r\n'))).toBe('x86_64-pc-windows-msvc');
  });

  test('fails loudly when there is no host line', () => {
    expect(() => hostTriple('rustc 1.91.1\n')).toThrow(/host/);
  });
});

describe('sidecarFileName', () => {
  // Tauri resolves `binaries/clawg` to `binaries/clawg-<target triple>[.exe]`.
  test('suffixes the triple and keeps .exe on Windows', () => {
    expect(sidecarFileName('x86_64-pc-windows-msvc')).toBe('clawg-x86_64-pc-windows-msvc.exe');
  });

  test('has no extension elsewhere', () => {
    expect(sidecarFileName('aarch64-apple-darwin')).toBe('clawg-aarch64-apple-darwin');
  });
});
