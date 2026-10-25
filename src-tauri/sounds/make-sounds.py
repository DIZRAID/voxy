#!/usr/bin/env python3
"""Synthesize Voxy's Windows cue sounds (start, stop, error).

    python3 src-tauri/sounds/make-sounds.py src-tauri/sounds

On macOS Voxy plays the system sounds Pop (start), Bottle (stop) and Basso
(error). Apple's sound files may only be used on Apple hardware, so they
cannot ship in the Windows build. These are sound-alikes built from scratch
from parameters measured on the macOS sounds: the loudness envelope (5 ms
steps), the frequencies of the partials over time and the balance across
third-octave bands. No Apple audio is copied into this script or its output.

- start.wav (like Pop): six quick, soft "pok"s at 0, 68, 90, 135, 180 and
  203 ms, each two damped modes near 500 and 670 Hz plus a low knock;
  almost nothing above 1 kHz.
- stop.wav (like Bottle): three blown, nearly pure tones, 494 Hz, then a
  rising 235 -> 278 Hz, then a long 186 Hz, with a breath of air at
  8-13 kHz.
- error.wav (like Basso): a low "bonk" of two harmonic tones (~83 and
  ~215 Hz) whose upper partials die first, so it darkens within ~100 ms.

platform.rs embeds the WAV files (include_bytes!) and plays them with
PlaySoundW(SND_MEMORY). Standard library only and deterministic (fixed
noise seeds), so the output is reproducible: 16-bit PCM, mono, 44.1 kHz,
peaks at -9 dBFS.
"""
import math
import random
import struct
import sys
import wave

RATE = 44100


def ms(t):
    return int(RATE * t / 1000)


def env_from_points(points, n):
    """[(ms, dB)] -> per-sample linear gain, interpolated linearly in dB."""
    out = [0.0] * n
    j = 0
    for i in range(n):
        t = i * 1000 / RATE
        while j < len(points) - 2 and t > points[j + 1][0]:
            j += 1
        (t0, d0), (t1, d1) = points[j], points[j + 1]
        if t <= t0:
            d = d0
        elif t >= t1:
            d = d1
        else:
            d = d0 + (d1 - d0) * (t - t0) / (t1 - t0)
        out[i] = 10 ** (d / 20) if d > -120 else 0.0
    return out


def match_envelope(x, target_db_5ms, max_boost_db=24.0):
    """Scale x so that its RMS in 5 ms blocks follows target_db_5ms (dB re max)."""
    step = ms(5)
    n = len(x)
    blocks = [x[i:i + step] for i in range(0, n, step)]
    raw = [math.sqrt(sum(v * v for v in b) / len(b)) if b else 0 for b in blocks]
    top = max(raw) or 1.0
    gains = []
    for k, r in enumerate(raw):
        target = target_db_5ms[k] if k < len(target_db_5ms) else -120
        now = 20 * math.log10(max(r / top, 1e-9))
        gains.append(min(target - now, max_boost_db) if target > -90 else -120)
    out = []
    for i in range(n):
        p = (i - step / 2) / step
        k = max(0, min(len(gains) - 1, int(math.floor(p))))
        f = min(1.0, max(0.0, p - k))
        g = gains[k] + (gains[min(k + 1, len(gains) - 1)] - gains[k]) * f
        out.append(x[i] * (10 ** (g / 20) if g > -100 else 0.0))
    return out


def lowpass1(x, fc):
    a = math.exp(-2 * math.pi * fc / RATE)
    y = 0.0
    out = []
    for v in x:
        y = (1 - a) * v + a * y
        out.append(y)
    return out


def highpass1(x, fc):
    return [a - b for a, b in zip(x, lowpass1(x, fc))]


def normalize(x, peak_db):
    top = max(abs(v) for v in x) or 1.0
    g = 10 ** (peak_db / 20) / top
    return [v * g for v in x]


def fade_out(x, fade_ms=6):
    f = ms(fade_ms)
    for i in range(min(f, len(x))):
        x[-1 - i] *= i / f
    return x


# Start (like Pop): (start ms, level dB, weight of the ~500 Hz mode, weight of
# the ~670 Hz mode) for each "pok".
POP_BURSTS = [
    (1, 0.0, 0.85, 1.0), (68, -1.7, 1.0, 0.45), (90, -10.0, 1.0, 0.3),
    (135, -15.7, 0.3, 1.0), (180, -26.8, 0.35, 1.0), (203, -31.6, 0.6, 1.0),
]


