//! Regenerates `src/generated.rs` and `generated/tokens.css` from
//! `tokens.json`. Run: `cargo run -p robroz-tokens --bin gen`

use std::path::Path;

fn main() -> std::io::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let json = std::fs::read_to_string(root.join(robroz_tokens::JSON_PATH))?;
    for (rel, body) in [
        (
            robroz_tokens::RUST_PATH,
            robroz_tokens::codegen::rust(&json),
        ),
        (robroz_tokens::CSS_PATH, robroz_tokens::css_from(&json)),
    ] {
        let path = root.join(rel);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, body)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}
