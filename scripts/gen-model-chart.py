#!/usr/bin/env python3
"""Generate the model comparison chart used in the README.

Two files rather than one: an SVG referenced by <img> does not follow GitHub's
theme toggle, so the README pairs a light and a dark copy inside <picture>.

Numbers come from docs/qwen3-spike-2026-07-31.md, measured with
src-tauri/examples/ab_models.rs on eight recordings of one voice. Re-run that
and regenerate rather than editing the SVGs by hand.
"""

# (label, word error rate %, seconds to transcribe 57s of audio, disk)
MODELS = [
    ("Qwen3 ASR",     5.6,  9.2, "940 MB"),
    ("Parakeet V2",   8.0,  3.6, "670 MB"),
    ("SenseVoice",    9.3,  1.7, "240 MB"),
    ("Whisper Turbo", 9.3, 19.8, "800 MB"),
    ("Parakeet V3",  10.5,  3.7, "670 MB"),
]

THEMES = {
    "light": dict(fg="#1f2328", muted="#59636e", bar="#d1d9e0", fill="#0969da",
                  accent="#1a7f37", grid="#d1d9e0"),
    "dark":  dict(fg="#e6edf3", muted="#9198a1", bar="#2a313c", fill="#4493f8",
                  accent="#3fb950", grid="#2a313c"),
}

W, ROW_H, TOP = 920, 46, 76
LABEL_W = 130
P1_X, P2_X, PANEL_W = 140, 530, 260
FONT = '-apple-system,BlinkMacSystemFont,"Segoe UI",Helvetica,Arial,sans-serif'

WER_MAX = 12.0
SEC_MAX = 21.0
BEST_WER = min(m[1] for m in MODELS)
BEST_SEC = min(m[2] for m in MODELS)


def esc(s):
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def svg(theme):
    c = THEMES[theme]
    h = TOP + len(MODELS) * ROW_H + 34
    o = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{h}" '
        f'viewBox="0 0 {W} {h}" font-family=\'{FONT}\' role="img" '
        f'aria-label="Model comparison: word error rate and transcription time">'
    ]

    # Panel headings. "lower is better" stated outright: a reader should not have
    # to infer the direction of a bar chart to know which model is good.
    o.append(f'<text x="{P1_X}" y="30" font-size="14" font-weight="600" fill="{c["fg"]}">Accuracy</text>')
    o.append(f'<text x="{P1_X}" y="50" font-size="12" fill="{c["muted"]}">word error rate, lower is better</text>')
    o.append(f'<text x="{P2_X}" y="30" font-size="14" font-weight="600" fill="{c["fg"]}">Speed</text>')
    o.append(f'<text x="{P2_X}" y="50" font-size="12" fill="{c["muted"]}">seconds to transcribe 57s of audio</text>')

    for i, (name, wer, sec, disk) in enumerate(MODELS):
        y = TOP + i * ROW_H
        bar_y, bar_h = y + 6, 14

        o.append(f'<text x="{LABEL_W}" y="{y + 17}" text-anchor="end" font-size="13" '
                 f'font-weight="500" fill="{c["fg"]}">{esc(name)}</text>')
        o.append(f'<text x="{LABEL_W}" y="{y + 32}" text-anchor="end" font-size="11" '
                 f'fill="{c["muted"]}">{esc(disk)}</text>')

        for x0, value, vmax, best, label in (
            (P1_X, wer, WER_MAX, BEST_WER, f"{wer}%"),
            (P2_X, sec, SEC_MAX, BEST_SEC, f"{sec}s"),
        ):
            fill = c["accent"] if value == best else c["fill"]
            o.append(f'<rect x="{x0}" y="{bar_y}" width="{PANEL_W}" height="{bar_h}" rx="7" fill="{c["bar"]}"/>')
            o.append(f'<rect x="{x0}" y="{bar_y}" width="{round(PANEL_W * value / vmax, 1)}" '
                     f'height="{bar_h}" rx="7" fill="{fill}"/>')
            o.append(f'<text x="{x0 + PANEL_W + 10}" y="{bar_y + 11}" font-size="12" '
                     f'font-weight="600" fill="{c["fg"]}">{label}</text>')

    o.append(f'<text x="{P1_X}" y="{h - 10}" font-size="11" fill="{c["muted"]}">'
             'Eight recordings of one voice. Directional, not a benchmark. '
             'Measure your own with cargo run --release --example ab_models.</text>')
    o.append("</svg>")
    return "\n".join(o)


if __name__ == "__main__":
    import pathlib
    out = pathlib.Path(__file__).resolve().parent.parent / "docs" / "media"
    for theme in THEMES:
        p = out / f"models-{theme}.svg"
        p.write_text(svg(theme))
        print(f"wrote {p}")
