import Phaser from 'phaser';
import {
  TILE_SIZE,
  MAP_COLS,
  MAP_ROWS,
  ROOMS,
  DOORS,
  ZONES,
  isWalkable,
  blockTiles,
} from '../map/office-map';
import { UserSprite } from '../sprites/user-sprite';
import { AgentSprite } from '../sprites/agent-sprite';
import { findPath } from '../pathfinding/grid-pathfinder';
import { eventBus } from '../event-bus';
import type { AgentState } from '../../models/agent.model';
import type { GameCommand } from '../../models/game-command.model';

const T = TILE_SIZE;
const USER_START = { x: 12, y: 5 };

export class OfficeScene extends Phaser.Scene {
  private userSprite!: UserSprite;
  private agentSprites = new Map<string, AgentSprite>();
  private availableAgents: AgentSprite[] = [];
  private readonly POOL_SIZE = 8;
  private commandQueue: GameCommand[] = [];
  private hoverHighlight!: Phaser.GameObjects.Rectangle;
  private cursors?: Phaser.Types.Input.Keyboard.CursorKeys;
  private wasdKeys?: Record<string, Phaser.Input.Keyboard.Key>;

  constructor() {
    super({ key: 'OfficeScene' });
  }

  create(): void {
    this.createFloor();
    this.createWalls();
    this.createFurniture();

    this.hoverHighlight = this.add
      .rectangle(0, 0, T, T, 0xffffff, 0.15)
      .setOrigin(0)
      .setVisible(false)
      .setDepth(9999);

    // User sprite — controlled by WASD/click
    this.userSprite = new UserSprite(this, USER_START.x, USER_START.y);

    this.setupKeyboard();
    this.setupInput();
    this.setupAgentEvents();
    this.setupCamera();
    this.initAgentPool();

    // Notify Angular services that the scene is ready to receive agent events
    eventBus.emit('scene-ready', undefined as unknown as void);
  }

  override update(): void {
    this.processCommandQueue();

    // Y-sort user sprite
    this.userSprite.setDepth(this.userSprite.y);

    // Y-sort all agent sprites
    for (const sprite of this.agentSprites.values()) {
      sprite.setDepth(sprite.y);
    }

    this.handleKeyboardMovement();
  }

  // ── Floor (tiled 2_001 sprites) ────────────────────────

  private createFloor(): void {
    for (const room of ROOMS) {
      const rx = room.x * T;
      const ry = room.y * T;
      const rw = room.w * T;
      const rh = room.h * T;

      for (let y = ry; y < ry + rh; y += 96) {
        for (let x = rx; x < rx + rw; x += 96) {
          const sprite = this.add
            .image(x, y, 'tileset2', '2_001')
            .setOrigin(0, 0)
            .setDepth(0);

          const cropW = Math.min(96, rx + rw - x);
          const cropH = Math.min(96, ry + rh - y);
          if (cropW < 96 || cropH < 96) {
            sprite.setCrop(0, 0, cropW, cropH);
          }
        }
      }
    }

    for (const d of DOORS) {
      const sprite = this.add
        .image(d.x * T, d.y * T, 'tileset2', '2_001')
        .setOrigin(0, 0)
        .setDepth(0);
      sprite.setCrop(0, 0, T, T);
    }
  }

  // ── Walls ──────────────────────────────────────────────

  private createWalls(): void {
    this.createMainNorthWall();
    this.createMainRightWall();
    this.createInnerWalls();
  }

  private createMainNorthWall(): void {
    const y = 0;
    let x = 1 * T;
    const OVERLAP = 5;

    const wall = () => {
      this.add.image(x, y, 'tileset2', '2_000').setOrigin(0, 0).setDepth(1);
      x += 96;
    };

    const window = () => {
      this.add.image(x - OVERLAP, y - 11, 'tileset4', '4_004').setOrigin(0, 0).setDepth(2);
      x += 109 - OVERLAP * 2;
    };

    const door = () => {
      this.add.image(x, y, 'tileset2', '2_002').setOrigin(0, 0).setDepth(1);
      x += 94;
    };

    wall();
    window();
    wall();
    window();
    wall();
    door();
    wall();
    window();
    wall();
    window();
    wall();
    window();

    const roomEndX = 25 * T;
    const remaining = roomEndX - x;
    if (remaining > 0) {
      const lastWall = this.add.image(x, y, 'tileset2', '2_000').setOrigin(0, 0).setDepth(1);
      lastWall.setCrop(0, 0, remaining, 96);
      x += remaining;
    }

    const wallEndX = roomEndX;
    for (let bx = 1 * T; bx < wallEndX; bx += 96) {
      const strip = this.add.image(bx, 96, 'tileset2', '2_083').setOrigin(0, 0).setDepth(1);
      const cropW = Math.min(96, wallEndX - bx);
      if (cropW < 96) {
        strip.setCrop(0, 0, cropW, 16);
      }
    }
  }

