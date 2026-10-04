use std::env;
use std::path::Path;

fn main() {
    let task_id = env::var("OPENBOTS_TASK_ID").unwrap_or_else(|_| panic!("OPENBOTS_TASK_ID 未设置"));
    let prompt = env::var("OPENBOTS_PROMPT").unwrap_or_else(|_| panic!("OPENBOTS_PROMPT 未设置"));
    hive_runner::apply_prompt(Path::new("/workspace"), &task_id, &prompt)
        .unwrap_or_else(|err| panic!("写入结果失败: {err}"));
}
