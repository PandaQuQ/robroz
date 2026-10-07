use std::env;
use std::path::Path;

fn main() {
    let task_id = env::var("OPENBOTS_TASK_ID").unwrap_or_else(|_| panic!("OPENBOTS_TASK_ID 未设置"));
    // Prompt comes from /workspace/tasks/{taskId}/prompt.txt, written by the host.
    // Takeover markers are read only from the read-only /control mount.
    hive_runner::run(Path::new("/workspace"), Path::new("/control"), &task_id)
        .unwrap_or_else(|err| panic!("写入结果失败: {err}"));
}
