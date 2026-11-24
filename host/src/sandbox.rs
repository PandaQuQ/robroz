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
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerPhase {
    Running,
    Exited { code: i32 },
    Missing,
}

/// What `docker run` needs. The prompt is not here: the host writes it to
/// `{control_dir}/prompt.txt` (read-only `/control/prompt.txt`) before starting.
pub struct RunRequest<'a> {
    pub name: &'a str,
    pub image: &'a str,
    pub group_dir: &'a Path,
    /// This task's control dir. Mounted read-only at `/control`.
    pub control_dir: &'a Path,
    pub task_id: &'a str,
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
    /// `--user` value. None means docker's default (the image's USER).
    user: Option<String>,
}

impl DockerSandbox {
    /// Containers run as `ROBROZ_CONTAINER_USER` if set, else the host
    /// process's uid:gid (unix only), so volume files stay owned by the host user.
    pub fn new() -> Self {
        Self {
            exec: Box::new(SystemExec),
            user: container_user(std::env::var("ROBROZ_CONTAINER_USER").ok()),
        }
    }

    #[cfg(test)]
    fn with_exec(exec: Box<dyn CommandExec>, user: Option<String>) -> Self {
        Self { exec, user }
    }
}

impl Default for DockerSandbox {
    fn default() -> Self {
        Self::new()
    }
}

/// Explicit override wins; empty override falls back to the host user.
fn container_user(env_override: Option<String>) -> Option<String> {
    env_override
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(host_user)
}

#[cfg(unix)]
fn host_user() -> Option<String> {
    // SAFETY: getuid/getgid have no preconditions and cannot fail.
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    Some(format!("{uid}:{gid}"))
}

#[cfg(not(unix))]
fn host_user() -> Option<String> {
    None
}

pub fn container_name(task_id: &str) -> String {
    format!("robroz-{task_id}")
}

pub fn result_rel(task_id: &str) -> String {
    format!("tasks/{task_id}/result.txt")
}

