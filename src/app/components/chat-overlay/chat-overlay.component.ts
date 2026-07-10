import {
  Component,
  ChangeDetectionStrategy,
  inject,
  signal,
  computed,
  effect,
  NgZone,
  DestroyRef,
} from '@angular/core';
import { AgentService } from '../../services/agent.service';
import { ChatService } from '../../services/chat.service';
import { ChatPanelComponent } from '../chat-panel/chat-panel.component';
import { eventBus } from '../../game/event-bus';

export type DockPosition = 'bottom' | 'top' | 'left' | 'right';
type ResizeAxis = 'width' | 'height';

interface DockDimensions {
  width: number;
  height: number;
}

interface OverlayLayout {
  position: DockPosition;
  dims: Record<DockPosition, DockDimensions>;
}

const STORAGE_KEY = 'chat-overlay-layout';
const EDGE_THRESHOLD = 80;
const MIN_W = 280;
const MIN_H = 200;

const DEFAULT_DIMS: Record<DockPosition, DockDimensions> = {
  bottom: { width: 480, height: 380 },
  top: { width: 480, height: 380 },
  left: { width: 380, height: 0 },
  right: { width: 380, height: 0 },
};

function loadLayout(): OverlayLayout {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const p = JSON.parse(raw);
      const dims = { ...DEFAULT_DIMS };
      for (const key of Object.keys(dims) as DockPosition[]) {
        if (p.dims?.[key]) dims[key] = { ...dims[key], ...p.dims[key] };
      }
      return { position: p.position ?? 'bottom', dims };
    }
  } catch { /* defaults */ }
  return { position: 'bottom', dims: { ...DEFAULT_DIMS } };
}

function saveLayout(layout: OverlayLayout): void {
  try { localStorage.setItem(STORAGE_KEY, JSON.stringify(layout)); } catch { /* */ }
}

function clamp(val: number, min: number, max: number): number {
  return Math.max(min, Math.min(val, max));
}

@Component({
  selector: 'app-chat-overlay',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ChatPanelComponent],
  templateUrl: './chat-overlay.component.html',
  styleUrl: './chat-overlay.component.css',
  host: {
    '[class]': 'hostClass()',
    '[style.--ov-w.px]': 'currentWidth()',
    '[style.--ov-h.px]': 'currentHeight()',
  },
})
export class ChatOverlayComponent {
  private readonly agentService = inject(AgentService);
  private readonly zone = inject(NgZone);
  private readonly destroyRef = inject(DestroyRef);
  readonly chatService = inject(ChatService);

  readonly isOpen = signal(false);
  readonly activeAgent = computed(() => this.agentService.selectedAgent());
  readonly position = signal<DockPosition>('bottom');
  readonly dims = signal<Record<DockPosition, DockDimensions>>({ ...DEFAULT_DIMS });
  readonly isDragging = signal(false);
  readonly dragTarget = signal<DockPosition | null>(null);

  readonly hostClass = computed(() => `pos-${this.position()}`);
  readonly isHorizontalDock = computed(() => {
    const p = this.position();
    return p === 'bottom' || p === 'top';
  });

  readonly currentWidth = computed(() => this.dims()[this.position()].width);
  readonly currentHeight = computed(() => this.dims()[this.position()].height);

  private resizeAxis: ResizeAxis | null = null;
  private boundMove: ((e: MouseEvent) => void) | null = null;
  private boundUp: ((e: MouseEvent) => void) | null = null;
  private readonly onAgentSelected = () => this.isOpen.set(true);

  constructor() {
    const saved = loadLayout();
    this.position.set(saved.position);
    this.dims.set(saved.dims);

    // Open overlay on every NPC click — eventBus fires every time,
    // unlike signals which skip if the same agent is already selected
    eventBus.on('agent-selected', this.onAgentSelected);

    effect(() => {
      saveLayout({ position: this.position(), dims: this.dims() });
    });

    this.destroyRef.onDestroy(() => {
      this.cleanup();
      eventBus.off('agent-selected', this.onAgentSelected);
    });
  }

  close(): void {
    this.isOpen.set(false);
  }

  // ── Resize ──

  startResize(event: MouseEvent, axis: ResizeAxis): void {
    event.preventDefault();
    event.stopPropagation();
    this.resizeAxis = axis;
    this.listen(
      (e) => this.onResizeMove(e),
      () => { this.resizeAxis = null; this.cleanup(); },
    );
  }

  private onResizeMove(e: MouseEvent): void {
    const pos = this.position();
    const axis = this.resizeAxis!;
    const vw = window.innerWidth;
    const vh = window.innerHeight;

    let newVal: number;

    if (axis === 'height') {
      newVal = pos === 'bottom' ? vh - e.clientY : e.clientY;
      newVal = clamp(newVal, MIN_H, Math.floor(vh * 0.8));
    } else {
      if (pos === 'left') {
        newVal = e.clientX;
      } else if (pos === 'right') {
        newVal = vw - e.clientX;
      } else {
        // Centered: symmetric resize — width = 2 * distance from center
        newVal = 2 * Math.abs(e.clientX - vw / 2);
      }
      newVal = clamp(newVal, MIN_W, Math.floor(vw * 0.9));
    }

    this.dims.update((d) => ({
      ...d,
      [pos]: { ...d[pos], [axis]: newVal },
    }));
  }

  // ── Header drag (reposition) ──

  onHeaderDown(event: MouseEvent): void {
    if (event.button !== 0 || (event.target as HTMLElement).closest('.close-btn')) return;
    event.preventDefault();
    this.isDragging.set(true);
    this.listen(
      (e) => this.dragTarget.set(this.detectEdge(e.clientX, e.clientY)),
      (e) => this.finishDrag(e),
    );
  }

  private finishDrag(e: MouseEvent): void {
    const edge = this.detectEdge(e.clientX, e.clientY);
    if (edge && edge !== this.position()) this.position.set(edge);
    this.isDragging.set(false);
    this.dragTarget.set(null);
    this.cleanup();
  }

  private detectEdge(x: number, y: number): DockPosition | null {
    const dists: [DockPosition, number][] = [
      ['top', y],
      ['bottom', window.innerHeight - y],
      ['left', x],
      ['right', window.innerWidth - x],
    ];
    dists.sort((a, b) => a[1] - b[1]);
    return dists[0][1] <= EDGE_THRESHOLD ? dists[0][0] : null;
  }

  // ── Listeners ──

  private listen(
    onMove: (e: MouseEvent) => void,
    onUp: (e: MouseEvent) => void,
  ): void {
    this.cleanup();
    this.zone.runOutsideAngular(() => {
      this.boundMove = (e: MouseEvent) => this.zone.run(() => onMove(e));
      this.boundUp = (e: MouseEvent) => this.zone.run(() => onUp(e));
      window.addEventListener('mousemove', this.boundMove);
      window.addEventListener('mouseup', this.boundUp);
    });
  }

  private cleanup(): void {
    if (this.boundMove) { window.removeEventListener('mousemove', this.boundMove); this.boundMove = null; }
    if (this.boundUp) { window.removeEventListener('mouseup', this.boundUp); this.boundUp = null; }
  }
}
