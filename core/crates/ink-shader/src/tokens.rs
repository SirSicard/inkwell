//! The design tokens, written once in `design/tokens.json` and generated for each shell: Swift
//! constants for the Mac app, and for the Windows shell a XAML resource dictionary (the colours
//! per theme, the faces, sizes and radii) and C# constants.
//!
//! The reader takes exactly the shape below and refuses anything else: a missing or unknown key, a
//! colour not written `#RRGGBB`, a number out of its range. A typo in the tokens fails here, never
//! in a shell as a default.

use std::fmt::{self, Write as _};

use serde_json::{Map, Value};

/// `design/tokens.json`, as this crate was built with it.
pub const TOKENS_JSON: &str = include_str!("../../../../design/tokens.json");

/// The tokens, from the repository root.
pub const TOKENS_PATH: &str = "design/tokens.json";

/// Where the generated Swift lives, from the repository root.
pub const SWIFT_OUT: &str = "mac/Sources/Inkwell/Generated/GlowTokens.swift";

/// Where the generated XAML lives, from the repository root.
pub const XAML_OUT: &str = "windows/Inkwell/Generated/GlowTokens.xaml";

/// Where the generated C# lives, from the repository root.
pub const CSHARP_OUT: &str = "windows/Inkwell.Core/Generated/GlowTokens.g.cs";

/// Why the tokens did not generate: what is wrong, and where in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenError(String);

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{TOKENS_PATH}: {}", self.0)
    }
}

impl std::error::Error for TokenError {}

type Result<T> = std::result::Result<T, TokenError>;

/// The two modes, as the tokens and the settings name them.
const MODES: [&str; 2] = ["light", "dark"];

/// Each mode's colours, in the order every output lists them, with what each is for.
const MODE_COLOURS: &[(&str, &str)] = &[
    ("background", "The window's background."),
    ("page", "The page behind the window."),
    ("card", "A card's fill, over what is behind it, blurred."),
    ("text", "Text."),
    ("secondary", "Secondary text."),
    ("border", "Borders and hairlines."),
    ("buttonFill", "A button's fill."),
    ("buttonLabel", "A button's label."),
    ("alert", "Something needs the user."),
    (
        "alertCard",
        "The fill of a card about something that needs the user.",
    ),
    ("ink", "The ink drop the orb blots down to."),
    ("idleOrb", "The orb at rest."),
];

/// How a dot colour is fitted to a mode, after the luminance weights (`luma`).
const FIT: &[(&str, &str)] = &[
    (
        "darkLiftBelow",
        "In dark mode, a colour whose luminance is under this is lifted toward white,",
    ),
    (
        "darkLift",
        "by this share of the way: c + (1 - c) * darkLift, per channel.",
    ),
    (
        "lightDimAbove",
        "In light mode, a colour whose luminance is over this is dimmed,",
    ),
    ("lightDim", "to c * lightDim, per channel."),
    (
        "partnerLift",
        "A colour's second shade in the orb: c + (1 - c) * partnerLift, per channel.",
    ),
];

/// Corner radii.
const RADII: &[(&str, &str)] = &[
    ("card", "A card."),
    ("window", "The window."),
    ("pill", "A pill button: fully round ends."),
];

/// The type roles' faces.
const FACES: &[(&str, &str)] = &[
    ("display", "Titles and the greeting."),
    ("ui", "The interface."),
    ("mono", "Times, counts and versions."),
];

/// The type sizes.
const SIZES: &[(&str, &str)] = &[
    ("greeting", "Today's greeting."),
    ("screenTitle", "A screen's title."),
    ("heading", "A heading."),
    ("body", "Body text."),
    ("caption", "Captions."),
    ("eyebrow", "The small line over a heading."),
];

/// The edge glow's numbers, after its strokes.
const EDGE: &[(&str, &str)] = &[
    (
        "blendBand",
        "Half the blend between the two colours, as a share of the width.",
    ),
    (
        "lean",
        "The blend's centre is 0.5 + lean * (you - them), as a share of the width.",
    ),
    (
        "flowAmplitude",
        "How far the blend drifts, as a share of the width,",
    ),
    ("flowRate", "and how fast, in radians per second."),
];

/// Where the orb sits.
const ORBS: &[(&str, &str)] = &[
    ("main", "Behind the main window's content."),
    ("drop", "In the Drop."),
];

