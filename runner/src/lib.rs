//! In-container executor, plus the file formats the host shares with it.
//!
//! Files in the group volume (mounted at `/workspace`, writable by the container):
//! - `tasks/{taskId}/result.txt`, `outbound.txt`, `intent`: written by the runner.
//!   `intent` only signals "blocked". The host never reads its content: when the
//!   container exits it recomputes the blocking line itself with
//!   [`robroz_policy::blocking_line`] from the stored prompt and the takeover hashes,
//!   then deletes the file.
//!
//! Files in the task's control dir (mounted read-only at `/control`, written only
//! by the host):
//! - `prompt.txt`: the task prompt, written before `docker run`.
//! - `reply.txt`: the model's answer, written by the host before `docker run`
//!   when a model is configured. Absent means no model: the prompt is echoed.
//! - `takeover`: written only by the host on resume. One sha256 per line, each the
//!   hash of an exact `robroz-action:` line a person already took over.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub use robroz_policy::{action_hash, blocking_kind, blocking_line, BlockingLine};

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

/// `reply.txt` inside one task's control dir: the model's answer, if any.
pub fn reply_path(control: &Path) -> PathBuf {
    control.join("reply.txt")
}

/// `takeover` inside one task's control dir (`/control` in the container).
pub fn takeover_path(control: &Path) -> PathBuf {
    control.join("takeover")
}

/// What the runner writes into `intent`, for people reading the volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    pub kind: String,
    pub hash: Option<String>,
    pub line: Option<String>,
}

/// Three lines: kind, sha256 of the line, the exact line. Informational only:
/// the host never parses it.
pub fn format_intent(kind: &str, line: &str) -> String {
    format!("{kind}\n{}\n{line}", action_hash(line))
}

