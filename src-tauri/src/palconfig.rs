//! Round-trip parser for PalWorldSettings.ini.
//!
//! The file is two lines: a section header and one long
//!   OptionSettings=(Key=Value,Key=Value,...)
//! line. A single malformed token makes the server silently revert EVERY setting to
//! default, so this parser preserves unknown keys verbatim, only touches the values it
//! is told to, and validates the result before it can be written back.

#[derive(Debug, Clone)]
pub struct PalConfig {
    pub header: String,
    /// (key, raw_value) in original order. raw_value keeps quotes for strings, e.g. `"My Server"`.
    pub pairs: Vec<(String, String)>,
}

pub fn parse(text: &str) -> Result<PalConfig, String> {
    let header = text
        .lines()
        .map(|l| l.trim())
        .find(|l| l.starts_with('['))
        .unwrap_or("[/Script/Pal.PalGameWorldSettings]")
        .to_string();

    let start = text
        .find("OptionSettings=(")
        .ok_or("No OptionSettings=(...) found in PalWorldSettings.ini")?;
    let open = start + "OptionSettings=(".len() - 1; // index of the '('

    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut in_q = false;
    let mut end = None;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => in_q = !in_q,
            b'(' if !in_q => depth += 1,
            b')' if !in_q => {
                depth -= 1;
                if depth == 0 {
                    end = Some(i);
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let end = end.ok_or("OptionSettings parentheses are not balanced")?;
    let inner = &text[open + 1..end];
    let pairs = split_pairs(inner)?;
    if pairs.is_empty() {
        return Err("OptionSettings had no keys".into());
    }
    Ok(PalConfig { header, pairs })
}

fn split_pairs(inner: &str) -> Result<Vec<(String, String)>, String> {
    let bytes = inner.as_bytes();
    let mut pairs = Vec::new();
    let mut depth = 0i32;
    let mut in_q = false;
    let mut tok_start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => in_q = !in_q,
            b'(' if !in_q => depth += 1,
            b')' if !in_q => depth -= 1,
            b',' if !in_q && depth == 0 => {
                push_pair(&inner[tok_start..i], &mut pairs)?;
                tok_start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    push_pair(&inner[tok_start..], &mut pairs)?;
    Ok(pairs)
}

fn push_pair(seg: &str, pairs: &mut Vec<(String, String)>) -> Result<(), String> {
    let seg = seg.trim();
    if seg.is_empty() {
        return Ok(());
    }
    let eq = seg
        .find('=')
        .ok_or_else(|| format!("Malformed setting (no '='): {seg}"))?;
    pairs.push((seg[..eq].trim().to_string(), seg[eq + 1..].to_string()));
    Ok(())
}

/// Set values for the given keys (append if not present); leave everything else untouched.
pub fn apply_updates(cfg: &mut PalConfig, updates: &[(String, String)]) {
    for (k, v) in updates {
        if let Some(pair) = cfg.pairs.iter_mut().find(|(pk, _)| pk == k) {
            pair.1 = v.clone();
        } else {
            cfg.pairs.push((k.clone(), v.clone()));
        }
    }
}

/// Serialize back to the strict two-line form, validating we can't produce a file that
/// would nuke the server's settings.
pub fn serialize(cfg: &PalConfig) -> Result<String, String> {
    if cfg.pairs.is_empty() {
        return Err("Refusing to write an empty OptionSettings line".into());
    }
    let inner = cfg
        .pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(",");

    if inner.contains('\n') || inner.contains('\r') {
        return Err("A value contains a line break — refusing to write (would corrupt the file)".into());
    }
    // Balanced parens and even quote count guard.
    let mut depth = 0i32;
    let mut quotes = 0usize;
    for b in inner.bytes() {
        match b {
            b'"' => quotes += 1,
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        if depth < 0 {
            return Err("Unbalanced ')' in settings — refusing to write".into());
        }
    }
    if depth != 0 {
        return Err("Unbalanced '(' in settings — refusing to write".into());
    }
    if quotes % 2 != 0 {
        return Err("Odd number of quotes in settings — refusing to write".into());
    }

    Ok(format!("{}\nOptionSettings=({})\n", cfg.header, inner))
}
