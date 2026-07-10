import Phaser from 'phaser';
import { TILE_SIZE } from '../map/office-map';

const USER_COLOR = 0xf0c040;
const USER_HEAD_COLOR = 0xffe080;

/**
 * The player-controlled sprite. NOT an agent — this is the user
 * who moves around the office with WASD/arrows and interacts with agents.
 */
export class UserSprite {
  readonly container: Phaser.GameObjects.Container;

  private scene: Phaser.Scene;
  private bodyCircle: Phaser.GameObjects.Arc;
  private headCircle: Phaser.GameObjects.Arc;
  private nameLabel: Phaser.GameObjects.Text;
  private movePath: Array<{ x: number; y: number }> = [];
  private moveIndex = 0;
  private isMoving = false;
  private bobTween: Phaser.Tweens.Tween | null = null;

  constructor(scene: Phaser.Scene, tileX: number, tileY: number) {
    this.scene = scene;

    const worldX = tileX * TILE_SIZE + TILE_SIZE / 2;
    const worldY = tileY * TILE_SIZE + TILE_SIZE / 2;

    this.container = scene.add.container(worldX, worldY);

    // Body — gold color to distinguish from agents
    this.bodyCircle = scene.add.circle(0, 2, 10, USER_COLOR);
    this.container.add(this.bodyCircle);

    // Head
    this.headCircle = scene.add.circle(0, -8, 6, USER_HEAD_COLOR);
    this.container.add(this.headCircle);

    // Name label
    this.nameLabel = scene.add
      .text(0, -20, 'You', {
        fontSize: '8px',
        fontFamily: '"Press Start 2P", monospace',
        color: '#f0c040',
        backgroundColor: '#00000088',
        padding: { x: 2, y: 1 },
      })
      .setOrigin(0.5);
    this.container.add(this.nameLabel);
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

  getIsMoving(): boolean {
    return this.isMoving;
  }

  walkTo(path: Array<{ x: number; y: number }>): void {
    if (path.length === 0) return;

    this.movePath = path;
    this.moveIndex = 0;
    this.isMoving = true;
    this.startBobTween();
    this.moveToNext();
  }

  private moveToNext(): void {
    if (this.moveIndex >= this.movePath.length) {
      this.isMoving = false;
      this.stopBobTween();
      return;
    }

    const target = this.movePath[this.moveIndex];
    const targetX = target.x * TILE_SIZE + TILE_SIZE / 2;
    const targetY = target.y * TILE_SIZE + TILE_SIZE / 2;
    const distance = Phaser.Math.Distance.Between(
      this.container.x,
      this.container.y,
      targetX,
      targetY,
    );
    const duration = (distance / TILE_SIZE) * 150;

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

  private startBobTween(): void {
    this.stopBobTween();
    this.bobTween = this.scene.tweens.add({
      targets: this.headCircle,
      y: { from: -8, to: -10 },
      duration: 200,
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

  destroy(): void {
    this.stopBobTween();
    this.container.destroy();
  }
}
