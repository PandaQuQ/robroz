use std::fs;
use std::io;
use std::path::{Path, PathBuf};

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

/// Apply a prompt to the group volume. Shared by `hive-runner` and the fake sandbox.
///
/// `hive-action:get` lines are recorded as `GET` in `outbound.txt`.
/// `hive-action:external` / `hive-action:captcha` write `intent` and skip `result.txt`.
/// Otherwise the prompt bytes are written to `result.txt`.
pub fn apply_prompt(root: &Path, task_id: &str, prompt: &str) -> io::Result<()> {
    let dir = task_dir(root, task_id);
    fs::create_dir_all(&dir)?;

    let mut gets = 0usize;
    let mut intent: Option<&str> = None;
    for line in prompt.lines() {
        match line.trim() {
            "hive-action:get" => gets += 1,
            "hive-action:external" => intent = Some("external"),
            "hive-action:captcha" => intent = Some("captcha"),
            _ => {}
        }
    }
    if gets > 0 {
        let mut body = String::new();
        for _ in 0..gets {
            body.push_str("GET\n");
        }
        fs::write(outbound_path(root, task_id), body)?;
    }
    if let Some(kind) = intent {
        fs::write(intent_path(root, task_id), kind)?;
        return Ok(());
    }
    fs::write(result_path(root, task_id), prompt.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "hive-runner-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
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
        assert_eq!(fs::read(intent_path(&root, "ext")).unwrap(), b"external");
        assert!(!result_path(&root, "ext").exists());

        apply_prompt(&root, "cap", "请看\nhive-action:captcha").unwrap();
        assert_eq!(fs::read(intent_path(&root, "cap")).unwrap(), b"captcha");
        assert!(!result_path(&root, "cap").exists());
        let _ = fs::remove_dir_all(&root);
    }
}
