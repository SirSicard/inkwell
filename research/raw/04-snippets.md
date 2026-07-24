# Snippets Engine - Research

## Key Insight
Inkwell doesn't need keystroke monitoring like Espanso/TextExpander. We own the transcription pipeline. Snippet expansion is a post-processing step between STT output and paste.

```
Audio → STT → Raw Text → [Dictionary] → [Snippets] → [Style] → Paste
```

## Trigger Detection
- **`aho-corasick` crate**: O(n) multi-pattern matching in a single pass. Used by ripgrep. Handles thousands of triggers with sub-microsecond performance on typical transcription segments.
- Word boundary post-filtering to avoid partial matches ("email" shouldn't trigger inside "emailing")
- Case-insensitive matching

## Data Model
```rust
struct Snippet {
    id: String,
    trigger: String,           // "sig", "addr", "eml"
    expansion: String,         // "Best regards,\nMattias Herzig"
    category_id: Option<String>,
    variables: Vec<Variable>,  // dynamic parts
    enabled: bool,
}

struct Category {
    id: String,
    name: String,              // "Email", "Code", "Personal"
    icon: Option<String>,
}

enum VariableType {
    Date { format: String },   // {date:%Y-%m-%d}
    Time { format: String },   // {time:%H:%M}
    Clipboard,                 // {clipboard}
    Cursor,                    // {cursor} - place cursor here after expansion
    Custom { name: String, prompt: String }, // ask user
}
```

## Variable System
- `{date}` / `{date:%B %d, %Y}` - current date with format
- `{time}` / `{time:%H:%M}` - current time
- `{clipboard}` - current clipboard content
- `{cursor}` - cursor placement marker (split paste into before/after)
- Custom variables with user prompt on expansion

## Storage
- JSON file in app data dir (`snippets.json`)
- Categories + snippets in one file (simple at this scale)

## Import/Export
- Native JSON export/import
- Espanso YAML import (parse `trigger:` and `replace:` fields)
- TextExpander CSV import (trigger, expansion columns)

## UX
- Snippet manager tab (Advanced Mode)
- Category sidebar + snippet list + editor
- Search/filter across all snippets
- "Test expansion" preview
- Trigger column shows the phrase, expansion shows preview (truncated)

## Prior Art
- **Espanso**: YAML config, regex triggers, OS keyboard hook. Overkill for Inkwell.
- **TextExpander**: GUI-first, groups, fill-in fields. Good UX reference.
- **PhraseExpress**: Autotext, macros, phrase folders. Enterprise-focused.

All three use keystroke monitoring. Inkwell's approach (post-STT pipeline) is simpler and more reliable since we control the text source.

## Implementation Order
1. Core engine (aho-corasick matching + variable interpolation)
2. JSON storage + Tauri CRUD commands
3. UI: snippet manager tab
4. Import from Espanso/TextExpander
