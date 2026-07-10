import type { StreamEvent } from './chat.model';

describe('StreamEvent.embedder_download discriminant', () => {
  it('narrows correctly when kind === "embedder_download"', () => {
    const e: StreamEvent = {
      kind: 'embedder_download',
      phase: 'downloading',
      bytes_downloaded: 50,
      bytes_total: 100,
    };

    if (e.kind === 'embedder_download') {
      // Inside this branch, all of these accesses MUST type-check.
      const phase: 'start' | 'downloading' | 'verifying' | 'done' | 'error' = e.phase;
      const bd: number | undefined = e.bytes_downloaded;
      const bt: number | undefined = e.bytes_total;
      const err: string | undefined = e.error;

      expect(phase).toBe('downloading');
      expect(bd).toBe(50);
      expect(bt).toBe(100);
      expect(err).toBeUndefined();
    } else {
      throw new Error('expected narrowing to embedder_download branch');
    }
  });

  it('accepts all 5 phases as valid discriminant values', () => {
    const phases: Array<'start' | 'downloading' | 'verifying' | 'done' | 'error'> = [
      'start',
      'downloading',
      'verifying',
      'done',
      'error',
    ];
    for (const p of phases) {
      const e: StreamEvent = { kind: 'embedder_download', phase: p };
      expect(e.kind).toBe('embedder_download');
      if (e.kind === 'embedder_download') {
        expect(e.phase).toBe(p);
      }
    }
  });

  it('rejects access to embedder_download fields on other discriminants at compile time', () => {
    const e: StreamEvent = { kind: 'delta', delta: 'hi' };

    if (e.kind === 'delta') {
      // @ts-expect-error - `phase` does not exist on the delta variant
      const _phase = e.phase;
      // @ts-expect-error - `bytes_downloaded` does not exist on the delta variant
      const _bd = e.bytes_downloaded;
      // @ts-expect-error - `bytes_total` does not exist on the delta variant
      const _bt = e.bytes_total;
      void _phase;
      void _bd;
      void _bt;
    }

    // Runtime sanity: the delta event itself still works.
    expect(e.kind).toBe('delta');
  });

  it('error variant carries the error string', () => {
    const e: StreamEvent = {
      kind: 'embedder_download',
      phase: 'error',
      error: 'boom',
    };
    if (e.kind === 'embedder_download') {
      expect(e.phase).toBe('error');
      expect(e.error).toBe('boom');
    }
  });
});
