//! `robroz-host --version` / `-V` work without a token or any setup.

use std::process::Command;

fn run(flag: &str) -> std::process::Output {
    let dir = std::env::temp_dir().join(format!("robroz-version-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // A public listener with no token would make the host exit 2; the flag
    // must be handled before that check.
    Command::new(env!("CARGO_BIN_EXE_robroz-host"))
        .arg(flag)
        .current_dir(&dir)
        .env_remove("ROBROZ_TOKEN")
        .env_remove("ROBROZ_TOKEN_FILE")
        .env("ROBROZ_ADDR", "0.0.0.0:1")
        .env("ROBROZ_DB", dir.join("must-not-exist.db"))
        .output()
        .unwrap()
}

#[test]
fn version_flags_print_version_and_sha_and_exit_zero_without_a_token() {
    for flag in ["--version", "-V"] {
        let out = run(flag);
        assert!(out.status.success(), "{flag}: {out:?}");
        let stdout = String::from_utf8(out.stdout).unwrap();
        let line = stdout.trim_end();
        let rest = line
            .strip_prefix(&format!("robroz-host {} (", env!("CARGO_PKG_VERSION")))
            .unwrap_or_else(|| panic!("{flag}: {line:?}"));
        let sha = rest.strip_suffix(')').unwrap_or_else(|| panic!("{line:?}"));
        assert!(
            sha == "unknown" || (sha.len() == 7 && sha.chars().all(|c| c.is_ascii_hexdigit())),
            "{flag}: sha {sha:?}"
        );
        assert_eq!(stdout.lines().count(), 1, "{stdout:?}");
    }
    let dir = std::env::temp_dir().join(format!("robroz-version-{}", std::process::id()));
    assert!(
        !dir.join("must-not-exist.db").exists(),
        "no database opened"
    );
    let _ = std::fs::remove_dir_all(dir);
}
