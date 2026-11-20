//! Turns `tokens.json` (UI's single source, copied byte-for-byte) into
//! Rust constants (`src/generated.rs`) and CSS custom properties
//! (`generated/tokens.css`). Both outputs are checked in; a test fails if
//! either is stale. Run `cargo run -p robroz-tokens --bin gen` to refresh.
//!
//! Only `base` and `skins.solid` are emitted for now; platform skin crates
//! will read their own `skins.<name>` from the same file.
//!
//! Value mapping (generic, so new fields need no code change):
//! - number → `f32`; array of numbers → `[f32; N]`; bool → `bool`
//! - `"#rrggbb"` → `Rgb`; `"rgba(r,g,b,a)"` / `"transparent"` → `Rgba`
//! - `"120ms ease-out"` → `&str` plus `<NAME>_MS: u32`
//! - `"13/700"` → `&str` plus `<NAME>_SIZE: f32` and `<NAME>_WEIGHT: u16`
//! - other strings → `&str`; `null` → a comment (not used by this skin)

use serde_json::{Map, Value};

/// Top-level section emitted as shared brand values (keys come out sorted).
const BASE: &str = "base";
/// The one skin this crate owns.
const SKIN: &str = "solid";

fn ident(key: &str) -> String {
    let mut s: String = key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    if s.starts_with(|c: char| c.is_ascii_digit()) {
        s.insert(0, '_');
    }
    s
}

fn module(key: &str) -> String {
    let m: String = key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    match m.as_str() {
        "type" => "type_".into(),
        _ => m,
    }
}

fn float(v: f64) -> String {
    let s = format!("{v}");
    if s.contains('.') || s.contains('e') {
        s
    } else {
        format!("{s}.0")
    }
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let b = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([b(0)?, b(2)?, b(4)?])
}

fn parse_rgba(s: &str) -> Option<([u8; 3], f64)> {
    if s == "transparent" {
        return Some(([0, 0, 0], 0.0));
    }
    let inner = s.strip_prefix("rgba(")?.strip_suffix(')')?;
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if parts.len() != 4 {
        return None;
    }
    let c = |i: usize| parts[i].parse::<u8>().ok();
    Some(([c(0)?, c(1)?, c(2)?], parts[3].parse().ok()?))
}

fn parse_ms(s: &str) -> Option<u32> {
    let head = s.split_whitespace().next()?;
    head.strip_suffix("ms")?.parse().ok()
}

fn parse_size_weight(s: &str) -> Option<(f64, u16)> {
    let (a, b) = s.split_once('/')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

fn rgb_lit(c: [u8; 3]) -> String {
    format!("[0x{:02x}, 0x{:02x}, 0x{:02x}]", c[0], c[1], c[2])
}

fn emit_value(out: &mut String, pad: &str, key: &str, v: &Value) {
    let name = ident(key);
    match v {
        Value::Null => out.push_str(&format!("{pad}// {name}: not used by this skin\n")),
        Value::Bool(b) => out.push_str(&format!("{pad}pub const {name}: bool = {b};\n")),
        Value::Number(n) => out.push_str(&format!(
            "{pad}pub const {name}: f32 = {};\n",
            float(n.as_f64().unwrap_or_default())
        )),
        Value::Array(items) if items.iter().all(Value::is_number) => {
            let list: Vec<String> = items
                .iter()
                .map(|n| float(n.as_f64().unwrap_or_default()))
                .collect();
            out.push_str(&format!(
                "{pad}pub const {name}: [f32; {}] = [{}];\n",
                list.len(),
                list.join(", ")
            ));
        }
        Value::Array(items) => {
            let list: Vec<String> = items
                .iter()
                .map(|i| format!("{:?}", i.to_string()))
                .collect();
            out.push_str(&format!(
                "{pad}pub const {name}: [&str; {}] = [{}];\n",
                list.len(),
                list.join(", ")
            ));
        }
        Value::String(s) => {
            if let Some(c) = parse_hex(s) {
                out.push_str(&format!("{pad}pub const {name}: Rgb = {};\n", rgb_lit(c)));
            } else if let Some((c, a)) = parse_rgba(s) {
                out.push_str(&format!(
                    "{pad}pub const {name}: Rgba = Rgba {{ rgb: {}, alpha: {} }};\n",
                    rgb_lit(c),
                    float(a)
                ));
            } else {
                out.push_str(&format!("{pad}pub const {name}: &str = {s:?};\n"));
                if let Some(ms) = parse_ms(s) {
                    out.push_str(&format!("{pad}pub const {name}_MS: u32 = {ms};\n"));
                }
                if let Some((size, weight)) = parse_size_weight(s) {
                    out.push_str(&format!(
                        "{pad}pub const {name}_SIZE: f32 = {};\n{pad}pub const {name}_WEIGHT: u16 = {weight};\n",
                        float(size)
                    ));
                }
            }
        }
        Value::Object(map) => emit_module(out, pad, key, map),
    }
}

fn emit_module(out: &mut String, pad: &str, key: &str, map: &Map<String, Value>) {
    out.push_str(&format!("{pad}pub mod {} {{\n", module(key)));
    let inner = format!("{pad}    ");
    out.push_str(&format!(
        "{inner}#[allow(unused_imports)]\n{inner}use crate::{{Rgb, Rgba}};\n"
    ));
    for (k, v) in map {
        emit_value(out, &inner, k, v);
    }
    out.push_str(&format!("{pad}}}\n"));
}

fn parse(json: &str) -> Value {
    serde_json::from_str(json).expect("tokens.json is valid JSON")
}

fn object<'a>(v: &'a Value, path: &[&str]) -> &'a Map<String, Value> {
    let mut cur = v;
    for p in path {
        cur = &cur[*p];
    }
    cur.as_object()
        .unwrap_or_else(|| panic!("tokens.json: {} is not an object", path.join(".")))
}

