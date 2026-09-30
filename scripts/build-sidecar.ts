// Stages `clawg` as the Tauri sidecar of a release bundle.
//
// `bun run tauri:build` passes src-tauri/tauri.release.conf.json, whose
// beforeBuildCommand (`bun run build:release`) runs this before the frontend
// build. It compiles clawg-cli in release and copies the binary to
// src-tauri/binaries/clawg-<host triple>[.exe] — the file Tauri resolves
// `externalBin: ["binaries/clawg"]` to, and installs next to the app as
// clawg.exe.
//
// Why the sidecar is not declared in tauri.conf.json: see
// src-tauri/tests/release_config.rs.

import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';

/** The host target triple, from the output of `rustc -vV`. */
export function hostTriple(rustcVerbose: string): string {
  const line = rustcVerbose.split(/\r?\n/).find((l) => l.startsWith('host: '));
  if (!line) {
    throw new Error('`rustc -vV` printed no host line');
  }
  return line.slice('host: '.length).trim();
}

/** The staged file name Tauri expects for the `clawg` sidecar on `triple`. */
export function sidecarFileName(triple: string): string {
  return `clawg-${triple}${triple.includes('windows') ? '.exe' : ''}`;
}

function run(cmd: string, args: string[], cwd: string): string {
  const result = spawnSync(cmd, args, {
    cwd,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'inherit'],
  });
  if (result.status !== 0) {
    throw new Error(`\`${cmd} ${args.join(' ')}\` exited with ${result.status}`);
  }
  return result.stdout;
}

if (import.meta.main) {
  const root = join(import.meta.dir, '..');
  const triple = hostTriple(run('rustc', ['-vV'], root));

  // --locked: a release is built from the committed lockfile, never from a
  // fresh resolution that nobody reviewed.
  run('cargo', ['build', '--release', '--locked', '-p', 'clawg-cli'], root);

  const targetDir = process.env['CARGO_TARGET_DIR'] ?? join(root, 'target');
  const from = join(targetDir, 'release', triple.includes('windows') ? 'clawg.exe' : 'clawg');
  const to = join(root, 'src-tauri', 'binaries', sidecarFileName(triple));
  mkdirSync(dirname(to), { recursive: true });
  copyFileSync(from, to);
  console.log(`staged ${from} -> ${to}`);
}
