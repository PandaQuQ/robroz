use std::io;
use std::path::Path;
use std::process::Command;
#[cfg(test)]
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub struct SandboxError {
    pub message: String,
}

impl SandboxError {
    fn new(message: impl Into<String>) -> Self {
        Self { message: message.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerPhase {
    Running,
    Exited { code: i32 },
    Missing,
}

pub struct RunRequest<'a> {
    pub name: &'a str,
    pub image: &'a str,
    pub group_dir: &'a Path,
    pub task_id: &'a str,
    pub prompt: &'a str,
}

pub trait Sandbox {
    fn start(&mut self, req: &RunRequest<'_>) -> Result<(), SandboxError>;
    fn phase(&mut self, name: &str) -> Result<ContainerPhase, SandboxError>;
    fn remove(&mut self, name: &str) -> Result<(), SandboxError>;
}

pub struct ExecOutput {
    pub code: i32,
    pub stdout: String,
}

pub trait CommandExec: Send {
    fn exec(&mut self, args: &[String]) -> io::Result<ExecOutput>;
}

pub struct SystemExec;

impl CommandExec for SystemExec {
    fn exec(&mut self, args: &[String]) -> io::Result<ExecOutput> {
        if args.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty argv"));
        }
        let output = Command::new(&args[0]).args(&args[1..]).output()?;
        Ok(ExecOutput {
            code: output.status.code().unwrap_or(1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        })
    }
}

#[cfg(test)]
struct RecordingExec {
    calls: Arc<Mutex<Vec<Vec<String>>>>,
    code: i32,
    stdout: String,
}

#[cfg(test)]
impl CommandExec for RecordingExec {
    fn exec(&mut self, args: &[String]) -> io::Result<ExecOutput> {
        self.calls.lock().expect("argv lock").push(args.to_vec());
        Ok(ExecOutput {
            code: self.code,
            stdout: self.stdout.clone(),
        })
    }
}

pub struct DockerSandbox {
    exec: Box<dyn CommandExec>,
}

impl DockerSandbox {
    pub fn new() -> Self {
        Self { exec: Box::new(SystemExec) }
    }

    #[cfg(test)]
    fn with_exec(exec: Box<dyn CommandExec>) -> Self {
        Self { exec }
    }
}

pub fn container_name(task_id: &str) -> String {
    format!("openbots-{task_id}")
}

pub fn result_rel(task_id: &str) -> String {
    format!("tasks/{task_id}/result.txt")
}

fn run_argv(req: &RunRequest<'_>) -> Vec<String> {
    vec![
        "docker".into(),
        "run".into(),
        "-d".into(),
        "--name".into(),
        req.name.to_string(),
        "--network".into(),
        "none".into(),
        "-v".into(),
        format!("{}:/workspace", req.group_dir.display()),
        "-e".into(),
        format!("OPENBOTS_TASK_ID={}", req.task_id),
        "-e".into(),
        format!("OPENBOTS_PROMPT={}", req.prompt),
        req.image.to_string(),
    ]
}

fn inspect_argv(name: &str) -> Vec<String> {
    vec![
        "docker".into(),
        "inspect".into(),
        "--format".into(),
        "{{.State.Running}} {{.State.ExitCode}}".into(),
        name.to_string(),
    ]
}

fn rm_argv(name: &str) -> Vec<String> {
    vec!["docker".into(), "rm".into(), "-f".into(), name.to_string()]
}

impl Sandbox for DockerSandbox {
    fn start(&mut self, req: &RunRequest<'_>) -> Result<(), SandboxError> {
        let args = run_argv(req);
        let output = self.exec.exec(&args).map_err(|err| SandboxError::new(err.to_string()))?;
        if output.code != 0 {
            return Err(SandboxError::new(format!("docker run exited {}", output.code)));
        }
        Ok(())
    }

    fn phase(&mut self, name: &str) -> Result<ContainerPhase, SandboxError> {
        let output = self
            .exec
            .exec(&inspect_argv(name))
            .map_err(|err| SandboxError::new(err.to_string()))?;
        if output.code != 0 {
            return Ok(ContainerPhase::Missing);
        }
        let mut parts = output.stdout.split_whitespace();
        let running = parts.next().unwrap_or("");
        let code: i32 = parts.next().unwrap_or("0").parse().unwrap_or(1);
        if running == "true" {
            Ok(ContainerPhase::Running)
        } else {
            Ok(ContainerPhase::Exited { code })
        }
    }

    fn remove(&mut self, name: &str) -> Result<(), SandboxError> {
        let output = self
            .exec
            .exec(&rm_argv(name))
            .map_err(|err| SandboxError::new(err.to_string()))?;
        if output.code != 0 {
            return Err(SandboxError::new(format!("docker rm exited {}", output.code)));
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) struct FakeContainer {
    pub name: String,
    pub group_dir: std::path::PathBuf,
    pub task_id: String,
    pub prompt: String,
    pub phase: ContainerPhase,
}

#[cfg(test)]
pub(crate) struct FakeSandbox {
    pub fail_start: bool,
    pub containers: Vec<FakeContainer>,
}

#[cfg(test)]
impl FakeSandbox {
    pub(crate) fn new() -> Self {
        Self {
            fail_start: false,
            containers: Vec::new(),
        }
    }

    pub(crate) fn names(&self) -> Vec<String> {
        self.containers.iter().map(|c| c.name.clone()).collect()
    }

    /// Simulate the runner finishing. When `write_file` is set, the prompt is
    /// written under the group directory and left there after removal.
    pub(crate) fn finish(&mut self, code: i32, write_file: bool) {
        for container in &mut self.containers {
            if write_file {
                let path = container
                    .group_dir
                    .join("tasks")
                    .join(&container.task_id)
                    .join("result.txt");
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).expect("create result dir");
                }
                std::fs::write(&path, container.prompt.as_bytes()).expect("write result");
            }
            container.phase = ContainerPhase::Exited { code };
        }
    }
}

#[cfg(test)]
impl Sandbox for FakeSandbox {
    fn start(&mut self, req: &RunRequest<'_>) -> Result<(), SandboxError> {
        if self.fail_start {
            return Err(SandboxError::new("fake start failed"));
        }
        self.containers.push(FakeContainer {
            name: req.name.to_string(),
            group_dir: req.group_dir.to_path_buf(),
            task_id: req.task_id.to_string(),
            prompt: req.prompt.to_string(),
            phase: ContainerPhase::Running,
        });
        Ok(())
    }

    fn phase(&mut self, name: &str) -> Result<ContainerPhase, SandboxError> {
        Ok(self
            .containers
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.phase.clone())
            .unwrap_or(ContainerPhase::Missing))
    }

    fn remove(&mut self, name: &str) -> Result<(), SandboxError> {
        self.containers.retain(|c| c.name != name);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_run_argv_mounts_workspace_without_socket() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut sandbox = DockerSandbox::with_exec(Box::new(RecordingExec {
            calls: calls.clone(),
            code: 0,
            stdout: String::new(),
        }));
        let dir = Path::new("/var/openbots/groups/group-1");
        sandbox
            .start(&RunRequest {
                name: "openbots-task-1",
                image: "openbots-runner:local",
                group_dir: dir,
                task_id: "task-1",
                prompt: "画一张草图",
            })
            .unwrap();

        let argv = calls.lock().unwrap()[0].clone();
        assert_eq!(argv[0], "docker");
        assert!(argv.windows(2).any(|pair| pair == ["--network", "none"]));
        assert!(argv.iter().any(|arg| arg.ends_with(":/workspace")));
        assert!(argv.iter().all(|arg| !arg.contains("docker.sock")));
        assert!(!argv.iter().any(|arg| arg == "host" || arg == "--network=host"));
        assert!(argv.windows(2).any(|pair| pair == ["--name", "openbots-task-1"]));
        assert!(argv.iter().any(|arg| arg == "OPENBOTS_TASK_ID=task-1"));
        assert!(argv.iter().any(|arg| arg == "OPENBOTS_PROMPT=画一张草图"));
        assert_eq!(argv.last().map(String::as_str), Some("openbots-runner:local"));
    }
}
