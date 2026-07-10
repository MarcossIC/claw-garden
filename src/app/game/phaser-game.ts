import Phaser from 'phaser';
import { BootScene } from './scenes/boot-scene';
import { OfficeScene } from './scenes/office-scene';

export function createPhaserGame(parent: HTMLElement): Phaser.Game {
  return new Phaser.Game({
    type: Phaser.AUTO,
    parent,
    pixelArt: true,
    antialias: false,
    roundPixels: true,
    backgroundColor: '#0d0d1a',
    physics: {
      default: 'arcade',
      arcade: { gravity: { x: 0, y: 0 }, debug: false },
    },
    scale: {
      mode: Phaser.Scale.RESIZE,
      autoCenter: Phaser.Scale.CENTER_BOTH,
    },
    scene: [BootScene, OfficeScene],
  });
}
