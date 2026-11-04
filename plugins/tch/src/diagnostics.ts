// Aggregate diagnostic counters shared by the decoders.

export function bump(counts: Map<string, number>, key: string): void {
  counts.set(key, (counts.get(key) ?? 0) + 1);
}

export function add(counts: Map<string, number>, key: string, value: number): void {
  counts.set(key, (counts.get(key) ?? 0) + value);
}
