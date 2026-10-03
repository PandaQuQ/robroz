use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn result_path(root: &Path, task_id: &str) -> PathBuf {
    root.join("tasks").join(task_id).join("result.txt")
}

fn write_result(root: &Path, task_id: &str, prompt: &str) -> std::io::Result<()> {
    let path = result_path(root, task_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, prompt.as_bytes())
}

fn main() {
    let task_id = env::var("OPENBOTS_TASK_ID").unwrap_or_else(|_| panic!("OPENBOTS_TASK_ID 未设置"));
    let prompt = env::var("OPENBOTS_PROMPT").unwrap_or_else(|_| panic!("OPENBOTS_PROMPT 未设置"));
    write_result(Path::new("/workspace"), &task_id, &prompt)
        .unwrap_or_else(|err| panic!("写入结果失败: {err}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_prompt_bytes_at_relative_result_path() {
        let root = std::env::temp_dir().join(format!("hive-runner-{}", uuid_ish()));
        let task_id = "task-1";
        write_result(&root, task_id, "画一张草图").unwrap();
        let path = result_path(&root, task_id);
        assert_eq!(std::fs::read(&path).unwrap(), "画一张草图".as_bytes());
        let rel = path.strip_prefix(&root).unwrap();
        assert_eq!(rel, Path::new("tasks/task-1/result.txt"));
        let _ = std::fs::remove_dir_all(&root);
    }

    fn uuid_ish() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        format!("{}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos())
    }
}