  private createMainRightWall(): void {
    const x = 25 * T;
    const endY = 25 * T;
    let wy = 0;

    this.add.image(x, wy, 'tileset1', '1_046').setOrigin(0, 0).setDepth(1);
    wy += 30;

    while (wy + 29 < endY) {
      this.add.image(x, wy, 'tileset1', '1_047').setOrigin(0, 0).setDepth(1);
      wy += 32;
    }

    this.add.image(x, wy, 'tileset1', '1_048').setOrigin(0, 0).setDepth(1);
  }

  private createInnerWalls(): void {
    this.tileSimpleWall(1 * T, 15 * T, 11 * T);
    this.tileSimpleWall(14 * T, 15 * T, 11 * T);
  }

  private tileSimpleWall(startX: number, y: number, width: number): void {
    const endX = startX + width;
    let x = startX;
    while (x < endX) {
      const sprite = this.add.image(x, y, 'tileset2', '2_000').setOrigin(0, 0).setDepth(1);
      const rem = endX - x;
      if (rem < 96) {
        sprite.setCrop(0, 0, rem, 96);
      }
      x += 96;
    }
  }

  // ── Furniture (with collision) ─────────────────────────

  private createFurniture(): void {
    const blocked: Array<{ x: number; y: number }> = [];

    const place = (
      atlas: string,
      frame: string,
      px: number,
      py: number,
      bw = 0,
      bh = 0,
      flipX = false,
    ) => {
      const sprite = this.add
        .image(px, py, atlas, frame)
        .setOrigin(0.5, 1)
        .setDepth(py)
        .setFlipX(flipX);

      if (bw > 0 && bh > 0) {
        const cx = Math.floor(px / T);
        const by = Math.floor((py - 1) / T);
        const left = cx - Math.floor(bw / 2);
        for (let r = by - bh + 1; r <= by; r++) {
          for (let c = left; c < left + bw; c++) {
            blocked.push({ x: c, y: r });
          }
        }
      }

      return sprite;
    };

    // ═══ MAIN WORK AREA ═════════════════════════════════

    place('tileset1', '1_005', 4 * T, 7 * T, 3, 2);
    place('tileset1', '1_004', 7 * T, 7 * T, 3, 2);
    place('tileset1', '1_005', 4 * T, 11 * T, 3, 2);
    place('tileset1', '1_004', 7 * T, 11 * T, 3, 2);

    place('tileset1', '1_005', 16 * T, 7 * T, 3, 2);
    place('tileset1', '1_004', 19 * T, 7 * T, 3, 2);
    place('tileset1', '1_005', 16 * T, 11 * T, 3, 2);
    place('tileset1', '1_004', 19 * T, 11 * T, 3, 2);

    // ═══ STORAGE ════════════════════════════════════════

    place('tileset1', '1_000', 3 * T, 24 * T, 2, 2);
    place('tileset1', '1_042', 7 * T, 24 * T, 3, 2);
    place('tileset1', '1_003', 13 * T, 24 * T, 4, 2);
    place('tileset1', '1_042', 19 * T, 24 * T, 3, 2);
    place('tileset2', '2_040', 23 * T, 24 * T, 3, 2);

    blockTiles(blocked);
  }

  // ── Keyboard (WASD + Arrows) — controls USER only ─────

  private setupKeyboard(): void {
    if (!this.input.keyboard) return;

    // ALL keys with enableCapture=false so they still reach DOM inputs
    this.cursors = {
      up: this.input.keyboard.addKey('UP', false, false),
      down: this.input.keyboard.addKey('DOWN', false, false),
      left: this.input.keyboard.addKey('LEFT', false, false),
      right: this.input.keyboard.addKey('RIGHT', false, false),
      space: this.input.keyboard.addKey('SPACE', false, false),
      shift: this.input.keyboard.addKey('SHIFT', false, false),
    } as Phaser.Types.Input.Keyboard.CursorKeys;
    this.wasdKeys = {
      w: this.input.keyboard.addKey('W', false, false),
      a: this.input.keyboard.addKey('A', false, false),
      s: this.input.keyboard.addKey('S', false, false),
      d: this.input.keyboard.addKey('D', false, false),
    };
  }

