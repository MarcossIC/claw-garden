import { TestBed } from '@angular/core/testing';
import { EmbedderService } from './embedder.service';
import type { StreamEvent } from '../models/chat.model';

describe('EmbedderService', () => {
  let service: EmbedderService;

  beforeEach(() => {
    TestBed.configureTestingModule({});
    service = TestBed.inject(EmbedderService);
  });

  it('initial status is idle', () => {
    expect(service.status()).toEqual({ phase: 'idle' });
  });

  it('updates signal on start event', () => {
    const ev: StreamEvent = { kind: 'embedder_download', phase: 'start' };
    service.handleEventForTest(ev);
    expect(service.status()).toEqual({ phase: 'start' });
  });

  it('updates signal on downloading event with byte counts', () => {
    const ev: StreamEvent = {
      kind: 'embedder_download',
      phase: 'downloading',
      bytes_downloaded: 100,
      bytes_total: 200,
    };
    service.handleEventForTest(ev);
    expect(service.status()).toEqual({
      phase: 'downloading',
      bytesDownloaded: 100,
      bytesTotal: 200,
    });
  });

  it('updates signal on verifying event without byte counts', () => {
    service.handleEventForTest({ kind: 'embedder_download', phase: 'verifying' });
    expect(service.status()).toEqual({ phase: 'verifying' });
  });

  it('updates signal on done event', () => {
    service.handleEventForTest({ kind: 'embedder_download', phase: 'done' });
    expect(service.status()).toEqual({ phase: 'done' });
  });

  it('updates signal on error event with error message', () => {
    service.handleEventForTest({
      kind: 'embedder_download',
      phase: 'error',
      error: 'boom',
    });
    expect(service.status()).toEqual({ phase: 'error', error: 'boom' });
  });

  it('ignores non-embedder StreamEvents', () => {
    service.handleEventForTest({ kind: 'embedder_download', phase: 'downloading', bytes_downloaded: 1, bytes_total: 2 });
    const beforeUnrelated = service.status();

    service.handleEventForTest({ kind: 'delta', delta: 'hello' });
    expect(service.status()).toEqual(beforeUnrelated);

    service.handleEventForTest({ kind: 'state', state: 'thinking' });
    expect(service.status()).toEqual(beforeUnrelated);
  });

  it('retry() invokes the Tauri command and resets status to start', async () => {
    const calls: Array<{ cmd: string; args?: Record<string, unknown> }> = [];
    service.setInvokeForTest(async (cmd, args) => {
      calls.push({ cmd, args });
      return undefined;
    });

    // Put the service in an error state first to make the "reset" assertion meaningful.
    service.handleEventForTest({
      kind: 'embedder_download',
      phase: 'error',
      error: 'previous failure',
    });
    expect(service.status().phase).toBe('error');

    await service.retry();

    expect(calls.length).toBe(1);
    expect(calls[0].cmd).toBe('retry_embedder_download');
    expect(service.status()).toEqual({ phase: 'start' });
  });

  it('retry() surfaces invoke errors as a phase: error status', async () => {
    service.setInvokeForTest(async () => {
      throw new Error('bootstrap rejected');
    });

    await service.retry();

    const s = service.status();
    expect(s.phase).toBe('error');
    expect(s.error).toMatch(/bootstrap rejected/);
  });

  it('strips already-undefined fields rather than persisting them', () => {
    // First populate byte counts.
    service.handleEventForTest({
      kind: 'embedder_download',
      phase: 'downloading',
      bytes_downloaded: 50,
      bytes_total: 100,
    });
    expect(service.status().bytesDownloaded).toBe(50);

    // Then move to verifying without byte counts — they should NOT bleed through.
    service.handleEventForTest({ kind: 'embedder_download', phase: 'verifying' });
    expect(service.status()).toEqual({ phase: 'verifying' });
  });
});