/// An sRGB colour, `0xRRGGBB`, and its opacity.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Colour {
    rgb: u32,
    alpha: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct Mode {
    /// In [`MODE_COLOURS`] order.
    colours: Vec<Colour>,
    card_blur: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct Preset {
    id: String,
    name: String,
    you: Colour,
    them: Colour,
}

/// `design/tokens.json`, read and checked. Each list is in the order of its table above.
#[derive(Debug, Clone, PartialEq)]
pub struct Tokens {
    /// Light, then dark.
    modes: Vec<Mode>,
    presets: Vec<Preset>,
    luma: [f64; 3],
    fit: Vec<f64>,
    radii: Vec<f64>,
    /// (Mac, Windows).
    faces: Vec<(String, String)>,
    sizes: Vec<f64>,
    /// (width, alpha), widest first.
    strokes: Vec<(f64, f64)>,
    edge: Vec<f64>,
    /// (x, y from the top, unit).
    orbs: Vec<[f64; 3]>,
}

fn fail<T>(message: String) -> Result<T> {
    Err(TokenError(message))
}

/// `v`, at `path`, as an object holding exactly `keys`.
fn object<'a>(v: &'a Value, path: &str, keys: &[&str]) -> Result<&'a Map<String, Value>> {
    let Some(map) = v.as_object() else {
        return fail(format!("{path} is not an object"));
    };
    if let Some(key) = map.keys().find(|k| !keys.contains(&k.as_str())) {
        return fail(format!("{path} has an unknown key \"{key}\""));
    }
    if let Some(key) = keys.iter().find(|k| !map.contains_key(**k)) {
        return fail(format!("{path} has no \"{key}\""));
    }
    Ok(map)
}

fn number(v: &Value, path: &str, min: f64, max: f64) -> Result<f64> {
    match v.as_f64() {
        Some(n) if n.is_finite() && (min..=max).contains(&n) => Ok(n),
        _ => fail(format!("{path} is not a number from {min} to {max}")),
    }
}

/// A colour: `"#RRGGBB"`, or `{"color": "#RRGGBB", "alpha": 0..1}`.
fn colour(v: &Value, path: &str) -> Result<Colour> {
    match v {
        Value::String(s) => Ok(Colour {
            rgb: hex(s, path)?,
            alpha: 1.0,
        }),
        Value::Object(_) => {
            let map = object(v, path, &["color", "alpha"])?;
            let at = format!("{path}.color");
            let Some(s) = map["color"].as_str() else {
                return fail(format!("{at} is not a colour written #RRGGBB"));
            };
            Ok(Colour {
                rgb: hex(s, &at)?,
                alpha: number(&map["alpha"], &format!("{path}.alpha"), 0.0, 1.0)?,
            })
        }
        _ => fail(format!(
            "{path} is not a colour: \"#RRGGBB\", or {{\"color\": \"#RRGGBB\", \"alpha\": 0..1}}"
        )),
    }
}

fn hex(s: &str, path: &str) -> Result<u32> {
    match s.strip_prefix('#') {
        Some(digits) if digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_hexdigit()) => {
            u32::from_str_radix(digits, 16)
                .map_err(|_| TokenError(format!("{path} is not a colour written #RRGGBB")))
        }
        _ => fail(format!("{path} is not a colour written #RRGGBB")),
    }
}

/// A name for the generated sources: printable, and nothing a Swift or C# string would need to
/// escape.
fn text(v: &Value, path: &str) -> Result<String> {
    match v.as_str() {
        Some(s) if !s.is_empty() && s.chars().all(|c| !c.is_control() && c != '"' && c != '\\') => {
            Ok(s.to_owned())
        }
        _ => fail(format!(
            "{path} is not a name (text, without quotes, backslashes or control characters)"
        )),
    }
}

