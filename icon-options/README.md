# Icon source art

Seven 1024x1024 ink-drop candidates from the branding pass. Nothing here is referenced by the build. The shipped icons live in `src-tauri/icons/`, generated from one of these.

## In use

**`option-4-inverted.png`** is the current app icon. Verified by hash: it is byte-identical to `src-tauri/icons/icon-1024.png` (sha256 `aa0b0fa7a849215db8dae806efa192332c04c03dd852a17accaa34381cadcdf7`). Cream drop with a dark navy highlight, chosen in 0.1.1 because it stays visible on dark docks and taskbars, which the dark variants do not.

## The set

| File | What it is | Verdict |
|---|---|---|
| `option-4-inverted.png` | Cream drop, dark navy highlight | **Shipped.** Reads at 16px on dark and light backgrounds |
| `option-4.png` | Same drop inverted: near-black fill, cream highlight | Best dark variant. Keep as the light-background alternate |
| `option-1.png` | Black drop, soft cream highlight, slightly narrower silhouette | Keep. Closest runner-up to option 4 |
| `option-2.png` | Black drop, thin swept highlight | Keep. The highlight disappears at small sizes |
| `option-3.png` | Charcoal drop, large hard-edged highlight | Keep. Highlight is heavy relative to the drop |
| `option-5.png` | Flat cream drop, thin outline, small specular | Weak. Outline vanishes on light backgrounds |
| `option-6.png` | Cream drop with soft shading, no outline | Weak. Lowest contrast of the set |

Nothing is deleted. These are source art, they cost nothing to keep, and a menu bar template icon still has to be derived from one of them (see the macOS platform pass in [../TODO.md](../TODO.md)).

## If you regenerate the app icons

`cargo tauri icon icon-options/<chosen>.png` regenerates the whole `src-tauri/icons/` set. Check the result at 16px and 32px on both a dark and a light dock before committing.
