//! In-container executor, plus the file formats the host shares with it.
//!
//! Files in the group volume (mounted at `/workspace`, writable by the container):
//! - `tasks/{taskId}/result.txt`, `outbound.txt`, `intent`: written by the runner.
//!   The host reads `intent` once when the container exits, keeps a validated copy
//!   in its database and deletes the file.
//!
//! Files in the task's control dir (mounted read-only at `/control`, written only
//! by the host):
//! - `prompt.txt`: the task prompt, written before `docker run`.
//! - `takeover`: written only by the host on resume. One sha256 per line, each the
//!   hash of an exact `hive-action:` line a person already took over.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

pub fn task_dir(root: &Path, task_id: &str) -> PathBuf {
    root.join("tasks").join(task_id)
}

pub fn result_path(root: &Path, task_id: &str) -> PathBuf {
    task_dir(root, task_id).join("result.txt")
}

pub fn outbound_path(root: &Path, task_id: &str) -> PathBuf {
    task_dir(root, task_id).join("outbound.txt")
}

pub fn intent_path(root: &Path, task_id: &str) -> PathBuf {
    task_dir(root, task_id).join("intent")
}

/// `prompt.txt` inside one task's control dir (`/control/prompt.txt` in the container).
pub fn prompt_path(control: &Path) -> PathBuf {
    control.join("prompt.txt")
}

/// `takeover` inside one task's control dir (`/control` in the container).
pub fn takeover_path(control: &Path) -> PathBuf {
    control.join("takeover")
}

/// Kind of a blocking action line, or None. Matching ignores surrounding spaces.
pub fn blocking_kind(line: &str) -> Option<&'static str> {
    match line.trim() {
        "hive-action:external" => Some("external"),
        "hive-action:captcha" => Some("captcha"),
        _ => None,
    }
}

/// Lowercase hex sha256 of the exact line bytes (no newline).
pub fn action_hash(line: &str) -> String {
    let digest = Sha256::digest(line.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// What the runner records when it stops on a blocking line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    pub kind: String,
    pub hash: Option<String>,
    pub line: Option<String>,
}

/// Three lines: kind, sha256 of the line, the exact line.
pub fn format_intent(kind: &str, line: &str) -> String {
    format!("{kind}\n{}\n{line}", action_hash(line))
}

/// Parses [`format_intent`]. A file holding only the kind (older runners) still
/// yields the kind, with no hash or line.
pub fn parse_intent(text: &str) -> Option<Intent> {
    let mut parts = text.splitn(3, '\n');
    let kind = parts.next().unwrap_or("").trim();
    if kind.is_empty() {
        return None;
    }
    let hash = parts.next().map(str::trim).filter(|h| !h.is_empty()).map(str::to_string);
    let line = parts.next().map(str::to_string);
    Some(Intent {
        kind: kind.to_string(),
        hash,
        line,
    })
}

/// A blocking action the host checked against the stored prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidIntent {
    pub kind: String,
    pub line: String,
    pub hash: String,
}

/// Host side: check an `intent` written by the container.
///
/// The container controls that file, so it is not trusted on its own. Only a line
/// that is a blocking action of the stated kind and appears verbatim in the task's
/// stored prompt is accepted, and the hash is recomputed here.
pub fn validate_intent(prompt: &str, intent: &str) -> Option<ValidIntent> {
    let intent = parse_intent(intent)?;
    let line = intent.line?;
    if blocking_kind(&line) != Some(intent.kind.as_str()) {
        return None;
    }
    if !prompt.lines().any(|candidate| candidate == line) {
        return None;
    }
    Some(ValidIntent {
        kind: intent.kind,
        hash: action_hash(&line),
        line,
    })
}

/// Host side: the hash to put in `takeover`, from [`validate_intent`].
pub fn takeover_hash(prompt: &str, intent: &str) -> Option<String> {
    validate_intent(prompt, intent).map(|valid| valid.hash)
}

/// Hashes listed in `takeover`. Missing file means nothing was taken over.
pub fn read_takeover(control: &Path) -> HashSet<String> {
    fs::read_to_string(takeover_path(control))
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Host side: add `hash` to `takeover` in `control`, creating the dir and file.
pub fn append_takeover(control: &Path, hash: &str) -> io::Result<()> {
    fs::create_dir_all(control)?;
    let mut known = read_takeover(control);
    if !known.insert(hash.to_string()) {
        return Ok(());
    }
    let path = takeover_path(control);
    let mut body = fs::read_to_string(&path).unwrap_or_default();
    if !body.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }
    body.push_str(hash);
    body.push('\n');
    fs::write(path, body)
}

/// Container entry: prompt and takeovers only from the read-only control dir
/// (`{control}/prompt.txt`, `{control}/takeover`). Nothing under `workspace` is
/// read; results are written there.
pub fn run(workspace: &Path, control: &Path, task_id: &str) -> io::Result<()> {
    let prompt = fs::read_to_string(prompt_path(control))?;
    let approved = read_takeover(control);
    apply_prompt_with(workspace, task_id, &prompt, &approved)
}