/// Parses [`format_intent`]. A file holding only the kind still yields the kind.
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
    apply_prompt_with(workspace, task_id, &prompt, &approved)?;
    // Blocking and outbound still come from the prompt; the model only
    // replaces the echoed text once the task is allowed to finish.
    match fs::read_to_string(reply_path(control)) {
        Ok(reply) if result_path(workspace, task_id).exists() => fs::write(result_path(workspace, task_id), reply),
        Ok(_) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// [`apply_prompt_with`] with nothing taken over.
pub fn apply_prompt(root: &Path, task_id: &str, prompt: &str) -> io::Result<()> {
    apply_prompt_with(root, task_id, prompt, &HashSet::new())
}

/// Apply a prompt to the group volume.
///
/// `robroz-action:get` lines are recorded as `GET` in `outbound.txt`.
/// Where to stop comes from [`blocking_line`] (shared with the host): if it finds
/// an unapproved blocking line, write `intent` and skip `result.txt`. Otherwise
/// write the prompt to `result.txt`, leaving out approved blocking lines (byte for
/// byte when there were none).
pub fn apply_prompt_with(
    root: &Path,
    task_id: &str,
    prompt: &str,
    approved: &HashSet<String>,
) -> io::Result<()> {
    let dir = task_dir(root, task_id);
    fs::create_dir_all(&dir)?;

    let gets = prompt.lines().filter(|line| line.trim() == "robroz-action:get").count();
    if gets > 0 {
        fs::write(outbound_path(root, task_id), "GET\n".repeat(gets))?;
    }
    if let Some(stop) = blocking_line(prompt, approved) {
        fs::write(intent_path(root, task_id), format_intent(stop.kind, &stop.line))?;
        return Ok(());
    }
    // No unapproved blocking line left: every blocking line here was taken over.
    let kept: Vec<&str> = prompt.lines().filter(|line| blocking_kind(line).is_none()).collect();
    if kept.len() == prompt.lines().count() {
        fs::write(result_path(root, task_id), prompt.as_bytes())?;
    } else {
        fs::write(result_path(root, task_id), kept.join("\n"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh dir per call; a counter, not the clock, so parallel tests never share one.
    fn scratch() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("robroz-runner-{}-{n}", std::process::id()));
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
        let prompt = "robroz-action:get\nrobroz-action:get\n画一张草图";
        apply_prompt(&root, "task-1", prompt).unwrap();
        assert_eq!(fs::read(outbound_path(&root, "task-1")).unwrap(), b"GET\nGET\n");
        assert_eq!(fs::read(result_path(&root, "task-1")).unwrap(), prompt.as_bytes());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn external_and_captcha_write_intent_without_result() {
        let root = scratch();
        apply_prompt(&root, "ext", "robroz-action:external\n画一张草图").unwrap();
        assert_eq!(intent_kind(&root, "ext"), "external");
        assert!(!result_path(&root, "ext").exists());

        apply_prompt(&root, "cap", "请看\nrobroz-action:captcha").unwrap();
        assert_eq!(intent_kind(&root, "cap"), "captcha");
        assert!(!result_path(&root, "cap").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn intent_records_kind_exact_line_and_hash() {
        let root = scratch();
        apply_prompt(&root, "ext", "开头\n  robroz-action:external \n结尾").unwrap();
        let text = fs::read_to_string(intent_path(&root, "ext")).unwrap();
        let intent = parse_intent(&text).unwrap();
        assert_eq!(intent.kind, "external");
        assert_eq!(intent.line.as_deref(), Some("  robroz-action:external "));
        assert_eq!(intent.hash, Some(action_hash("  robroz-action:external ")));
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
        let line = "robroz-action:external";
        let (base, workspace, control) = mounts("t1", "robroz-action:external\n正文");
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
        let (base, workspace, control) = mounts("t1", "robroz-action:external\n正文");
        append_takeover(&control, &action_hash("robroz-action:captcha")).unwrap();
        append_takeover(&control, &action_hash(" robroz-action:external")).unwrap();
        run(&workspace, &control, "t1").unwrap();
        assert_eq!(intent_kind(&workspace, "t1"), "external");
        assert!(!result_path(&workspace, "t1").exists());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn matching_marker_skips_only_that_line() {
        let prompt = "robroz-action:external\n正文\nrobroz-action:captcha";
        let (base, workspace, control) = mounts("t1", prompt);
        append_takeover(&control, &action_hash("robroz-action:external")).unwrap();

        run(&workspace, &control, "t1").unwrap();
        let intent = parse_intent(&fs::read_to_string(intent_path(&workspace, "t1")).unwrap()).unwrap();
        assert_eq!(intent.kind, "captcha", "a different blocking line still blocks");
        assert_eq!(intent.line.as_deref(), Some("robroz-action:captcha"));
        assert!(!result_path(&workspace, "t1").exists());

        fs::remove_file(intent_path(&workspace, "t1")).unwrap();
        append_takeover(&control, &action_hash("robroz-action:captcha")).unwrap();
        run(&workspace, &control, "t1").unwrap();
        assert!(!intent_path(&workspace, "t1").exists());
        assert_eq!(fs::read_to_string(result_path(&workspace, "t1")).unwrap(), "正文");
        let _ = fs::remove_dir_all(&base);
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
    fn runner_stops_where_shared_blocking_line_says() {
        let prompt = "开头\n robroz-action:captcha\n正文\nrobroz-action:external\nrobroz-action:captcha";
        let mut approved = HashSet::<String>::new();
        loop {
            let (base, workspace, control) = mounts("t1", prompt);
            for hash in &approved {
                append_takeover(&control, hash).unwrap();
            }
            run(&workspace, &control, "t1").unwrap();
            match robroz_policy::blocking_line(prompt, &approved) {
                Some(expected) => {
                    let intent = parse_intent(&fs::read_to_string(intent_path(&workspace, "t1")).unwrap()).unwrap();
                    assert_eq!(intent.kind, expected.kind);
                    assert_eq!(intent.line.as_deref(), Some(expected.line.as_str()));
                    assert!(!result_path(&workspace, "t1").exists());
                    approved.insert(expected.hash);
                }
                None => {
                    assert!(!intent_path(&workspace, "t1").exists());
                    assert_eq!(fs::read_to_string(result_path(&workspace, "t1")).unwrap(), "开头\n正文");
                    let _ = fs::remove_dir_all(&base);
                    break;
                }
            }
            let _ = fs::remove_dir_all(&base);
        }
        assert_eq!(approved.len(), 3, "three distinct blocking lines, three takeovers");
    }

    #[test]
    fn model_reply_replaces_echo_but_not_blocking() {
        let (base, workspace, control) = mounts("t1", "写周报");
        fs::write(reply_path(&control), "周报：本周完成三项").unwrap();
        run(&workspace, &control, "t1").unwrap();
        assert_eq!(fs::read_to_string(result_path(&workspace, "t1")).unwrap(), "周报：本周完成三项");
        let _ = fs::remove_dir_all(&base);

        let (base, workspace, control) = mounts("t2", "发邮件\nrobroz-action:external");
        fs::write(reply_path(&control), "已发送").unwrap();
        run(&workspace, &control, "t2").unwrap();
        assert!(!result_path(&workspace, "t2").exists(), "still blocked, reply unused");
        assert!(intent_path(&workspace, "t2").exists());
        let _ = fs::remove_dir_all(&base);
    }
}