/// Rust source for `src/generated.rs`.
pub fn rust(json: &str) -> String {
    let v = parse(json);
    let mut out = String::new();
    out.push_str("// Generated from crates/robroz-tokens/tokens.json. Do not edit.\n");
    out.push_str("// Regenerate: cargo run -p robroz-tokens --bin gen\n\n");
    if let Some(version) = v["version"].as_str() {
        out.push_str(&format!(
            "/// `tokens.json` version.\npub const VERSION: &str = {version:?};\n\n"
        ));
    }
    out.push_str("/// Shared brand values (same on every skin).\n");
    emit_module(&mut out, "", BASE, object(&v, &[BASE]));
    out.push_str("\n/// The default solid skin (robroz-ui's own), light and dark.\n");
    emit_module(&mut out, "", SKIN, object(&v, &["skins", SKIN]));
    out
}

fn css_value(v: &Value) -> Option<String> {
    match v {
        Value::Number(n) => Some(format!("{}px", n.as_f64().unwrap_or_default())),
        Value::String(s) if parse_hex(s).is_some() || parse_rgba(s).is_some() => Some(s.clone()),
        Value::String(s) => parse_ms(s).map(|ms| format!("{ms}ms")),
        _ => None,
    }
}

fn css_vars(prefix: &str, map: &Map<String, Value>, out: &mut Vec<(String, String)>) {
    for (k, v) in map {
        let name = format!("{prefix}-{}", k.replace('_', "-"));
        match v {
            Value::Object(m) => css_vars(&name, m, out),
            other => {
                if let Some(val) = css_value(other) {
                    out.push((name, val));
                }
            }
        }
    }
}

/// CSS custom properties from `base` and the solid skin: light under
/// `:root`, dark under `[data-theme="dark"]` and the system preference.
/// `extra` (component metrics owned by robroz-ui's SPEC) is appended to
/// `:root`.
pub fn css(json: &str, extra: &[(String, String)]) -> String {
    let v = parse(json);
    let mut base = Vec::new();
    let b = object(&v, &[BASE]);
    for (k, val) in b {
        let name = format!("--robroz-{}", k.replace('_', "-"));
        match val {
            // Status dots differ by scheme (emitted with the skin below);
            // contrast ratios are unitless rules, not CSS values.
            Value::Object(_) if k == "status_dot" || k == "contrast" => {}
            Value::Object(m) => css_vars(&name, m, &mut base),
            Value::Array(items) => {
                for (i, it) in items.iter().enumerate() {
                    if let Some(x) = css_value(it) {
                        base.push((format!("{name}-{i}"), x));
                    }
                }
            }
            _ => {}
        }
    }
    let scheme = |s: &str| {
        let mut vars = Vec::new();
        css_vars(
            "--robroz-dot",
            object(&v, &[BASE, "status_dot", s]),
            &mut vars,
        );
        css_vars("--robroz", object(&v, &["skins", SKIN, s]), &mut vars);
        vars
    };
    let line = |(k, val): &(String, String)| format!("  {k}: {val};\n");
    let mut out = String::new();
    out.push_str(
        "/* Generated from crates/robroz-tokens/tokens.json (base + solid). Do not edit.\n",
    );
    out.push_str("   Regenerate: cargo run -p robroz-tokens --bin gen */\n\n:root {\n");
    for kv in base
        .iter()
        .chain(scheme("light").iter())
        .chain(extra.iter())
    {
        out.push_str(&line(kv));
    }
    out.push_str("}\n\n");
    let dark: String = scheme("dark").iter().map(line).collect();
    out.push_str(":root[data-theme=\"dark\"] {\n");
    out.push_str(&dark);
    out.push_str(
        "}\n\n@media (prefers-color-scheme: dark) {\n  :root:not([data-theme=\"light\"]) {\n",
    );
    for l in dark.lines() {
        out.push_str(&format!("  {l}\n"));
    }
    out.push_str("  }\n}\n");
    out
}
