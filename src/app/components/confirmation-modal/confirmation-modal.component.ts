import {
  Component,
  ChangeDetectionStrategy,
  ElementRef,
  computed,
  effect,
  inject,
  viewChild,
} from '@angular/core';
import { ConfirmationService } from '../../services/confirmation.service';
import type { ConfirmationRequest } from '../../models/chat.model';

@Component({
  selector: 'app-confirmation-modal',
  templateUrl: './confirmation-modal.component.html',
  styleUrl: './confirmation-modal.component.css',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    '(document:keydown)': 'onDocumentKeydown($event)',
  },
})
export class ConfirmationModalComponent {
  private readonly confirmationService = inject(ConfirmationService);

  private readonly denyBtn = viewChild<ElementRef<HTMLButtonElement>>('denyBtn');

  readonly current = this.confirmationService.current;
  readonly hasPending = this.confirmationService.hasPending;

  readonly headline = computed(() => {
    const req = this.current();
    if (!req) return '';
    return req.source === 'destructive_tool'
      ? 'Approve a destructive operation'
      : 'A hook is asking for your approval';
  });

  readonly formattedArgs = computed(() => {
    const req = this.current();
    if (!req) return '';
    try {
      return JSON.stringify(req.arguments, null, 2);
    } catch {
      return String(req.arguments);
    }
  });

  constructor() {
    // Auto-focus the safe (Deny) button when a request is shown so a
    // distracted user pressing Enter does NOT auto-approve. Tracking both
    // signals means this re-runs when either changes: when current() flips
    // null→request the @if creates the button, the viewChild signal updates
    // on the next CD pass, and this effect re-runs with the live element.
    effect(() => {
      const req = this.current();
      const btn = this.denyBtn();
      if (req && btn) {
        btn.nativeElement.focus();
      }
    });
  }

  async approve(req: ConfirmationRequest): Promise<void> {
    await this.confirmationService.resolve(req.call_id, 'allow');
  }

  async deny(req: ConfirmationRequest): Promise<void> {
    await this.confirmationService.resolve(req.call_id, 'deny');
  }

  onDocumentKeydown(event: KeyboardEvent): void {
    const req = this.current();
    if (!req) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      void this.deny(req);
    }
  }
}
