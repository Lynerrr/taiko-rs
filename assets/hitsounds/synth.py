"""Synthesizes taiko.rs's built-in hitsounds (don.wav, kat.wav, finish.wav, combobreak.ogg) from
scratch, so none of them comes from anyone else's skin or game.

    python3 assets/hitsounds/synth.py      (needs numpy, and ffmpeg for the .ogg)

A don is the drum's face: a membrane whose pitch drops as it settles, with its second mode and a
soft attack. A kat is the rim: a few inharmonic wooden partials and a click. A finisher is a
bigger, deeper don with a metallic shimmer. The combo break is a falling, wobbling "bwomp".
"""

import pathlib
import subprocess
import wave

import numpy as np

HERE = pathlib.Path(__file__).resolve().parent
SR = 44100
rng = np.random.default_rng(7)


def time(seconds):
    return np.arange(int(SR * seconds)) / SR


def phase(freq):
    """The running phase of a tone whose frequency (Hz, per sample) changes."""
    return 2 * np.pi * np.cumsum(freq) / SR


def lowpass(x, hz):
    a = np.exp(-2 * np.pi * hz / SR)
    out = np.empty_like(x)
    acc = 0.0
    for i, v in enumerate(x):
        acc = (1 - a) * v + a * acc
        out[i] = acc
    return out


def highpass(x, hz):
    return x - lowpass(x, hz)


def finish_up(x, rms_db):
    """Faded out at the end and brought to `rms_db` loudness (as loud as the sounds it replaced),
    peaks kept under -0.3 dB."""
    x = np.tanh(0.8 * x / np.max(np.abs(x)))
    fade = min(len(x), int(0.012 * SR))
    x[-fade:] *= np.linspace(1, 0, fade)
    x *= 10 ** (rms_db / 20) / np.sqrt(np.mean(x**2))
    return x * min(1.0, 10 ** (-0.3 / 20) / np.max(np.abs(x)))


def membrane(t, low, high, drop, decay):
    """A drum face: pitch from `high` down to `low` (Hz) over `drop` s, ringing for `decay` s."""
    f = low + (high - low) * np.exp(-t / drop)
    ph = phase(f)
    body = np.sin(ph) * np.exp(-t / decay)
    # The membrane's second mode (about 1.59x), shorter-lived.
    body += 0.32 * np.sin(1.59 * ph + 0.4) * np.exp(-t / (decay * 0.45))
    attack = lowpass(rng.standard_normal(len(t)), 1800) * np.exp(-t / 0.005) * 2.2
    return body + attack


def don():
    t = time(0.32)
    return finish_up(membrane(t, 58, 165, 0.032, 0.085), -19)


def kat():
    t = time(0.26)
    x = np.zeros_like(t)
    for hz, amp, decay in [(640, 1.0, 0.060), (1170, 0.75, 0.045), (1890, 0.5, 0.030), (3010, 0.32, 0.018), (4420, 0.18, 0.010)]:
        x += amp * np.sin(2 * np.pi * hz * t + rng.uniform(0, 6.28)) * np.exp(-t / decay)
    click = highpass(rng.standard_normal(len(t)), 2500) * np.exp(-t / 0.0035) * 1.6
    return finish_up(x + click, -21)


def finish():
    t = time(0.75)
    x = membrane(t, 48, 150, 0.045, 0.2) * 1.1
    shimmer = highpass(highpass(rng.standard_normal(len(t)), 5000), 5000) * np.exp(-t / 0.09) * 0.22
    bell = sum(a * np.sin(2 * np.pi * hz * t) * np.exp(-t / d) for hz, a, d in [(830, 0.22, 0.32), (1297, 0.16, 0.26), (2071, 0.1, 0.18)])
    return finish_up(x + shimmer + bell, -16)


def combobreak():
    t = time(0.62)
    f = (115 + 470 * np.exp(-t / 0.22)) * (1 + 0.035 * np.sin(2 * np.pi * 17 * t))
    ph = phase(f)
    tone = np.sin(ph) + 0.35 * np.sin(2 * ph) + 0.18 * np.sin(3 * ph) + 0.08 * np.sin(5 * ph)
    env = (1 - np.exp(-t / 0.004)) * np.exp(-t / 0.28)
    return finish_up(lowpass(tone * env, 3200), -11)


def write_wav(path, x):
    pcm = (np.clip(x, -1, 1) * 32767).astype("<i2")
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(pcm.tobytes())


def main():
    write_wav(HERE / "don.wav", don())
    write_wav(HERE / "kat.wav", kat())
    write_wav(HERE / "finish.wav", finish())
    tmp = HERE / "combobreak.tmp.wav"
    write_wav(tmp, combobreak())
    subprocess.run(["ffmpeg", "-nostdin", "-v", "error", "-y", "-i", str(tmp), "-c:a", "libvorbis", "-q:a", "5", str(HERE / "combobreak.ogg")], check=True)
    tmp.unlink()


if __name__ == "__main__":
    main()
