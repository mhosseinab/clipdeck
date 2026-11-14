let ctx: AudioContext | null = null;

function ac(): AudioContext {
  ctx ??= new AudioContext();
  return ctx;
}

/** Original synthesized sounds — no sampled assets. */
export function playSound(kind: "copy" | "paste"): void {
  try {
    const a = ac();
    const t = a.currentTime;
    if (kind === "copy") {
      const notes = [660, 990];
      notes.forEach((f, i) => {
        const o = a.createOscillator();
        const g = a.createGain();
        o.type = "sine";
        o.frequency.value = f;
        g.gain.setValueAtTime(0.0001, t + i * 0.055);
        g.gain.exponentialRampToValueAtTime(0.12, t + i * 0.055 + 0.012);
        g.gain.exponentialRampToValueAtTime(0.0001, t + i * 0.055 + 0.11);
        o.connect(g).connect(a.destination);
        o.start(t + i * 0.055);
        o.stop(t + i * 0.055 + 0.13);
      });
    } else {
      const buf = a.createBuffer(1, 1200, a.sampleRate);
      const data = buf.getChannelData(0);
      for (let i = 0; i < data.length; i++) {
        data[i] = (Math.random() * 2 - 1) * (1 - i / data.length) ** 2;
      }
      const src = a.createBufferSource();
      const filter = a.createBiquadFilter();
      const g = a.createGain();
      filter.type = "lowpass";
      filter.frequency.value = 1600;
      g.gain.value = 0.18;
      src.buffer = buf;
      src.connect(filter).connect(g).connect(a.destination);
      src.start(t);
    }
  } catch {
    /* audio unavailable */
  }
}
