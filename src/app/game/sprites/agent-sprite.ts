import Phaser from 'phaser';
import { TILE_SIZE } from '../map/office-map';
import { eventBus } from '../event-bus';
import type { AgentState } from '../../models/agent.model';

const STATE_COLORS: Record<string, number> = {
  Idle: 0x4a90d9,
  Walking: 0x50c878,
  Thinking: 0xf5c542,
  Coding: 0x9b59b6,
  Reading: 0x3cb371,
  Searching: 0xe67e22,
  Error: 0xe74c3c,
};

const BUBBLE_TEXT: Record<string, (state: AgentState) => string | null> = {
  Idle: () => null,
  Walking: () => null,
  Thinking: (s) =>
    'taskSummary' in s && s.taskSummary ? `\u2699 ${s.taskSummary}` : '\u2699 Thinking...',
  Coding: (s) => ('file' in s && s.file ? `\u2328 ${s.file}` : '\u2328 Writing code...'),
  Reading: () => '\uD83D\uDCD6 Reading...',
  Searching: () => '\uD83D\uDD0D Searching...',
  Error: () => '\u26A0 Error!',
};

/**
 * AgentSprite uses COMPOSITION — it HAS a container, not IS a container.
 * This avoids inheriting unwanted methods from Phaser.GameObjects.Container
 * and follows the recommended pattern from game design literature.
 */
export class AgentSprite {
  private _agentId: string;
  readonly container: Phaser.GameObjects.Container;

  get agentId(): string {
    return this._agentId;
  }

  private scene: Phaser.Scene;
  private bodyCircle: Phaser.GameObjects.Arc;
  private headCircle: Phaser.GameObjects.Arc;
  private nameLabel: Phaser.GameObjects.Text;
  private speechBubble: Phaser.GameObjects.Text;
  private currentState: AgentState = { kind: 'Idle' };
  private movePath: Array<{ x: number; y: number }> = [];
  private moveIndex = 0;
  private isMoving = false;
  private bobTween: Phaser.Tweens.Tween | null = null;
  private arrivalState: AgentState | null = null;
  private bubbleTween: Phaser.Tweens.Tween | null = null;

  constructor(scene: Phaser.Scene, agentId: string, name: string, tileX: number, tileY: number) {
    this.scene = scene;
    this._agentId = agentId;

    const worldX = tileX * TILE_SIZE + TILE_SIZE / 2;
    const worldY = tileY * TILE_SIZE + TILE_SIZE / 2;

    this.container = scene.add.container(worldX, worldY);

    // Body
    this.bodyCircle = scene.add.circle(0, 2, 10, STATE_COLORS['Idle']);
    this.container.add(this.bodyCircle);

    // Head
    this.headCircle = scene.add.circle(0, -8, 6, STATE_COLORS['Idle']);
    this.container.add(this.headCircle);

    // Name label — always visible above head
    this.nameLabel = scene.add
      .text(0, -22, name, {
        fontSize: '8px',
        fontFamily: 'monospace',
        color: '#ffffff',
        backgroundColor: '#1a1a2e',
        padding: { x: 3, y: 1 },
      })
      .setOrigin(0.5);
    this.container.add(this.nameLabel);

    // Speech bubble — visible when agent is active
    this.speechBubble = scene.add
      .text(0, -36, '', {
        fontSize: '7px',
        fontFamily: 'monospace',
        color: '#1a1a2e',
        backgroundColor: '#f5c542',
        padding: { x: 4, y: 2 },
        wordWrap: { width: 100 },
      })
      .setOrigin(0.5, 1)
      .setVisible(false);
    this.container.add(this.speechBubble);

    // Make interactive
    this.container.setSize(TILE_SIZE, TILE_SIZE);
    this.container.setInteractive({ useHandCursor: true });
    this.container.on('pointerdown', (pointer: Phaser.Input.Pointer) => {
      pointer.event.stopPropagation();
      eventBus.emit('agent-selected', { agentId: this.agentId });
    });
  }

  get x(): number {
    return this.container.x;
  }

  get y(): number {
    return this.container.y;
  }

  get tileX(): number {
    return Math.floor(this.container.x / TILE_SIZE);
  }

  get tileY(): number {
    return Math.floor(this.container.y / TILE_SIZE);
  }

  setDepth(depth: number): this {
    this.container.setDepth(depth);
    return this;
  }

  /**
   * Update the visual state of the agent sprite.
   * If `emitEvent` is true, emits the state change to the EventBus
   * (for changes originating from Phaser, e.g. walking arrival).
   * External state changes (from ChatService) set emitEvent=false
   * to avoid infinite loops.
   */
  setAgentState(state: AgentState, emitEvent = true): void {
    this.currentState = state;
    const color = STATE_COLORS[state.kind] ?? STATE_COLORS['Idle'];
    this.bodyCircle.setFillStyle(color);
    this.headCircle.setFillStyle(color);
    this.updateSpeechBubble(state);

    if (emitEvent) {
      eventBus.emit('agent-state-changed', {
        agentId: this.agentId,
        state,
        position: { x: this.tileX, y: this.tileY },
      });
    }
  }

