import { Component, ChangeDetectionStrategy, signal, inject } from '@angular/core';
import { GameCanvasComponent } from './components/game-canvas/game-canvas.component';
import { SidebarComponent } from './components/sidebar/sidebar.component';
import { ChatOverlayComponent } from './components/chat-overlay/chat-overlay.component';
import { ConfirmationModalComponent } from './components/confirmation-modal/confirmation-modal.component';
import { SessionService } from './services/session.service';

@Component({
  selector: 'app-root',
  imports: [GameCanvasComponent, SidebarComponent, ChatOverlayComponent, ConfirmationModalComponent],
  templateUrl: './app.html',
  styleUrl: './app.css',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class App {
  private readonly sessionService = inject(SessionService);
  readonly sidebarOpen = signal(false);

  constructor() {
    this.sessionService.initialize();
  }

  toggleSidebar(): void {
    this.sidebarOpen.update((v) => !v);
  }

  closeSidebar(): void {
    this.sidebarOpen.set(false);
  }
}