  private handleKeyboardMovement(): void {
    const focused = document.activeElement?.tagName;
    if (focused === 'INPUT' || focused === 'TEXTAREA' || focused === 'SELECT') return;

    if (this.userSprite.getIsMoving()) return;

    let dx = 0;
    let dy = 0;

    const up = this.cursors?.up.isDown || this.wasdKeys?.['w'].isDown;
    const down = this.cursors?.down.isDown || this.wasdKeys?.['s'].isDown;
    const left = this.cursors?.left.isDown || this.wasdKeys?.['a'].isDown;
    const right = this.cursors?.right.isDown || this.wasdKeys?.['d'].isDown;

    if (up) dy = -1;
    else if (down) dy = 1;
    else if (left) dx = -1;
    else if (right) dx = 1;

    if (dx === 0 && dy === 0) return;

    const tx = this.userSprite.tileX + dx;
    const ty = this.userSprite.tileY + dy;
    if (!isWalkable(tx, ty)) return;

    this.userSprite.walkTo([{ x: tx, y: ty }]);
  }

  // ── Mouse Input — click moves USER ────────────────────

  private setupInput(): void {
    this.input.on('pointerdown', (pointer: Phaser.Input.Pointer) => {
      // Don't move user when clicking on an interactive object (NPC agents)
      const hitObjects = this.input.hitTestPointer(pointer);
      if (hitObjects.length > 0) return;

      const tx = Math.floor(pointer.worldX / T);
      const ty = Math.floor(pointer.worldY / T);

      eventBus.emit('tile-clicked', {
        tileX: tx,
        tileY: ty,
        worldX: pointer.worldX,
        worldY: pointer.worldY,
      });

      if (!isWalkable(tx, ty) || this.userSprite.getIsMoving()) return;

      const path = findPath(this.userSprite.tileX, this.userSprite.tileY, tx, ty);
      if (path && path.length > 0) {
        this.userSprite.walkTo(path);
      }
    });

    this.input.on('pointermove', (pointer: Phaser.Input.Pointer) => {
      const tx = Math.floor(pointer.worldX / T);
      const ty = Math.floor(pointer.worldY / T);

      if (isWalkable(tx, ty)) {
        this.hoverHighlight.setPosition(tx * T, ty * T).setVisible(true);
      } else {
        this.hoverHighlight.setVisible(false);
      }
    });
  }

  // ── Agent Events (NPCs — NOT controlled by user) ─────

  private setupAgentEvents(): void {
    eventBus.on('agent-created', (data) => {
      this.commandQueue.push({ type: 'SPAWN_AGENT', ...data });
    });

    eventBus.on('agent-deleted', (data) => {
      this.commandQueue.push({ type: 'REMOVE_AGENT', agentId: data.agentId });
    });

    eventBus.on('agent-state-changed', (data) => {
      this.commandQueue.push({ type: 'SET_AGENT_STATE', agentId: data.agentId, state: data.state });
    });

    eventBus.on('move-agent', (data) => {
      this.commandQueue.push({ type: 'MOVE_AGENT', agentId: data.agentId, tileX: data.tileX, tileY: data.tileY });
    });
  }

  // ── Command Queue ─────────────────────────────────────

  private processCommandQueue(): void {
    const commands = this.commandQueue.splice(0);
    for (const cmd of commands) {
      this.executeCommand(cmd);
    }
  }

