// Counts native rendered frames only; polling and React ticks are not frames.

/** The sampler's window/minimum/staleness knobs, in milliseconds. */
export interface FpsSamplerOptions {
  readonly windowMs?: number;
  readonly minimumMs?: number;
  readonly staleMs?: number;
}

/** One `(time, frameCount)` observation. */
interface FpsSample {
  time: number;
  count: number;
}

/** The sampler the app polls with `cad_status.frameCount`. */
export interface FpsSampler {
  reset(): void;
  sample(frameCount: number, now: number, available?: boolean): number | null;
}

export function createFpsSampler({
  windowMs = 2000,
  minimumMs = 1000,
  staleMs = 5000,
}: FpsSamplerOptions = {}): FpsSampler {
  let samples: FpsSample[] = [];
  let lastAdvance: number | null = null;
  const reset = (): void => { samples = []; lastAdvance = null; };
  return {
    reset,
    sample(frameCount: number, now: number, available = true): number | null {
      if (!available || !Number.isSafeInteger(frameCount) || frameCount < 0 || !Number.isFinite(now)) {
        reset();
        return null;
      }
      const previous = samples.at(-1);
      if (previous && (now <= previous.time || frameCount < previous.count || now - previous.time > staleMs)) reset();
      if (!samples.length) {
        samples.push({ time: now, count: frameCount });
        lastAdvance = now;
        return null;
      }
      const newest = samples.at(-1);
      if (newest && frameCount > newest.count) lastAdvance = now;
      samples.push({ time: now, count: frameCount });
      // Keep one boundary sample so low-frequency polling still spans the window.
      while (samples.length > 2 && samples[1].time <= now - windowMs) samples.shift();
      const oldest = samples[0];
      const elapsed = now - oldest.time;
      // `?? 0` mirrors JS number coercion of the `null` it can never be here.
      if (elapsed < minimumMs || now - (lastAdvance ?? 0) >= windowMs) return null;
      const delta = frameCount - oldest.count;
      return delta > 0 ? delta * 1000 / elapsed : null;
    },
  };
}
