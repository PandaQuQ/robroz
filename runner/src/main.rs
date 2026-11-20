use std::env;
use std::path::Path;

fn main() {
    let task_id = env::var("ROBROZ_TASK_ID").unwrap_or_else(|_| panic!("ROBROZ_TASK_ID 未设置"));
    // Prompt (/control/prompt.txt) and takeover markers (/control/takeover) are read
    // only from the read-only /control mount, written by the host.
    hive_runner::run(Path::new("/workspace"), Path::new("/control"), &task_id)
        .unwrap_or_else(|err| panic!("写入结果失败: {err}"));
}
