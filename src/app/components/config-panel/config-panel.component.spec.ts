import { TestBed, type ComponentFixture } from '@angular/core/testing';
import { signal } from '@angular/core';
import { ConfigPanelComponent } from './config-panel.component';
import { EmbedderService, type EmbedderStatus } from '../../services/embedder.service';

/**
 * Minimal stub that exposes the surface ConfigPanel actually reads/calls.
 * Driving the underlying signal from the test gives us deterministic phase rendering
 * without the real `listen()` lifecycle.
 */
class EmbedderServiceStub {
  readonly status = signal<EmbedderStatus>({ phase: 'idle' });
  readonly retryCalls: number[] = [];

  retry = async (): Promise<void> => {
    this.retryCalls.push(Date.now());
  };
}

function setupFixture(): {
  fixture: ComponentFixture<ConfigPanelComponent>;
  embedder: EmbedderServiceStub;
} {
  TestBed.configureTestingModule({
    imports: [ConfigPanelComponent],
    providers: [{ provide: EmbedderService, useClass: EmbedderServiceStub }],
  });
  const fixture = TestBed.createComponent(ConfigPanelComponent);
  const embedder = TestBed.inject(EmbedderService) as unknown as EmbedderServiceStub;
  return { fixture, embedder };
}

describe('ConfigPanelComponent — Embedder section', () => {
  afterEach(() => {
    TestBed.resetTestingModule();
  });

  it('renders the idle placeholder when phase === "idle"', () => {
    const { fixture, embedder } = setupFixture();
    embedder.status.set({ phase: 'idle' });
    fixture.detectChanges();

    const root = fixture.nativeElement as HTMLElement;
    const section = root.querySelector<HTMLElement>('section.embedder-section');
    expect(section).not.toBeNull();
    expect(section?.getAttribute('role')).toBe('region');
    expect(section?.getAttribute('aria-labelledby')).toBe('embedder-heading');
    expect(section?.textContent ?? '').toMatch(/not yet started/i);
  });

  it('renders a progressbar with integer aria-valuenow when downloading', () => {
    const { fixture, embedder } = setupFixture();
    embedder.status.set({
      phase: 'downloading',
      bytesDownloaded: 100,
      bytesTotal: 200,
    });
    fixture.detectChanges();

    const root = fixture.nativeElement as HTMLElement;
    const bars = root.querySelectorAll('[role="progressbar"]');
    expect(bars.length).toBe(1);

    const bar = bars[0] as HTMLElement;
    expect(bar.getAttribute('aria-valuemin')).toBe('0');
    expect(bar.getAttribute('aria-valuemax')).toBe('100');
    expect(bar.getAttribute('aria-valuenow')).toBe('50');
    expect(bar.getAttribute('aria-label')).toMatch(/embedder download/i);
  });

  it('renders role="status" + aria-live="polite" when verifying', () => {
    const { fixture, embedder } = setupFixture();
    embedder.status.set({ phase: 'verifying' });
    fixture.detectChanges();

    const root = fixture.nativeElement as HTMLElement;
    const live = root.querySelector('section.embedder-section [role="status"]');
    expect(live).not.toBeNull();
    expect(live?.getAttribute('aria-live')).toBe('polite');
    expect(live?.textContent ?? '').toMatch(/verifying/i);
  });

  it('renders role="alert" + a retry button when phase === "error"', () => {
    const { fixture, embedder } = setupFixture();
    embedder.status.set({ phase: 'error', error: 'boom' });
    fixture.detectChanges();

    const root = fixture.nativeElement as HTMLElement;
    const alert = root.querySelector('section.embedder-section [role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert?.textContent ?? '').toMatch(/boom/);

    const retryBtn = root.querySelector<HTMLButtonElement>(
      'section.embedder-section button',
    );
    expect(retryBtn).not.toBeNull();
    expect(retryBtn?.getAttribute('type')).toBe('button');
    expect(retryBtn?.textContent ?? '').toMatch(/retry/i);
  });

  it('renders the "embedder ready" message when phase === "done"', () => {
    const { fixture, embedder } = setupFixture();
    embedder.status.set({ phase: 'done' });
    fixture.detectChanges();

    const root = fixture.nativeElement as HTMLElement;
    const section = root.querySelector<HTMLElement>('section.embedder-section');
    expect(section?.textContent ?? '').toMatch(/embedder ready/i);
    expect(section?.textContent ?? '').toMatch(/multilingual-e5-small/);
  });

  it('clicking retry invokes embedderService.retry exactly once', async () => {
    const { fixture, embedder } = setupFixture();
    embedder.status.set({ phase: 'error', error: 'boom' });
    fixture.detectChanges();

    const root = fixture.nativeElement as HTMLElement;
    const retryBtn = root.querySelector<HTMLButtonElement>(
      'section.embedder-section button',
    );
    expect(retryBtn).not.toBeNull();

    retryBtn?.click();
    await fixture.whenStable();

    expect(embedder.retryCalls.length).toBe(1);
  });

  it('clamps progress percent to 0..100 and floors fractional bytes', () => {
    const { fixture, embedder } = setupFixture();

    // 1 of 3 bytes → round(33.33) = 33
    embedder.status.set({ phase: 'downloading', bytesDownloaded: 1, bytesTotal: 3 });
    fixture.detectChanges();
    let bar = (fixture.nativeElement as HTMLElement).querySelector('[role="progressbar"]');
    expect(bar?.getAttribute('aria-valuenow')).toBe('33');

    // 0 of 0 → guard against division by zero, must be 0.
    embedder.status.set({ phase: 'downloading', bytesDownloaded: 0, bytesTotal: 0 });
    fixture.detectChanges();
    bar = (fixture.nativeElement as HTMLElement).querySelector('[role="progressbar"]');
    expect(bar?.getAttribute('aria-valuenow')).toBe('0');

    // downloaded > total (defensive) → clamped to 100.
    embedder.status.set({ phase: 'downloading', bytesDownloaded: 500, bytesTotal: 200 });
    fixture.detectChanges();
    bar = (fixture.nativeElement as HTMLElement).querySelector('[role="progressbar"]');
    expect(bar?.getAttribute('aria-valuenow')).toBe('100');
  });

  it('does not render the progressbar in non-downloading phases', () => {
    const { fixture, embedder } = setupFixture();
    for (const phase of ['idle', 'start', 'verifying', 'done', 'error'] as const) {
      embedder.status.set({ phase, error: phase === 'error' ? 'x' : undefined });
      fixture.detectChanges();
      const bar = (fixture.nativeElement as HTMLElement).querySelector(
        'section.embedder-section [role="progressbar"]',
      );
      // Use a custom failure message via a throw — vitest's expect lacks `.withContext`.
      if (bar !== null) {
        throw new Error(`expected NO progressbar in phase ${phase}`);
      }
      expect(bar).toBeNull();
    }
  });
});
