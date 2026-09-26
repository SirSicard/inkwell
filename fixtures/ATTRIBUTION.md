# Fixture attribution

`fixtures/` holds only synthetic audio and short excerpts of public datasets whose licence allows
redistribution. Never add recordings of real meetings or real people outside those datasets.

Every excerpt gets a row: the file, the dataset, the source item and time range, and the licence.

| File | Dataset | Source item and range | Licence |
|---|---|---|---|
| `ami/IS1009a-mic.wav` | AMI Meeting Corpus | meeting IS1009a, individual headset 0, 792.5–822.5 s | CC-BY-4.0 |
| `ami/IS1009a-far.wav` | AMI Meeting Corpus | meeting IS1009a, individual headsets 1–3 summed and scaled by 0.88, 792.5–822.5 s | CC-BY-4.0 |

The AMI excerpts are modified: cut to 30 s, and the far side is a mix of three headsets. The
manifest `ami/IS1009a.json` records the source files' SHA-256, the range and the mix, and
`ami/make_excerpt.py` regenerates both files byte for byte from the corpus.

## Datasets

- **AMI Meeting Corpus** (University of Edinburgh and partners), CC-BY-4.0.
  Cite: Carletta, J. et al., "The AMI Meeting Corpus: A Pre-announcement", MLMI 2005.
- **FLEURS** (Google), CC-BY-4.0.
  Cite: Conneau, A. et al., "FLEURS: Few-shot Learning Evaluation of Universal Representations of
  Speech", IEEE SLT 2022.
