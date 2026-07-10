import {
  Component,
  ChangeDetectionStrategy,
  viewChild,
  ElementRef,
  afterNextRender,
  OnDestroy,
} from '@angular/core';

@Component({
  selector: 'app-game-canvas',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: '<div #gameContainer class="game-container"></div>',
  styles: `
    :host {
      display: block;
      width: 100%;
      height: 100%;
    }
    .game-container {
      width: 100%;
      height: 100%;
    }
  `,
})
export class GameCanvasComponent implements OnDestroy {
  private game: Phaser.Game | null = null;
  private destroyed = false;

  readonly gameContainer = viewChild.required<ElementRef<HTMLDivElement>>('gameContainer');

  constructor() {
    afterNextRender(() => {
      if (this.destroyed) return;

      import('../../game/phaser-game')
        .then(({ createPhaserGame }) => {
          if (this.destroyed) return;
          this.game = createPhaserGame(this.gameContainer().nativeElement);
        })
        .catch((err) => {
          console.error('Failed to initialize Phaser:', err);
        });
    });
  }

  ngOnDestroy(): void {
    this.destroyed = true;
    this.game?.destroy(true);
    this.game = null;
  }
}
