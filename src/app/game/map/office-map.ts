export const TILE_SIZE = 32;
export const MAP_COLS = 26;
export const MAP_ROWS = 26;

// ── Room Definitions ─────────────────────────────────────
export interface RoomDef {
  name: string;
  x: number;
  y: number;
  w: number;
  h: number;
  floorColor: number;
}

// Floor renders right up against the walls (no visual gap)
export const ROOMS: RoomDef[] = [
  { name: 'main', x: 1, y: 1, w: 24, h: 14, floorColor: 0x353545 },
  { name: 'storage', x: 1, y: 16, w: 24, h: 9, floorColor: 0x333338 },
];

// ── Door Openings (walkable tiles through the inner wall) ─
export const DOORS: Array<{ x: number; y: number }> = [
  // Passage through inner wall (rows 15-17) at cols 12-13
  { x: 12, y: 15 },
  { x: 13, y: 15 },
  { x: 12, y: 16 },
  { x: 13, y: 16 },
  { x: 12, y: 17 },
  { x: 13, y: 17 },
];

// ── Agent State Zones (tile coordinates) ─────────────────
export const ZONES: Record<string, { x: number; y: number }> = {
  desk: { x: 4, y: 7 },
  bookshelf: { x: 20, y: 19 },
  whiteboard: { x: 12, y: 19 },
  coffee: { x: 5, y: 19 },
};

// ── Walkability Grid ─────────────────────────────────────
// 1 = walkable floor, 0 = wall / blocked
const grid: number[][] = [];

(function initGrid() {
  for (let r = 0; r < MAP_ROWS; r++) {
    grid[r] = new Array(MAP_COLS).fill(0);
  }
  for (const room of ROOMS) {
    for (let r = room.y; r < room.y + room.h; r++) {
      for (let c = room.x; c < room.x + room.w; c++) {
        grid[r][c] = 1;
      }
    }
  }
  for (const d of DOORS) {
    grid[d.y][d.x] = 1;
  }

  // Block tiles visually covered by wall sprites (96px = 3 tiles)
  // North wall covers rows 0-2, row 3 is baseboard → block rows 1-3
  for (let c = 1; c <= 24; c++) {
    grid[1][c] = 0;
    grid[2][c] = 0;
    grid[3][c] = 0;
  }
  // Inner wall at row 15 covers rows 15-17 → block 16-17 (except door cols)
  for (let c = 1; c <= 24; c++) {
    if (c === 12 || c === 13) continue;
    grid[16][c] = 0;
    grid[17][c] = 0;
  }
})();

export function isWalkable(x: number, y: number): boolean {
  if (y < 0 || y >= MAP_ROWS || x < 0 || x >= MAP_COLS) return false;
  return grid[y][x] === 1;
}

export function blockTiles(tiles: Array<{ x: number; y: number }>): void {
  for (const t of tiles) {
    if (t.y >= 0 && t.y < MAP_ROWS && t.x >= 0 && t.x < MAP_COLS) {
      grid[t.y][t.x] = 0;
    }
  }
}
