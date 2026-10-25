#!/usr/bin/env python3
"""Synthesize Voxy's Windows cue sounds (start, stop, error).

    python3 src-tauri/sounds/make-sounds.py src-tauri/sounds

On macOS Voxy plays the system sounds Pop (start), Bottle (stop) and Basso
(error). Apple's sound files may only be used on Apple hardware, so they
cannot ship in the Windows build; these are look-alikes made from scratch,
close in character: a short pop with a quick pitch drop, a round hollow
bottle tone and a low buzzy bass. platform.rs embeds the WAV files
(include_bytes!) and plays them with PlaySoundW(SND_MEMORY).

Standard library only and deterministic (fixed noise seed), so the output is
reproducible. Format: 16-bit PCM, mono, 44.1 kHz; peaks stay well below
full scale so the cues are no louder than Windows' own notification sounds.
"""
import math
import random
import struct
import sys
import wave

RATE = 44100


def render(duration, sample):
    """sample(t) -> float in [-1, 1]; a 6 ms fade-out avoids an end click."""
    n = int(RATE * duration)
    fade = int(RATE * 0.006)
    out = []
    for i in range(n):
        v = sample(i / RATE)
        if i > n - fade:
            v *= (n - i) / fade
        out.append(v)
    return out


def envelope(t, attack, decay):
    return min(1.0, t / attack) * math.exp(-t / decay)


def normalize(samples, peak):
    top = max(abs(v) for v in samples) or 1.0
    return [v * peak / top for v in samples]


def swept(freq_at):
    """Phase accumulator for a sine whose frequency changes over time."""
    state = {"phase": 0.0, "t": 0.0}

    def phase(t):
        dt = t - state["t"]
        state["phase"] += 2 * math.pi * freq_at(t) * dt
        state["t"] = t
        return state["phase"]

    return phase


def pop():
    # Start: a short bubbly "pop", pitch falling fast from ~1.1 kHz to ~560 Hz.
    phase = swept(lambda t: 560 + 540 * math.exp(-t / 0.018))

    def s(t):
        p = phase(t)
        return (math.sin(p) + 0.18 * math.sin(2 * p)) * envelope(t, 0.002, 0.030)

    return normalize(render(0.13, s), 0.55)


def bottle():
    # Stop: a round, hollow tone (like blowing across a bottle): ~620 Hz with
    # a slight upward glide at the start, weak harmonics and a soft breath.
    phase = swept(lambda t: 620 - 60 * math.exp(-t / 0.012))
    rng = random.Random(7)
    low = {"y": 0.0}

    def s(t):
        p = phase(t)
        tone = math.sin(p) + 0.12 * math.sin(2 * p) + 0.05 * math.sin(3 * p)
        low["y"] += 0.15 * (rng.uniform(-1, 1) - low["y"])  # one-pole low-pass
        breath = low["y"] * math.exp(-t / 0.010) * 0.8
        return tone * envelope(t, 0.004, 0.060) + breath

    return normalize(render(0.28, s), 0.5)


def basso():
    # Error: a low, buzzy bass note (~98 Hz) with harmonics up to ~800 Hz,
    # so it is still audible on small laptop speakers.
    f0 = 98.0

    def s(t):
        p = 2 * math.pi * f0 * t
        tone = sum(math.sin(k * p * (1 + 0.002 * k)) / k ** 1.3 for k in range(1, 9))
        return tone * envelope(t, 0.006, 0.11)

    return normalize(render(0.45, s), 0.6)


def write(path, samples):
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(b"".join(struct.pack("<h", int(round(v * 32767))) for v in samples))


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "."
    for name, make in [("start", pop), ("stop", bottle), ("error", basso)]:
        write(f"{out}/{name}.wav", make())


if __name__ == "__main__":
    main()