/// [`apply_prompt_with`] with nothing taken over.
pub fn apply_prompt(root: &Path, task_id: &str, prompt: &str) -> io::Result<()> {
    apply_prompt_with(root, task_id, prompt, &HashSet::new())
}

/// Apply a prompt to the group volume.
///
/// `hive-action:get` lines are recorded as `GET` in `outbound.txt`.
/// The first `hive-action:external` / `hive-action:captcha` line whose hash is not in
/// `approved` writes `intent` and skips `result.txt`. Approved lines are skipped and
/// left out of `result.txt`. Otherwise the prompt is written to `result.txt`
/// (byte for byte when nothing was skipped).
pub fn apply_prompt_with(
    root: &Path,
    task_id: &str,
    prompt: &str,
    approved: &HashSet<String>,
) -> io::Result<()> {
    let dir = task_dir(root, task_id);
    fs::create_dir_all(&dir)?;

    let mut gets = 0usize;
    let mut blocked: Option<(&str, &str)> = None;
    let mut skipped = false;
    let mut kept: Vec<&str> = Vec::new();
    for line in prompt.lines() {
        if line.trim() == "hive-action:get" {
            gets += 1;
        }
        if let Some(kind) = blocking_kind(line) {
            if approved.contains(&action_hash(line)) {
                skipped = true;
                continue;
            }
            if blocked.is_none() {
                blocked = Some((kind, line));
            }
        }
        kept.push(line);
    }
    if gets > 0 {
        fs::write(outbound_path(root, task_id), "GET\n".repeat(gets))?;
    }
    if let Some((kind, line)) = blocked {
        fs::write(intent_path(root, task_id), format_intent(kind, line))?;
        return Ok(());
    }
    if skipped {
        fs::write(result_path(root, task_id), kept.join("\n"))?;
    } else {
        fs::write(result_path(root, task_id), prompt.as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "hive-runner-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn intent_kind(root: &Path, task_id: &str) -> String {
        parse_intent(&fs::read_to_string(intent_path(root, task_id)).unwrap())
            .unwrap()
            .kind
    }

    /// Workspace plus a separate control dir, like the two container mounts.
    fn mounts(task_id: &str, prompt: &str) -> (PathBuf, PathBuf, PathBuf) {
        let base = scratch();
        let workspace = base.join("workspace");
        let control = base.join("control");
        fs::create_dir_all(task_dir(&workspace, task_id)).unwrap();
        fs::create_dir_all(&control).unwrap();
        fs::write(prompt_path(&control), prompt).unwrap();
        (base, workspace, control)
    }

    #[test]
    fn writes_prompt_bytes_at_relative_result_path() {
        let root = scratch();
        apply_prompt(&root, "task-1", "画一张草图").unwrap();
        let path = result_path(&root, "task-1");
        assert_eq!(fs::read(&path).unwrap(), "画一张草图".as_bytes());
        assert!(!intent_path(&root, "task-1").exists());
        assert!(!outbound_path(&root, "task-1").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn get_lines_are_recorded_and_result_is_still_written() {
        let root = scratch();
        let prompt = "hive-action:get\nhive-action:get\n画一张草图";
        apply_prompt(&root, "task-1", prompt).unwrap();
        assert_eq!(fs::read(outbound_path(&root, "task-1")).unwrap(), b"GET\nGET\n");
        assert_eq!(fs::read(result_path(&root, "task-1")).unwrap(), prompt.as_bytes());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn external_and_captcha_write_intent_without_result() {
        let root = scratch();
        apply_prompt(&root, "ext", "hive-action:external\n画一张草图").unwrap();
        assert_eq!(intent_kind(&root, "ext"), "external");
        assert!(!result_path(&root, "ext").exists());

        apply_prompt(&root, "cap", "请看\nhive-action:captcha").unwrap();
        assert_eq!(intent_kind(&root, "cap"), "captcha");
        assert!(!result_path(&root, "cap").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn intent_records_kind_exact_line_and_hash() {
        let root = scratch();
        apply_prompt(&root, "ext", "开头\n  hive-action:external \n结尾").unwrap();
        let text = fs::read_to_string(intent_path(&root, "ext")).unwrap();
        let intent = parse_intent(&text).unwrap();
        assert_eq!(intent.kind, "external");
        assert_eq!(intent.line.as_deref(), Some("  hive-action:external "));
        assert_eq!(intent.hash, Some(action_hash("  hive-action:external ")));
        assert_eq!(text.lines().next(), Some("external"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn old_kind_only_intent_still_parses() {
        let intent = parse_intent("external").unwrap();
        assert_eq!(intent.kind, "external");
        assert_eq!(intent.hash, None);
        assert_eq!(intent.line, None);
        assert!(parse_intent("  ").is_none());
    }

    #[test]
    fn action_hash_is_sha256_hex() {
        // sha256("abc")
        assert_eq!(
            action_hash("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn run_reads_prompt_file() {
        let (base, workspace, control) = mounts("t1", "画一张草图");
        run(&workspace, &control, "t1").unwrap();
        assert_eq!(fs::read(result_path(&workspace, "t1")).unwrap(), "画一张草图".as_bytes());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn forged_marker_inside_workspace_does_not_skip() {
        let line = "hive-action:external";
        let (base, workspace, control) = mounts("t1", "hive-action:external\n正文");
        // The container can write anywhere under /workspace. None of these count.
        fs::write(task_dir(&workspace, "t1").join("takeover"), format!("{}\n", action_hash(line))).unwrap();
        fs::write(workspace.join("takeover"), format!("{}\n", action_hash(line))).unwrap();
        fs::create_dir_all(workspace.join("control")).unwrap();
        fs::write(workspace.join("control/takeover"), format!("{}\n", action_hash(line))).unwrap();

        run(&workspace, &control, "t1").unwrap();
        assert_eq!(intent_kind(&workspace, "t1"), "external");
        assert!(!result_path(&workspace, "t1").exists());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn marker_with_different_hash_does_not_skip() {
        let (base, workspace, control) = mounts("t1", "hive-action:external\n正文");
        append_takeover(&control, &action_hash("hive-action:captcha")).unwrap();
        append_takeover(&control, &action_hash(" hive-action:external")).unwrap();
        run(&workspace, &control, "t1").unwrap();
        assert_eq!(intent_kind(&workspace, "t1"), "external");
        assert!(!result_path(&workspace, "t1").exists());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn matching_marker_skips_only_that_line() {
        let prompt = "hive-action:external\n正文\nhive-action:captcha";
        let (base, workspace, control) = mounts("t1", prompt);
        append_takeover(&control, &action_hash("hive-action:external")).unwrap();

        run(&workspace, &control, "t1").unwrap();
        let intent = parse_intent(&fs::read_to_string(intent_path(&workspace, "t1")).unwrap()).unwrap();
        assert_eq!(intent.kind, "captcha", "a different blocking line still blocks");
        assert_eq!(intent.line.as_deref(), Some("hive-action:captcha"));
        assert!(!result_path(&workspace, "t1").exists());

        fs::remove_file(intent_path(&workspace, "t1")).unwrap();
        append_takeover(&control, &action_hash("hive-action:captcha")).unwrap();
        run(&workspace, &control, "t1").unwrap();
        assert!(!intent_path(&workspace, "t1").exists());
        assert_eq!(fs::read_to_string(result_path(&workspace, "t1")).unwrap(), "正文");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn takeover_hash_requires_line_from_prompt() {
        let prompt = "hive-action:external\n正文";
        let good = format_intent("external", "hive-action:external");
        assert_eq!(takeover_hash(prompt, &good), Some(action_hash("hive-action:external")));
        // Line not in the stored prompt.
        assert_eq!(takeover_hash(prompt, &format_intent("captcha", "hive-action:captcha")), None);
        // Kind does not match the line.
        assert_eq!(takeover_hash(prompt, "captcha\nxx\nhive-action:external"), None);
        // A non-blocking line from the prompt.
        assert_eq!(takeover_hash(prompt, "external\nxx\n正文"), None);
        // Kind only.
        assert_eq!(takeover_hash(prompt, "external"), None);
        // Stated hash is ignored; the host recomputes it.
        assert_eq!(
            takeover_hash(prompt, "external\nbogus\nhive-action:external"),
            Some(action_hash("hive-action:external"))
        );
    }

    #[test]
    fn append_takeover_dedupes() {
        let control = scratch().join("c");
        append_takeover(&control, "aa").unwrap();
        append_takeover(&control, "aa").unwrap();
        append_takeover(&control, "bb").unwrap();
        assert_eq!(fs::read_to_string(takeover_path(&control)).unwrap(), "aa\nbb\n");
    }

    #[test]
    fn run_reads_control_prompt_and_ignores_workspace_prompt() {
        let (base, workspace, control) = mounts("t1", "真正的任务");
        // A prompt file the container could plant in the volume.
        fs::write(task_dir(&workspace, "t1").join("prompt.txt"), "伪造的任务").unwrap();
        run(&workspace, &control, "t1").unwrap();
        assert_eq!(fs::read_to_string(result_path(&workspace, "t1")).unwrap(), "真正的任务");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn run_does_not_fall_back_to_workspace_prompt() {
        let (base, workspace, control) = mounts("t1", "x");
        fs::remove_file(prompt_path(&control)).unwrap();
        fs::write(task_dir(&workspace, "t1").join("prompt.txt"), "伪造的任务").unwrap();
        assert!(run(&workspace, &control, "t1").is_err());
        assert!(!result_path(&workspace, "t1").exists());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn validate_intent_returns_kind_line_and_recomputed_hash() {
        let prompt = "hive-action:external\n正文";
        let valid = validate_intent(prompt, "external\nbogus\nhive-action:external").unwrap();
        assert_eq!(valid.kind, "external");
        assert_eq!(valid.line, "hive-action:external");
        assert_eq!(valid.hash, action_hash("hive-action:external"));
        assert_eq!(validate_intent(prompt, &format_intent("captcha", "hive-action:captcha")), None);
    }
}
