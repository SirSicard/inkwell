#!/usr/bin/env python3
"""Regenerates the two-channel AMI meeting fixture from the corpus's individual headset files.

    INK_BENCH_DIR=<bench directory> python3 fixtures/ami/make_excerpt.py

Reads `$INK_BENCH_DIR/ami-ihm-full/IS1009a/IS1009a.Headset-{0..3}.wav` (AMI Meeting Corpus,
CC-BY-4.0; see fixtures/ATTRIBUTION.md), checks each file's SHA-256 against the manifest, and
writes the excerpt next to this script:

- `IS1009a-mic.wav`: headset 0 alone, the "you" side (the microphone).
- `IS1009a-far.wav`: headsets 1, 2 and 3 summed and scaled by a fixed gain, the "them" side (what
  the system audio tap would hear). Samples are rounded to the nearest integer and clipped to
  16 bits.

Both are 16 kHz mono 16-bit PCM. The script then checks the outputs' SHA-256 against the manifest,
so a regeneration that differs by one sample fails loudly. Standard library only.
"""

import hashlib
import json
import os
import sys
import wave
from array import array

HERE = os.path.dirname(os.path.abspath(__file__))
MANIFEST = os.path.join(HERE, "IS1009a.json")


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def read_range(path, start, end):
    with wave.open(path, "rb") as w:
        if (w.getnchannels(), w.getsampwidth(), w.getframerate()) != (1, 2, 16000):
            sys.exit(f"{os.path.basename(path)}: expected 16 kHz mono 16-bit")
        w.setpos(start)
        samples = array("h", w.readframes(end - start))
    if sys.byteorder != "little":
        samples.byteswap()
    if len(samples) != end - start:
        sys.exit(f"{os.path.basename(path)}: shorter than the range")
    return samples


def write(path, samples):
    out = array("h", samples)
    if sys.byteorder != "little":
        out.byteswap()
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(16000)
        w.writeframes(out.tobytes())


def main():
    with open(MANIFEST) as f:
        manifest = json.load(f)
    bench = os.environ.get("INK_BENCH_DIR")
    if not bench:
        sys.exit("set INK_BENCH_DIR to the bench directory")
    source_dir = os.path.join(bench, manifest["source"]["dir"])
    start, end = manifest["range"]["start_sample"], manifest["range"]["end_sample"]

    headsets = {}
    for name, digest in manifest["source"]["sha256"].items():
        path = os.path.join(source_dir, name)
        if sha256(path) != digest:
            sys.exit(f"{name}: SHA-256 does not match the manifest")
        headsets[name] = read_range(path, start, end)

    for channel in manifest["channels"]:
        gain = channel["gain"]
        sources = [headsets[name] for name in channel["sources"]]
        mixed = []
        for frame in zip(*sources):
            v = round(sum(frame) * gain)
            mixed.append(max(-32768, min(32767, v)))
        path = os.path.join(HERE, channel["file"])
        write(path, mixed)
        digest = sha256(path)
        if digest != channel["sha256"]:
            sys.exit(f"{channel['file']}: wrote SHA-256 {digest}, the manifest says {channel['sha256']}")
        print(f"{channel['file']}: ok")


if __name__ == "__main__":
    main()
