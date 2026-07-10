import Phaser from 'phaser';

const TILESET_COUNT = 4;

/**
 * BootScene processes tileset PNGs at load time:
 * - Loads raw images + atlas JSON descriptors
 * - Replaces near-black pixels (#000000) with transparent (the tilesets use solid black as background)
 * - Creates proper Phaser atlas textures from the processed images
 * - Transitions to OfficeScene once all atlases are ready
 */
export class BootScene extends Phaser.Scene {
  constructor() {
    super({ key: 'BootScene' });
  }

  preload(): void {
    for (let i = 1; i <= TILESET_COUNT; i++) {
      this.load.image(`_raw${i}`, `tileset/tileset_${i}.png`);
      this.load.json(`_atlas${i}`, `tileset/atlas_${i}.json`);
    }
  }

  create(): void {
    let processed = 0;

    for (let i = 1; i <= TILESET_COUNT; i++) {
      this.processAtlas(i, () => {
        processed++;
        if (processed === TILESET_COUNT) {
          this.scene.start('OfficeScene');
        }
      });
    }
  }

  private processAtlas(index: number, onDone: () => void): void {
    const src = this.textures.get(`_raw${index}`).getSourceImage() as HTMLImageElement;
    const w = src.naturalWidth || src.width;
    const h = src.naturalHeight || src.height;

    const canvas = document.createElement('canvas');
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext('2d')!;
    ctx.drawImage(src, 0, 0);

    // Replace near-black pixels with transparent
    const imgData = ctx.getImageData(0, 0, w, h);
    const px = imgData.data;
    for (let j = 0; j < px.length; j += 4) {
      if (px[j] < 15 && px[j + 1] < 15 && px[j + 2] < 15) {
        px[j + 3] = 0;
      }
    }
    ctx.putImageData(imgData, 0, 0);

    const img = new Image();
    img.onload = () => {
      const atlasData = this.cache.json.get(`_atlas${index}`);
      this.textures.addAtlas(`tileset${index}`, img, atlasData);
      this.textures.remove(`_raw${index}`);
      onDone();
    };
    img.src = canvas.toDataURL();
  }
}