  private executeCommand(cmd: GameCommand): void {
    switch (cmd.type) {
      case 'SPAWN_AGENT': {
        if (this.agentSprites.has(cmd.agentId)) break;
        const { x, y } = this.findFreeSpawn(cmd.tileX, cmd.tileY);
        const sprite = this.spawnAgent(cmd.agentId, cmd.name, x, y);
        this.agentSprites.set(cmd.agentId, sprite);
        break;
      }
      case 'REMOVE_AGENT': {
        const sprite = this.agentSprites.get(cmd.agentId);
        if (sprite) {
          this.despawnAgent(sprite);
          this.agentSprites.delete(cmd.agentId);
        }
        break;
      }
      case 'SET_AGENT_STATE': {
        const sprite = this.agentSprites.get(cmd.agentId);
        if (!sprite) break;
        const inView = this.cameras.main.worldView.contains(sprite.x, sprite.y);
        this.handleAgentStateChange(sprite, cmd.state, inView);
        break;
      }
      case 'MOVE_AGENT': {
        const sprite = this.agentSprites.get(cmd.agentId);
        if (!sprite || sprite.getIsMoving()) break;
        const path = findPath(sprite.tileX, sprite.tileY, cmd.tileX, cmd.tileY);
        if (path && path.length > 0) sprite.walkTo(path);
        break;
      }
    }
  }

  private handleAgentStateChange(sprite: AgentSprite, state: AgentState, updateVisuals = true): void {
    if (sprite.getIsMoving() && state.kind !== 'Idle') return;

    const zone = this.getZone(state);
    if (zone) {
      const atZone = sprite.tileX === zone.x && sprite.tileY === zone.y;
      if (!atZone && !sprite.getIsMoving()) {
        const path = findPath(sprite.tileX, sprite.tileY, zone.x, zone.y);
        if (path && path.length > 0) {
          sprite.walkToExternal(path, state);
          return;
        }
      }
    }

    if (updateVisuals) {
      sprite.setAgentState(state, false);
    }
  }

  private getZone(state: AgentState): { x: number; y: number } | null {
    switch (state.kind) {
      case 'Thinking':
        // Compaction: agent "archives" at the bookshelf
        if (state.taskSummary.includes('Organizing')) return ZONES['bookshelf'];
        return ZONES['desk'];
      case 'Coding':
        return ZONES['desk'];
      case 'Reading':
        return ZONES['bookshelf'];
      default:
        return null;
    }
  }

  // ── Agent Pool ────────────────────────────────────────

  private initAgentPool(): void {
    for (let i = 0; i < this.POOL_SIZE; i++) {
      const sprite = new AgentSprite(this, `__pool_${i}__`, '', 0, 0);
      sprite.reset();
      this.availableAgents.push(sprite);
    }
  }

  private spawnAgent(agentId: string, name: string, tileX: number, tileY: number): AgentSprite {
    const sprite = this.availableAgents.pop() ?? new AgentSprite(this, agentId, name, tileX, tileY);
    sprite.init(agentId, name, tileX, tileY);
    return sprite;
  }

  private despawnAgent(sprite: AgentSprite): void {
    sprite.reset();
    this.availableAgents.push(sprite);
  }

  // ── Spawn validation ───────────────────────────────────

  /** Find the nearest walkable tile not occupied by another agent. */
  private findFreeSpawn(tileX: number, tileY: number): { x: number; y: number } {
    if (this.isTileFree(tileX, tileY)) return { x: tileX, y: tileY };

    // BFS spiral outward to find the closest free tile
    const visited = new Set<string>();
    const queue: Array<{ x: number; y: number }> = [{ x: tileX, y: tileY }];
    visited.add(`${tileX},${tileY}`);

    while (queue.length > 0) {
      const pos = queue.shift()!;
      for (const [dx, dy] of [[0, -1], [1, 0], [0, 1], [-1, 0]]) {
        const nx = pos.x + dx;
        const ny = pos.y + dy;
        const key = `${nx},${ny}`;
        if (visited.has(key)) continue;
        visited.add(key);
        if (this.isTileFree(nx, ny)) return { x: nx, y: ny };
        queue.push({ x: nx, y: ny });
      }
    }

    // Fallback (should never happen in a valid map)
    return { x: tileX, y: tileY };
  }

  private isTileFree(x: number, y: number): boolean {
    if (!isWalkable(x, y)) return false;
    // Check no other agent is already on this tile
    for (const sprite of this.agentSprites.values()) {
      if (sprite.tileX === x && sprite.tileY === y) return false;
    }
    // Check user isn't on this tile
    if (this.userSprite.tileX === x && this.userSprite.tileY === y) return false;
    return true;
  }

  // ── Camera — follows USER ─────────────────────────────

  private setupCamera(): void {
    const cam = this.cameras.main;
    cam.setBounds(0, 0, MAP_COLS * T, MAP_ROWS * T);
    cam.setZoom(2);
    cam.roundPixels = true;
    cam.startFollow(this.userSprite.container, true, 0.1, 0.1);
  }
}
