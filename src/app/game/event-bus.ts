import type { GameEventMap } from '../models/game-events.model';

type Callback<T> = (data: T) => void;

class EventBus<T extends { [K in keyof T]: T[K] }> {
  private listeners = new Map<keyof T, Set<Callback<never>>>();

  on<K extends keyof T>(event: K, callback: Callback<T[K]>): void {
    if (!this.listeners.has(event)) {
      this.listeners.set(event, new Set());
    }
    this.listeners.get(event)!.add(callback as Callback<never>);
  }

  off<K extends keyof T>(event: K, callback: Callback<T[K]>): void {
    this.listeners.get(event)?.delete(callback as Callback<never>);
  }

  emit<K extends keyof T>(event: K, data: T[K]): void {
    const callbacks = this.listeners.get(event);
    if (callbacks) {
      callbacks.forEach((cb) => (cb as Callback<T[K]>)(data));
    }
  }

  removeAllListeners(): void {
    this.listeners.clear();
  }
}

export const eventBus = new EventBus<GameEventMap>();