def pop():
    n = ms(235)
    x = [0.0] * n
    rng = random.Random(5)
    for start, level, w_lo, w_hi in POP_BURSTS:
        a = 10 ** (level / 20)
        s0 = ms(start)
        f_lo = 520 * (1 + rng.uniform(-0.015, 0.015))
        f_hi = 700 * (1 + rng.uniform(-0.015, 0.015))
        for i in range(s0, min(n, s0 + ms(60))):
            t = (i - s0) / RATE
            env = min(1.0, t / 0.004) * math.exp(-t / 0.0075)
            bend = 1 - 0.05 * (1 - math.exp(-t / 0.012))  # pitch settles ~5 %
            knock = 0.45 * math.sin(2 * math.pi * 330 * t) * math.exp(-t / 0.009)
            x[i] += a * env * (
                w_lo * math.sin(2 * math.pi * f_lo * bend * t)
                + 1.3 * w_hi * math.sin(2 * math.pi * f_hi * bend * t)
                + knock
            )
    return fade_out(normalize(lowpass1(x, 2200), -9.0))


# Stop (like Bottle): loudness contour of the three blown notes, (ms, dB).
BOTTLE_ENV = [
    (0, -70), (3, -60), (8, -11), (12, -6.3), (20, -13.9), (35, -21.7), (55, -42), (60, -60),
    (62, -60), (66, -9), (72, -5.3), (80, -8.9), (100, -14.6), (115, -18.9), (135, -21.7), (139, -40),
    (140, -40), (145, -1.2), (150, 0.0), (165, -4.8), (180, -8.2), (200, -11.1), (215, -11.7),
    (225, -14.0), (250, -19.4), (275, -26.8), (290, -32.9), (305, -38.9), (330, -46), (420, -54),
    (480, -70),
]


def bottle():
    n = ms(480)
    rng = random.Random(9)

    def freq(t_ms):
        if t_ms < 61:
            return 494.0
        if t_ms < 139:  # the middle note rises
            u = (t_ms - 61) / 78
            return 235 + 43 * u * u
        return 185.5 * (1 + 0.004 * math.sin(2 * math.pi * 5.5 * (t_ms - 139) / 1000))

    ph = 0.0
    x = []
    for i in range(n):
        t_ms = i * 1000 / RATE
        if abs(t_ms - 61) < 0.03 or abs(t_ms - 139.5) < 0.03:
            ph = 0.0  # each note is blown afresh (amplitude is ~0 here)
        ph += 2 * math.pi * freq(t_ms) / RATE
        x.append(math.sin(ph) + 0.03 * math.sin(2 * ph))
    env = env_from_points(BOTTLE_ENV, n)
    tone = highpass1(highpass1([a * b for a, b in zip(x, env)], 140), 140)
    air = highpass1(highpass1([rng.uniform(-1, 1) for _ in range(n)], 6000), 6000)
    air = [a * e * 0.05 for a, e in zip(air, env)]
    return fade_out(normalize([a + b for a, b in zip(tone, air)], -9.0), 30)


# Error (like Basso): (Hz, level, attack ms, decay ms) for each partial of the
# two harmonic tones, and the loudness in 5 ms steps (dB re max).
BASSO_PARTIALS = [
    (215, 0.15, 40, 200), (430, 0.45, 45, 50), (648, 1.0, 3, 32), (866, 0.75, 3, 24),
    (83, 0.025, 40, 200), (164, 0.15, 30, 90), (250, 0.40, 3, 35), (333, 0.33, 3, 28),
]
BASSO_ENV5 = [
    -1, 0, -2, -1, -2, -1, -2, -1, -1, -2, -4, -6, -8, -8, -8, -8, -11, -15, -16, -16, -18, -19,
    -18, -16, -16, -17, -18, -19, -22, -21, -22, -24, -33, -42, -50, -57, -66, -72, -90,
]


def basso():
    n = ms(195)
    x = [0.0] * n
    for i in range(n):
        t = i / RATE
        # the note rises ~5 % over 160 ms: phase of f * (1 + 0.05 * min(t, 0.16) / 0.16)
        bent = t + 0.05 * min(t, 0.16) ** 2 / (2 * 0.16) + max(0.0, t - 0.16) * 0.05
        s = 0.0
        for f, a, attack, decay in BASSO_PARTIALS:
            s += a * min(1.0, t * 1000 / attack) * math.exp(-t * 1000 / decay) * math.sin(2 * math.pi * f * bent)
        x[i] = s
    return fade_out(normalize(match_envelope(x, BASSO_ENV5), -9.0))


def write(path, samples):
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(b"".join(
            struct.pack("<h", max(-32767, min(32767, int(round(v * 32767))))) for v in samples))


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "."
    for name, make in [("start", pop), ("stop", bottle), ("error", basso)]:
        write(f"{out}/{name}.wav", make())


if __name__ == "__main__":
    main()