fn run_argv(req: &RunRequest<'_>, user: Option<&str>) -> Vec<String> {
    let mut argv: Vec<String> = vec![
        "docker".into(),
        "run".into(),
        "-d".into(),
        "--name".into(),
        req.name.to_string(),
    ];
    if let Some(user) = user {
        argv.push("--user".into());
        argv.push(user.to_string());
    }
    argv.extend([
        "--network".into(),
        "none".into(),
        "-v".into(),
        format!("{}:/workspace", req.group_dir.display()),
        "-v".into(),
        format!("{}:/control:ro", req.control_dir.display()),
        "-e".into(),
        format!("ROBROZ_TASK_ID={}", req.task_id),
        req.image.to_string(),
    ]);
    argv
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
        let args = run_argv(req, self.user.as_deref());
        let output = self
            .exec
            .exec(&args)
            .map_err(|err| SandboxError::new(err.to_string()))?;
        if output.code != 0 {
            return Err(SandboxError::new(format!(
                "docker run exited {}",
                output.code
            )));
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
            return Err(SandboxError::new(format!(
                "docker rm exited {}",
                output.code
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) struct FakeContainer {
    pub name: String,
    pub group_dir: std::path::PathBuf,
    pub control_dir: std::path::PathBuf,
    pub task_id: String,
    /// Read from the control dir's `prompt.txt` at start, for assertions.
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

    /// Run the shared runner entry (same as the container), then mark exited.
    pub(crate) fn finish(&mut self, code: i32) {
        self.finish_inner(code, true);
    }

    /// Exit without writing volume files, for the missing-result case.
    pub(crate) fn finish_without_output(&mut self, code: i32) {
        self.finish_inner(code, false);
    }

    fn finish_inner(&mut self, code: i32, apply: bool) {
        for container in &mut self.containers {
            if apply && container.phase == ContainerPhase::Running {
                robroz_runner::run(
                    &container.group_dir,
                    &container.control_dir,
                    &container.task_id,
                )
                .expect("runner");
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
        assert!(req.control_dir.is_dir(), "control dir exists before start");
        let prompt = std::fs::read_to_string(robroz_runner::prompt_path(req.control_dir))
            .map_err(|err| SandboxError::new(format!("control prompt.txt missing: {err}")))?;
        assert!(
            !robroz_runner::task_dir(req.group_dir, req.task_id)
                .join("prompt.txt")
                .exists(),
            "prompt must not be written into the group volume"
        );
        self.containers.push(FakeContainer {
            name: req.name.to_string(),
            group_dir: req.group_dir.to_path_buf(),
            control_dir: req.control_dir.to_path_buf(),
            task_id: req.task_id.to_string(),
            prompt,
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

    fn recorded_run(user: Option<String>) -> Vec<String> {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut sandbox = DockerSandbox::with_exec(
            Box::new(RecordingExec {
                calls: calls.clone(),
                code: 0,
                stdout: String::new(),
            }),
            user,
        );
        sandbox
            .start(&RunRequest {
                name: "robroz-task-1",
                image: "robroz-runner:local",
                group_dir: Path::new("/var/robroz/groups/group-1"),
                control_dir: Path::new("/var/robroz/control/task-1"),
                task_id: "task-1",
            })
            .unwrap();
        let argv = calls.lock().unwrap()[0].clone();
        argv
    }

    #[test]
    fn docker_run_argv_mounts_workspace_without_socket() {
        let argv = recorded_run(Some("1000:1000".into()));
        assert_eq!(argv[0], "docker");
        assert!(argv.windows(2).any(|pair| pair == ["--network", "none"]));
        assert!(argv
            .windows(2)
            .any(|pair| pair == ["-v", "/var/robroz/groups/group-1:/workspace"]));
        assert!(argv.iter().all(|arg| !arg.contains("docker.sock")));
        assert!(!argv
            .iter()
            .any(|arg| arg == "host" || arg == "--network=host"));
        assert!(argv
            .windows(2)
            .any(|pair| pair == ["--name", "robroz-task-1"]));
        assert!(argv.iter().any(|arg| arg == "ROBROZ_TASK_ID=task-1"));
        assert_eq!(argv.last().map(String::as_str), Some("robroz-runner:local"));
    }

    #[test]
    fn docker_run_argv_sets_user_control_ro_and_no_prompt_env() {
        let argv = recorded_run(Some("1000:1000".into()));
        assert!(argv.windows(2).any(|pair| pair == ["--user", "1000:1000"]));
        assert!(argv
            .windows(2)
            .any(|pair| pair == ["-v", "/var/robroz/control/task-1:/control:ro"]));
        // Only the task's own control dir, and only read-only.
        let control_mounts: Vec<&String> = argv
            .iter()
            .filter(|arg| arg.contains(":/control"))
            .collect();
        assert_eq!(control_mounts.len(), 1);
        assert!(!argv.iter().any(|arg| arg.contains("ROBROZ_PROMPT")));
        let envs: Vec<&String> = argv
            .windows(2)
            .filter(|pair| pair[0] == "-e")
            .map(|pair| &pair[1])
            .collect();
        assert_eq!(envs, [&"ROBROZ_TASK_ID=task-1".to_string()]);
        // --user comes before the image, so docker treats it as an option.
        let user_at = argv.iter().position(|arg| arg == "--user").unwrap();
        assert!(user_at < argv.len() - 1);
    }

    #[test]
    fn docker_run_argv_omits_user_when_unset() {
        let argv = recorded_run(None);
        assert!(!argv.iter().any(|arg| arg == "--user"));
    }

    #[test]
    fn container_user_prefers_override_then_host() {
        assert_eq!(
            container_user(Some(" 1234:5678 ".into())),
            Some("1234:5678".into())
        );
        #[cfg(unix)]
        {
            let host = container_user(None).expect("unix host user");
            let mut parts = host.split(':');
            assert!(parts.next().unwrap().parse::<u32>().is_ok());
            assert!(parts.next().unwrap().parse::<u32>().is_ok());
            assert_eq!(container_user(Some("  ".into())), Some(host));
        }
        #[cfg(not(unix))]
        assert_eq!(container_user(None), None);
    }
}
