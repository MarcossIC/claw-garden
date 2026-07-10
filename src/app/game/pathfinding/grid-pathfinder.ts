import { isWalkable, MAP_COLS, MAP_ROWS } from '../map/office-map';

interface Node {
  x: number;
  y: number;
  g: number;
  h: number;
  f: number;
  parent: Node | null;
}

function heuristic(ax: number, ay: number, bx: number, by: number): number {
  return Math.abs(ax - bx) + Math.abs(ay - by);
}

const DIRECTIONS = [
  { x: 0, y: -1 },
  { x: 1, y: 0 },
  { x: 0, y: 1 },
  { x: -1, y: 0 },
];

export function findPath(
  startX: number,
  startY: number,
  endX: number,
  endY: number,
): Array<{ x: number; y: number }> | null {
  if (!isWalkable(endX, endY)) return null;
  if (startX === endX && startY === endY) return [];

  const open: Node[] = [];
  const closed = new Set<string>();
  const key = (x: number, y: number) => `${x},${y}`;

  const startNode: Node = {
    x: startX,
    y: startY,
    g: 0,
    h: heuristic(startX, startY, endX, endY),
    f: heuristic(startX, startY, endX, endY),
    parent: null,
  };
  open.push(startNode);

  while (open.length > 0) {
    // Find node with lowest f
    let lowestIdx = 0;
    for (let i = 1; i < open.length; i++) {
      if (open[i].f < open[lowestIdx].f) lowestIdx = i;
    }
    const current = open.splice(lowestIdx, 1)[0];

    if (current.x === endX && current.y === endY) {
      const path: Array<{ x: number; y: number }> = [];
      let node: Node | null = current;
      while (node) {
        path.unshift({ x: node.x, y: node.y });
        node = node.parent;
      }
      return path;
    }

    closed.add(key(current.x, current.y));

    for (const dir of DIRECTIONS) {
      const nx = current.x + dir.x;
      const ny = current.y + dir.y;

      if (nx < 0 || nx >= MAP_COLS || ny < 0 || ny >= MAP_ROWS) continue;
      if (!isWalkable(nx, ny)) continue;
      if (closed.has(key(nx, ny))) continue;

      const g = current.g + 1;
      const h = heuristic(nx, ny, endX, endY);
      const existing = open.find((n) => n.x === nx && n.y === ny);

      if (existing) {
        if (g < existing.g) {
          existing.g = g;
          existing.f = g + h;
          existing.parent = current;
        }
      } else {
        open.push({ x: nx, y: ny, g, h, f: g + h, parent: current });
      }
    }
  }

  return null;
}