fn keys(table: &[(&'static str, &str)]) -> Vec<&'static str> {
    table.iter().map(|(k, _)| *k).collect()
}

/// Reads and checks `json`, the contents of `design/tokens.json`.
pub fn parse(json: &str) -> std::result::Result<Tokens, TokenError> {
    let root: Value =
        serde_json::from_str(json).map_err(|e| TokenError(format!("not JSON: {e}")))?;
    let top = object(
        &root,
        "the file",
        &[
            "about", "modes", "presets", "fit", "radii", "type", "edgeGlow", "orb",
        ],
    )?;

    let modes_map = object(&top["modes"], "modes", &MODES)?;
    let mut modes = Vec::new();
    for mode in MODES {
        let at = format!("modes.{mode}");
        let m = object(&modes_map[mode], &at, &["colors", "cardBlur"])?;
        let at_colours = format!("{at}.colors");
        let c = object(&m["colors"], &at_colours, &keys(MODE_COLOURS))?;
        let colours = MODE_COLOURS
            .iter()
            .map(|(k, _)| colour(&c[*k], &format!("{at_colours}.{k}")))
            .collect::<Result<Vec<_>>>()?;
        modes.push(Mode {
            colours,
            card_blur: number(&m["cardBlur"], &format!("{at}.cardBlur"), 0.0, 200.0)?,
        });
    }

    let Some(list) = top["presets"].as_array().filter(|l| !l.is_empty()) else {
        return fail("presets is not a list of presets".into());
    };
    let mut presets: Vec<Preset> = Vec::new();
    for (i, p) in list.iter().enumerate() {
        let at = format!("presets[{i}]");
        let m = object(p, &at, &["id", "name", "you", "them"])?;
        let id = text(&m["id"], &format!("{at}.id"))?;
        if !id.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
            return fail(format!("{at}.id is not lowercase letters and underscores"));
        }
        if presets.iter().any(|q| q.id == id) {
            return fail(format!("{at}.id \"{id}\" is used twice"));
        }
        presets.push(Preset {
            name: text(&m["name"], &format!("{at}.name"))?,
            you: colour(&m["you"], &format!("{at}.you"))?,
            them: colour(&m["them"], &format!("{at}.them"))?,
            id,
        });
    }

    let mut fit_keys = vec!["luma"];
    fit_keys.extend(keys(FIT));
    let fit_map = object(&top["fit"], "fit", &fit_keys)?;
    let luma = match fit_map["luma"].as_array().map(Vec::as_slice) {
        Some([r, g, b]) => [
            number(r, "fit.luma[0]", 0.0, 1.0)?,
            number(g, "fit.luma[1]", 0.0, 1.0)?,
            number(b, "fit.luma[2]", 0.0, 1.0)?,
        ],
        _ => return fail("fit.luma is not three weights: red, green, blue".into()),
    };
    let fit = FIT
        .iter()
        .map(|(k, _)| number(&fit_map[*k], &format!("fit.{k}"), 0.0, 1.0))
        .collect::<Result<Vec<_>>>()?;

    let radii_map = object(&top["radii"], "radii", &keys(RADII))?;
    let radii = RADII
        .iter()
        .map(|(k, _)| number(&radii_map[*k], &format!("radii.{k}"), 0.0, 10_000.0))
        .collect::<Result<Vec<_>>>()?;

    let type_map = object(&top["type"], "type", &["faces", "sizes"])?;
    let faces_map = object(&type_map["faces"], "type.faces", &keys(FACES))?;
    let faces = FACES
        .iter()
        .map(|(k, _)| {
            let at = format!("type.faces.{k}");
            let f = object(&faces_map[*k], &at, &["mac", "windows"])?;
            Ok((
                text(&f["mac"], &format!("{at}.mac"))?,
                text(&f["windows"], &format!("{at}.windows"))?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let sizes_map = object(&type_map["sizes"], "type.sizes", &keys(SIZES))?;
    let sizes = SIZES
        .iter()
        .map(|(k, _)| number(&sizes_map[*k], &format!("type.sizes.{k}"), 1.0, 500.0))
        .collect::<Result<Vec<_>>>()?;

    let mut edge_keys = vec!["strokes"];
    edge_keys.extend(keys(EDGE));
    let edge_map = object(&top["edgeGlow"], "edgeGlow", &edge_keys)?;
    let Some(list) = edge_map["strokes"].as_array().filter(|l| !l.is_empty()) else {
        return fail("edgeGlow.strokes is not a list of [width, alpha]".into());
    };
    let strokes = list
        .iter()
        .enumerate()
        .map(|(i, s)| match s.as_array().map(Vec::as_slice) {
            Some([w, a]) => Ok((
                number(w, &format!("edgeGlow.strokes[{i}][0]"), 0.0, 500.0)?,
                number(a, &format!("edgeGlow.strokes[{i}][1]"), 0.0, 1.0)?,
            )),
            _ => fail(format!("edgeGlow.strokes[{i}] is not [width, alpha]")),
        })
        .collect::<Result<Vec<_>>>()?;
    let edge = EDGE
        .iter()
        .map(|(k, _)| number(&edge_map[*k], &format!("edgeGlow.{k}"), 0.0, 100.0))
        .collect::<Result<Vec<_>>>()?;

    let orb_map = object(&top["orb"], "orb", &keys(ORBS))?;
    let orbs = ORBS
        .iter()
        .map(|(k, _)| {
            let at = format!("orb.{k}");
            let o = object(&orb_map[*k], &at, &["x", "y", "unit"])?;
            Ok([
                number(&o["x"], &format!("{at}.x"), 0.0, 1.0)?,
                number(&o["y"], &format!("{at}.y"), 0.0, 1.0)?,
                number(&o["unit"], &format!("{at}.unit"), 0.0, 10.0)?,
            ])
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(Tokens {
        modes,
        presets,
        luma,
        fit,
        radii,
        faces,
        sizes,
        strokes,
        edge,
        orbs,
    })
}

/// A number as a source literal: the shortest form that reads back as the same value.
fn num(n: f64) -> String {
    format!("{n}")
}

/// `key` with its first letter in capitals: a C# or XAML name.
fn pascal(key: &str) -> String {
    let mut chars = key.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_ascii_uppercase().to_string() + chars.as_str()
    })
}

fn swift_colour(c: Colour) -> String {
    if c.alpha == 1.0 {
        format!("GlowColor(0x{:06X})", c.rgb)
    } else {
        format!("GlowColor(0x{:06X}, alpha: {})", c.rgb, num(c.alpha))
    }
}

fn csharp_colour(c: Colour) -> String {
    if c.alpha == 1.0 {
        format!("new GlowColor(0x{:06X})", c.rgb)
    } else {
        format!("new GlowColor(0x{:06X}, {})", c.rgb, num(c.alpha))
    }
}

/// `#AARRGGBB`, as XAML writes a colour.
fn xaml_colour(c: Colour) -> String {
    let alpha = (c.alpha * 255.0).round() as u8;
    format!("#{alpha:02X}{:06X}", c.rgb)
}

fn xml_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The generated Swift, for the Mac app (`mac/Sources/Inkwell/Generated/GlowTokens.swift`).
pub fn swift(t: &Tokens) -> String {
    let mut out = String::from(
        "// Generated from design/tokens.json by `cargo run -p ink-shader --bin ink-tokens`.\n\
         // Do not edit: change the tokens and regenerate. A core test fails while this is stale.\n\
         \n\
         /// An sRGB colour from the design tokens: its hex value, `0xRRGGBB`, and its opacity.\n\
         struct GlowColor: Equatable, Sendable {\n    \
             let hex: UInt32\n    \
             let alpha: Double\n\
         \n    \
             init(_ hex: UInt32, alpha: Double = 1) {\n        \
                 self.hex = hex\n        \
                 self.alpha = alpha\n    \
             }\n\
         \n    \
             var red: Double { Double((hex >> 16) & 0xFF) / 255 }\n    \
             var green: Double { Double((hex >> 8) & 0xFF) / 255 }\n    \
             var blue: Double { Double(hex & 0xFF) / 255 }\n\
         }\n\
         \n\
         /// One mode's colours.\n\
         struct GlowPalette: Equatable, Sendable {\n",
    );
    for (key, doc) in MODE_COLOURS {
        let _ = writeln!(out, "    /// {doc}\n    let {key}: GlowColor");
    }
    out.push_str(
        "    /// The blur behind a card, in points.\n    \
             let cardBlur: Double\n\
         }\n\
         \n\
         /// A dot colour preset: your colour and the far end's.\n\
         struct GlowPreset: Equatable, Sendable, Identifiable {\n    \
             /// What `appearance.dots.light` and `appearance.dots.dark` store.\n    \
             let id: String\n    \
             /// What Settings shows.\n    \
             let name: String\n    \
             let you: GlowColor\n    \
             let them: GlowColor\n\
         }\n\
         \n\
         /// Where the orb sits in its view: its centre as shares of the width and of the height (from\n\
         /// the top), and its unit (the orb shader's `unit`) as a share of the shorter side.\n\
         struct GlowOrbPlacement: Equatable, Sendable {\n    \
             let x: Double\n    \
             let y: Double\n    \
             let unit: Double\n\
         }\n\
         \n\
         /// One stroke of the edge glow: its width in points, and its opacity.\n\
         struct GlowStroke: Equatable, Sendable {\n    \
             let width: Double\n    \
             let alpha: Double\n\
         }\n\
         \n\
         /// The design tokens (design/tokens.json).\n\
         enum GlowTokens {\n",
    );
    for (mode, m) in MODES.iter().zip(&t.modes) {
        let _ = writeln!(
            out,
            "    /// The {mode} mode's colours.\n    static let {mode} = GlowPalette("
        );
        for ((key, _), c) in MODE_COLOURS.iter().zip(&m.colours) {
            let _ = writeln!(out, "        {key}: {},", swift_colour(*c));
        }
        let _ = writeln!(out, "        cardBlur: {}\n    )\n", num(m.card_blur));
    }
    out.push_str(
        "    /// The dot colour presets, in the order Settings lists them.\n    \
             static let presets: [GlowPreset] = [\n",
    );
    for p in &t.presets {
        let _ = writeln!(
            out,
            "        GlowPreset(\n            id: \"{}\", name: \"{}\",\n            you: {}, them: {}),",
            p.id,
            p.name,
            swift_colour(p.you),
            swift_colour(p.them)
        );
    }
    out.push_str(
        "    ]\n\
         \n    \
             /// How a dot colour is fitted to the mode. Luminance is lumaRed * r + lumaGreen * g +\n    \
             /// lumaBlue * b, each channel 0 to 1.\n    \
             enum Fit {\n",
    );
    for (name, weight) in ["lumaRed", "lumaGreen", "lumaBlue"].iter().zip(t.luma) {
        let _ = writeln!(out, "        static let {name}: Double = {}", num(weight));
    }
    for ((key, doc), n) in FIT.iter().zip(&t.fit) {
        let _ = writeln!(
            out,
            "        /// {doc}\n        static let {key}: Double = {}",
            num(*n)
        );
    }
    out.push_str("    }\n\n    /// Corner radii, in points.\n    enum Radius {\n");
    for ((key, doc), n) in RADII.iter().zip(&t.radii) {
        let _ = writeln!(
            out,
            "        /// {doc}\n        static let {key}: Double = {}",
            num(*n)
        );
    }
    out.push_str(
        "    }\n\
         \n    \
             /// The type roles: the system's own faces, none bundled. Sizes in points.\n    \
             enum TypeRole {\n",
    );
    for ((key, doc), (mac, _)) in FACES.iter().zip(&t.faces) {
        let _ = writeln!(
            out,
            "        /// {doc}\n        static let {key} = \"{mac}\""
        );
    }
    for ((key, doc), n) in SIZES.iter().zip(&t.sizes) {
        let _ = writeln!(
            out,
            "        /// {doc}\n        static let {key}: Double = {}",
            num(*n)
        );
    }
    out.push_str(
        "    }\n\
         \n    \
             /// The glow round the window's edge.\n    \
             enum EdgeGlow {\n        \
                 /// Its strokes, from the widest and faintest to the narrowest.\n        \
                 static let strokes: [GlowStroke] = [\n",
    );
    for (w, a) in &t.strokes {
        let _ = writeln!(
            out,
            "            GlowStroke(width: {}, alpha: {}),",
            num(*w),
            num(*a)
        );
    }
    out.push_str("        ]\n");
    for ((key, doc), n) in EDGE.iter().zip(&t.edge) {
        let _ = writeln!(
            out,
            "        /// {doc}\n        static let {key}: Double = {}",
            num(*n)
        );
    }
    out.push_str("    }\n\n    /// Where the orb sits.\n    enum Orb {\n");
    for ((key, doc), [x, y, unit]) in ORBS.iter().zip(&t.orbs) {
        let _ = writeln!(
            out,
            "        /// {doc}\n        static let {key} = GlowOrbPlacement(x: {}, y: {}, unit: {})",
            num(*x),
            num(*y),
            num(*unit)
        );
    }
    out.push_str("    }\n}\n");
    out
}

/// The generated XAML, for the Windows shell (`windows/Inkwell/Generated/GlowTokens.xaml`): each
/// theme's colours as a `Color` and a `SolidColorBrush`, then the faces, sizes and radii.
pub fn xaml(t: &Tokens) -> String {
    // No double hyphen in an XML comment, so the command is not spelled out here.
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <!-- Generated from design/tokens.json by the ink-tokens generator (core/crates/ink-shader).\n     \
              Do not edit: change the tokens and regenerate. A core test fails while this is stale.\n     \
              Each theme's colours as GlowNameColor and GlowNameBrush; then the faces, sizes and\n     \
              corner radii, the same in both themes. -->\n\
         <ResourceDictionary\n    \
             xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\"\n    \
             xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\">\n    \
             <ResourceDictionary.ThemeDictionaries>\n",
    );
    for (mode, m) in MODES.iter().zip(&t.modes) {
        let _ = writeln!(
            out,
            "        <ResourceDictionary x:Key=\"{}\">",
            pascal(mode)
        );
        for ((key, _), c) in MODE_COLOURS.iter().zip(&m.colours) {
            let name = pascal(key);
            let colour = xaml_colour(*c);
            let _ = writeln!(
                out,
                "            <Color x:Key=\"Glow{name}Color\">{colour}</Color>\n            \
                 <SolidColorBrush x:Key=\"Glow{name}Brush\" Color=\"{colour}\" />"
            );
        }
        out.push_str("        </ResourceDictionary>\n");
    }
    out.push_str("    </ResourceDictionary.ThemeDictionaries>\n\n");
    for ((key, _), (_, windows)) in FACES.iter().zip(&t.faces) {
        let _ = writeln!(
            out,
            "    <FontFamily x:Key=\"Glow{}FontFamily\">{}</FontFamily>",
            pascal(key),
            xml_text(windows)
        );
    }
    for ((key, _), n) in SIZES.iter().zip(&t.sizes) {
        let _ = writeln!(
            out,
            "    <x:Double x:Key=\"Glow{}FontSize\">{}</x:Double>",
            pascal(key),
            num(*n)
        );
    }
    for ((key, _), n) in RADII.iter().zip(&t.radii) {
        let _ = writeln!(
            out,
            "    <CornerRadius x:Key=\"Glow{}CornerRadius\">{}</CornerRadius>",
            pascal(key),
            num(*n)
        );
    }
    out.push_str("</ResourceDictionary>\n");
    out
}

/// The generated C#, for the Windows shell (`windows/Inkwell.Core/Generated/GlowTokens.g.cs`).
pub fn csharp(t: &Tokens) -> String {
    let mut out = String::from(
        "// <auto-generated>\n\
         // Generated from design/tokens.json by `cargo run -p ink-shader --bin ink-tokens`.\n\
         // Do not edit: change the tokens and regenerate. A core test fails while this is stale.\n\
         // </auto-generated>\n\
         #nullable enable\n\
         \n\
         namespace Inkwell.Core.Glow;\n\
         \n\
         /// <summary>\n\
         /// An sRGB colour from the design tokens: its hex value, <c>0xRRGGBB</c>, and its opacity.\n\
         /// </summary>\n\
         public readonly record struct GlowColor(uint Hex, double Alpha = 1)\n\
         {\n    \
             /// <summary>Red, 0 to 1.</summary>\n    \
             public double Red => ((Hex >> 16) & 0xFF) / 255.0;\n\
         \n    \
             /// <summary>Green, 0 to 1.</summary>\n    \
             public double Green => ((Hex >> 8) & 0xFF) / 255.0;\n\
         \n    \
             /// <summary>Blue, 0 to 1.</summary>\n    \
             public double Blue => (Hex & 0xFF) / 255.0;\n\
         }\n\
         \n\
         /// <summary>\n\
         /// One mode's colours, and the blur behind a card in pixels.\n\
         /// </summary>\n",
    );
    for (key, doc) in MODE_COLOURS {
        let _ = writeln!(out, "/// <param name=\"{}\">{doc}</param>", pascal(key));
    }
    out.push_str("/// <param name=\"CardBlur\">The blur behind a card.</param>\npublic sealed record GlowPalette(\n");
    for (key, _) in MODE_COLOURS {
        let _ = writeln!(out, "    GlowColor {},", pascal(key));
    }
    out.push_str(
        "    double CardBlur);\n\
         \n\
         /// <summary>\n\
         /// A dot colour preset: your colour and the far end's.\n\
         /// </summary>\n\
         /// <param name=\"Id\">What <c>appearance.dots.light</c> and <c>appearance.dots.dark</c> store.</param>\n\
         /// <param name=\"Name\">What Settings shows.</param>\n\
         /// <param name=\"You\">Your colour.</param>\n\
         /// <param name=\"Them\">The far end's.</param>\n\
         public sealed record GlowPreset(string Id, string Name, GlowColor You, GlowColor Them);\n\
         \n\
         /// <summary>\n\
         /// Where the orb sits in its view: its centre as shares of the width and of the height (from\n\
         /// the top), and its unit (the orb shader's <c>unit</c>) as a share of the shorter side.\n\
         /// </summary>\n\
         public readonly record struct GlowOrbPlacement(double X, double Y, double Unit);\n\
         \n\
         /// <summary>\n\
         /// One stroke of the edge glow: its width in pixels, and its opacity.\n\
         /// </summary>\n\
         public readonly record struct GlowStroke(double Width, double Alpha);\n\
         \n\
         /// <summary>\n\
         /// The design tokens (design/tokens.json).\n\
         /// </summary>\n\
         public static class GlowTokens\n\
         {\n",
    );
    for (mode, m) in MODES.iter().zip(&t.modes) {
        let _ = writeln!(
            out,
            "    /// <summary>The {mode} mode's colours.</summary>\n    \
             public static GlowPalette {} {{ get; }} = new(",
            pascal(mode)
        );
        for ((key, _), c) in MODE_COLOURS.iter().zip(&m.colours) {
            let _ = writeln!(out, "        {}: {},", pascal(key), csharp_colour(*c));
        }
        let _ = writeln!(out, "        CardBlur: {});\n", num(m.card_blur));
    }
    out.push_str(
        "    /// <summary>The dot colour presets, in the order Settings lists them.</summary>\n    \
             public static global::System.Collections.Generic.IReadOnlyList<GlowPreset> Presets { get; } =\n    \
             [\n",
    );
    for p in &t.presets {
        let _ = writeln!(
            out,
            "        new(\"{}\", \"{}\", {}, {}),",
            p.id,
            p.name,
            csharp_colour(p.you),
            csharp_colour(p.them)
        );
    }
    out.push_str(
        "    ];\n\
         \n    \
             /// <summary>\n    \
             /// How a dot colour is fitted to the mode. Luminance is LumaRed * r + LumaGreen * g +\n    \
             /// LumaBlue * b, each channel 0 to 1.\n    \
             /// </summary>\n    \
             public static class Fit\n    \
             {\n",
    );
    for (name, weight) in ["LumaRed", "LumaGreen", "LumaBlue"].iter().zip(t.luma) {
        let _ = writeln!(
            out,
            "        /// <summary>A luminance weight.</summary>\n        \
             public const double {name} = {};",
            num(weight)
        );
    }
    for ((key, doc), n) in FIT.iter().zip(&t.fit) {
        let _ = writeln!(
            out,
            "        /// <summary>{}</summary>\n        public const double {} = {};",
            doc.replace(key, &pascal(key)),
            pascal(key),
            num(*n)
        );
    }
    out.push_str(
        "    }\n\
         \n    \
             /// <summary>Corner radii, in pixels.</summary>\n    \
             public static class Radius\n    \
             {\n",
    );
    for ((key, doc), n) in RADII.iter().zip(&t.radii) {
        let _ = writeln!(
            out,
            "        /// <summary>{doc}</summary>\n        public const double {} = {};",
            pascal(key),
            num(*n)
        );
    }
    out.push_str(
        "    }\n\
         \n    \
             /// <summary>The type roles: the system's own faces, none bundled. Sizes in pixels.</summary>\n    \
             public static class TypeRole\n    \
             {\n",
    );
    for ((key, doc), (_, windows)) in FACES.iter().zip(&t.faces) {
        let _ = writeln!(
            out,
            "        /// <summary>{doc}</summary>\n        public const string {} = \"{windows}\";",
            pascal(key)
        );
    }
    for ((key, doc), n) in SIZES.iter().zip(&t.sizes) {
        let _ = writeln!(
            out,
            "        /// <summary>{doc}</summary>\n        public const double {} = {};",
            pascal(key),
            num(*n)
        );
    }
    out.push_str(
        "    }\n\
         \n    \
             /// <summary>The glow round the window's edge.</summary>\n    \
             public static class EdgeGlow\n    \
             {\n        \
                 /// <summary>Its strokes, from the widest and faintest to the narrowest.</summary>\n        \
                 public static global::System.Collections.Generic.IReadOnlyList<GlowStroke> Strokes { get; } =\n        \
                 [\n",
    );
    for (w, a) in &t.strokes {
        let _ = writeln!(out, "            new({}, {}),", num(*w), num(*a));
    }
    out.push_str("        ];\n");
    for ((key, doc), n) in EDGE.iter().zip(&t.edge) {
        let _ = writeln!(
            out,
            "        /// <summary>{doc}</summary>\n        public const double {} = {};",
            pascal(key),
            num(*n)
        );
    }
    out.push_str(
        "    }\n\
         \n    \
             /// <summary>Where the orb sits.</summary>\n    \
             public static class Orb\n    \
             {\n",
    );
    for ((key, doc), [x, y, unit]) in ORBS.iter().zip(&t.orbs) {
        let _ = writeln!(
            out,
            "        /// <summary>{doc}</summary>\n        \
             public static GlowOrbPlacement {} {{ get; }} = new({}, {}, {});",
            pascal(key),
            num(*x),
            num(*y),
            num(*unit)
        );
    }
    out.push_str("    }\n}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens() -> Tokens {
        parse(TOKENS_JSON).expect("design/tokens.json reads")
    }

    #[test]
    fn the_tokens_read() {
        let t = tokens();
        assert_eq!(t.modes.len(), 2);
        assert_eq!(t.modes[0].colours.len(), MODE_COLOURS.len());
        assert_eq!(t.presets.len(), 7);
        assert_eq!(t.presets[0].id, "indigo");
        assert_eq!(t.luma, [0.299, 0.587, 0.114]);
        assert_eq!(t.strokes.first(), Some(&(44.0, 0.06)));
        assert_eq!(t.strokes.last(), Some(&(2.0, 1.0)));
    }

    #[test]
    fn a_wrong_shape_is_refused_by_where_it_is() {
        let edit = |from: &str, to: &str| {
            assert!(TOKENS_JSON.contains(from), "{from}");
            let e = parse(&TOKENS_JSON.replacen(from, to, 1)).expect_err(to);
            e.to_string()
        };
        assert!(edit("\"#FBF8F4\"", "\"#FBF8F\"").contains("modes.light.colors.background"));
        assert!(edit("\"#FBF8F4\"", "\"FBF8F4\"").contains("#RRGGBB"));
        assert!(
            edit("\"alpha\": 0.72", "\"alpha\": 1.5").contains("modes.light.colors.card.alpha")
        );
        assert!(edit("\"idleOrb\"", "\"idle\"").contains("unknown key \"idle\""));
        assert!(edit("\"id\": \"dusk\"", "\"id\": \"indigo\"").contains("used twice"));
        assert!(edit("\"id\": \"dusk\"", "\"id\": \"Dusk\"").contains("presets[1].id"));
        assert!(edit("\"name\": \"Dusk\"", "\"name\": \"Du\\\"sk\"").contains("presets[1].name"));
        assert!(edit("[2, 1]", "[2]").contains("edgeGlow.strokes[4]"));
        assert!(edit("\"unit\": 1.15", "\"size\": 1.15").contains("orb.drop"));
        assert!(parse("not json").is_err());
    }

    #[test]
    fn the_swift_has_both_modes_the_presets_and_the_rules() {
        let swift = swift(&tokens());
        assert!(swift.starts_with("// Generated from design/tokens.json"));
        assert!(swift.contains("static let light = GlowPalette("));
        assert!(swift.contains("static let dark = GlowPalette("));
        assert!(swift.contains("card: GlowColor(0xFFFFFF, alpha: 0.72),"));
        assert!(swift.contains("id: \"ink_sand\", name: \"Ink & Sand\","));
        assert!(swift.contains("static let partnerLift: Double = 0.4"));
        assert!(swift.contains("static let display = \"New York\""));
        assert!(swift.contains("static let main = GlowOrbPlacement(x: 0.56, y: 0.26, unit: 0.72)"));
        assert!(swift.ends_with("}\n"));
    }

    #[test]
    fn the_xaml_has_a_light_and_a_dark_theme() {
        let xaml = xaml(&tokens());
        assert!(xaml.contains("<ResourceDictionary x:Key=\"Light\">"));
        assert!(xaml.contains("<ResourceDictionary x:Key=\"Dark\">"));
        // 0.72 of 255 is 184 (B8), 0.62 is 158 (9E).
        assert!(xaml.contains("<Color x:Key=\"GlowCardColor\">#B8FFFFFF</Color>"));
        assert!(xaml.contains("<SolidColorBrush x:Key=\"GlowCardBrush\" Color=\"#9E1C1A24\" />"));
        assert!(xaml.contains("<FontFamily x:Key=\"GlowDisplayFontFamily\">Sitka</FontFamily>"));
        assert!(xaml.contains("<x:Double x:Key=\"GlowBodyFontSize\">15</x:Double>"));
        assert!(xaml.contains("<CornerRadius x:Key=\"GlowCardCornerRadius\">22</CornerRadius>"));
        // An XML comment may not hold a double hyphen.
        let comments = xaml
            .split("<!--")
            .skip(1)
            .map(|c| c.split("-->").next().unwrap_or(""));
        for comment in comments {
            assert!(!comment.contains("--"), "{comment}");
        }
    }

    #[test]
    fn the_csharp_is_marked_generated_and_has_the_rules() {
        let cs = csharp(&tokens());
        assert!(cs.starts_with("// <auto-generated>"));
        assert!(cs.contains("namespace Inkwell.Core.Glow;"));
        assert!(cs.contains("Card: new GlowColor(0x1C1A24, 0.62),"));
        assert!(cs.contains(
            "new(\"indigo\", \"Indigo & Coral\", new GlowColor(0x6B5CFF), new GlowColor(0xFFA34D)),"
        ));
        assert!(cs.contains("public const double DarkLiftBelow = 0.3;"));
        assert!(cs.contains("public const string Display = \"Sitka\";"));
        assert!(cs.contains("public static GlowOrbPlacement Drop { get; } = new(0.5, 0.5, 1.15);"));
    }
}
