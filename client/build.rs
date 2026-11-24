//! Embeds the application icon into the Windows executable.
//!
//! A no-op for every other target, so Linux and macOS builds are unaffected.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../assets/brand/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../assets/brand/icon.ico");
        res.set("ProductName", "robroz");
        res.set("FileDescription", "robroz");
        if let Err(e) = res.compile() {
            // Fail loudly: a silent fallback would ship an exe with the default icon.
            panic!("failed to embed the Windows icon: {e}");
        }
    }
}