  walkTo(path: Array<{ x: number; y: number }>): void {
    if (path.length === 0) return;

    this.movePath = path;
    this.moveIndex = 0;
    this.isMoving = true;
    this.arrivalState = null;
    this.setAgentState({ kind: 'Walking', target: path[path.length - 1] });
    this.startBobTween();
    this.moveToNext();
  }

  /** Walk to a destination and set a state on arrival. Does NOT emit EventBus events. */
  walkToExternal(path: Array<{ x: number; y: number }>, arrivalState: AgentState): void {
    if (path.length === 0) return;

    this.movePath = path;
    this.moveIndex = 0;
    this.isMoving = true;
    this.arrivalState = arrivalState;
    this.setAgentState({ kind: 'Walking', target: path[path.length - 1] }, false);
    this.startBobTween();
    this.moveToNext();
  }

  private moveToNext(): void {
    if (this.moveIndex >= this.movePath.length) {
      this.arriveAtDestination();
      return;
    }

    const target = this.movePath[this.moveIndex];
    const targetX = target.x * TILE_SIZE + TILE_SIZE / 2;
    const targetY = target.y * TILE_SIZE + TILE_SIZE / 2;
    const distance = Phaser.Math.Distance.Between(this.container.x, this.container.y, targetX, targetY);
    const duration = (distance / TILE_SIZE) * 200;

    this.scene.tweens.add({
      targets: this.container,
      x: targetX,
      y: targetY,
      duration: Math.max(duration, 50),
      ease: 'Linear',
      onComplete: () => {
        this.moveIndex++;
        this.moveToNext();
      },
    });
  }

  private arriveAtDestination(): void {
    this.isMoving = false;
    this.stopBobTween();

    const isExternal = this.arrivalState !== null;
    const targetState = this.arrivalState ?? { kind: 'Idle' as const };
    this.arrivalState = null;

    // External walks don't emit back to EventBus to avoid loops
    this.setAgentState(targetState, !isExternal);

    if (!isExternal) {
      eventBus.emit('agent-arrived', {
        agentId: this.agentId,
        tileX: this.tileX,
        tileY: this.tileY,
      });
    }
  }

  private startBobTween(): void {
    this.stopBobTween();
    this.bobTween = this.scene.tweens.add({
      targets: this.headCircle,
      y: { from: -8, to: -10 },
      duration: 250,
      yoyo: true,
      repeat: -1,
      ease: 'Sine.easeInOut',
    });
  }

  private stopBobTween(): void {
    if (this.bobTween) {
      this.bobTween.stop();
      this.bobTween = null;
      this.headCircle.y = -8;
    }
  }

  getIsMoving(): boolean {
    return this.isMoving;
  }

  private updateSpeechBubble(state: AgentState): void {
    const getText = BUBBLE_TEXT[state.kind];
    const text = getText?.(state) ?? null;

    if (text) {
      this.speechBubble.setText(text).setVisible(true);
      this.startBubbleFloat();
    } else {
      this.speechBubble.setVisible(false);
      this.stopBubbleFloat();
    }
  }

  private startBubbleFloat(): void {
    this.stopBubbleFloat();
    this.bubbleTween = this.scene.tweens.add({
      targets: this.speechBubble,
      y: { from: -36, to: -39 },
      duration: 1000,
      yoyo: true,
      repeat: -1,
      ease: 'Sine.easeInOut',
    });
  }

  private stopBubbleFloat(): void {
    if (this.bubbleTween) {
      this.bubbleTween.stop();
      this.bubbleTween = null;
      this.speechBubble.y = -36;
    }
  }

  /** Reconfigure this sprite for a new agent. Called when taken from the pool. */
  init(agentId: string, name: string, tileX: number, tileY: number): void {
    this._agentId = agentId;
    const worldX = tileX * TILE_SIZE + TILE_SIZE / 2;
    const worldY = tileY * TILE_SIZE + TILE_SIZE / 2;
    this.container.setPosition(worldX, worldY);
    this.nameLabel.setText(name);
    this.bodyCircle.setFillStyle(STATE_COLORS['Idle']);
    this.headCircle.setFillStyle(STATE_COLORS['Idle']);
    this.headCircle.y = -8;
    this.currentState = { kind: 'Idle' };
    this.movePath = [];
    this.moveIndex = 0;
    this.isMoving = false;
    this.arrivalState = null;
    this.speechBubble.setText('').setVisible(false);
    this.container.setActive(true).setVisible(true);
    this.container.setInteractive({ useHandCursor: true });
  }

  /** Return this sprite to the pool. Hides it and clears all state. */
  reset(): void {
    this.stopBobTween();
    this.stopBubbleFloat();
    this.movePath = [];
    this.moveIndex = 0;
    this.isMoving = false;
    this.arrivalState = null;
    this.currentState = { kind: 'Idle' };
    this._agentId = '';
    this.container.setActive(false).setVisible(false);
    this.container.disableInteractive();
  }

  destroy(): void {
    this.stopBobTween();
    this.stopBubbleFloat();
    this.container.destroy();
  }
}
