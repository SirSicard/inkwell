# Export (TXT, SRT, JSON, CSV) - Research

## Format Specs

### TXT
Model + duration + date header, then text. Batch: concatenate with `---` separators.

### SRT
```
1
00:00:00,000 --> 00:00:04,500
First subtitle block text
```
- Comma for milliseconds (not period)
- Max ~42 chars wide, 2 lines per block
- Timestamps estimated via sentence-proportional distribution (no word-level timing available)

### CSV (RFC 4180)
Use `csv` crate. Headers: id, text, raw_text, style, model, audio_duration_ms, created_at. Proper escaping handled by crate.

### JSON
Versioned envelope: `{ "version": 1, "exported_at": "...", "transcripts": [...] }`. Single transcript uses `transcript` field, batch uses `transcripts` array + `count`.

## SRT Timestamp Generation
Since we don't have word-level timestamps, use sentence-proportional distribution:
1. Split text into sentences
2. Count words per sentence
3. Distribute total duration proportionally by word count
4. Each sentence = one SRT block

Document as "Timestamps are approximate" in UI.

## Tauri Integration
- `tauri-plugin-dialog` for save file dialog
- `tauri-plugin-clipboard-manager` for clipboard write
- One Rust command: `export_transcripts(format, ids)` returns formatted string
- Frontend shows save dialog, writes file

## Batch Export
- Selected: user picks transcript IDs
- All: empty IDs = export everything
- TXT: concatenated with separators
- SRT: cumulative time offset
- CSV: natural batch format
- Clipboard: good for single, prefer file for batch

## Dependencies
```toml
tauri-plugin-dialog = "2"
tauri-plugin-clipboard-manager = "2"
csv = "1"
chrono = "0.4"
```

## UI
Format selector (TXT/SRT/JSON/CSV) + two buttons (Save to File, Copy to Clipboard) + "Export all" checkbox.
