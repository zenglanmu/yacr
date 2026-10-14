// Counts native rendered frames only; polling and React ticks are not frames.
export function createFpsSampler({ windowMs = 2000, minimumMs = 1000, staleMs = 5000 } = {}) {
  let samples = [];
  let lastAdvance = null;
  const reset = () => { samples = []; lastAdvance = null; };
  return {
    reset,
    sample(frameCount, now, available = true) {
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
      if (frameCount > samples.at(-1).count) lastAdvance = now;
      samples.push({ time: now, count: frameCount });
      // Keep one boundary sample so low-frequency polling still spans the window.
      while (samples.length > 2 && samples[1].time <= now - windowMs) samples.shift();
      const oldest = samples[0];
      const elapsed = now - oldest.time;
      if (elapsed < minimumMs || now - lastAdvance >= windowMs) return null;
      const delta = frameCount - oldest.count;
      return delta > 0 ? delta * 1000 / elapsed : null;
    },
  };
}
