//! Embedded Neovim component for GPUI, rendered by [`gpui_ghostty`].

use std::{
    io::Read as _,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use gpui::{App, Context, Entity, IntoElement, Render, RenderImage, Task, Window};
use gpui_ghostty::{ClipboardApprovalCallback, Terminal, TerminalOptions};
use wait_timeout::ChildExt as _;

static NEXT_SOCKET_ID: AtomicU64 = AtomicU64::new(1);
const DEFAULT_REMOTE_TIMEOUT: Duration = Duration::from_secs(10);
const REMOTE_RETRY_INTERVAL: Duration = Duration::from_millis(25);
const REMOTE_POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_REMOTE_STDERR: usize = 64 * 1024;

/// Configuration for an embedded Neovim instance.
pub struct NvimOptions {
    pub project: PathBuf,
    pub initial_file: PathBuf,
    pub initial_line: Option<u64>,
    pub executable: PathBuf,
    pub remote_timeout: Duration,
    /// Approval policy for protected terminal clipboard operations.
    pub clipboard_approval: Option<ClipboardApprovalCallback>,
}

impl NvimOptions {
    pub fn new(project: impl Into<PathBuf>, initial_file: impl Into<PathBuf>) -> Self {
        Self {
            project: project.into(),
            initial_file: initial_file.into(),
            initial_line: None,
            executable: std::env::var_os("GPUI_NVIM")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("nvim")),
            remote_timeout: DEFAULT_REMOTE_TIMEOUT,
            clipboard_approval: None,
        }
    }
}

/// A Neovim process hosted inside a [`Terminal`].
pub struct NvimEditor {
    project: PathBuf,
    path: PathBuf,
    socket: PathBuf,
    executable: PathBuf,
    remote_timeout: Duration,
    terminal: Entity<Terminal>,
}

impl NvimEditor {
    pub fn spawn<T: 'static>(
        options: NvimOptions,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Result<Self, String> {
        let socket = socket_path();
        let command = nvim_command(
            &options.executable,
            &socket,
            &options.initial_file,
            options.initial_line,
        );
        let mut terminal_options = TerminalOptions::new(command, options.project.clone());
        terminal_options.clipboard_approval = options.clipboard_approval;
        let terminal = Terminal::spawn(terminal_options, window, cx)?;
        Ok(Self {
            project: options.project,
            path: options.initial_file,
            socket,
            executable: options.executable,
            remote_timeout: options.remote_timeout,
            terminal,
        })
    }

    pub fn project(&self) -> &Path {
        &self.project
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_alive(&self, cx: &App) -> bool {
        self.terminal.read(cx).is_alive()
    }

    pub fn focus<T>(&mut self, window: &mut Window, cx: &mut Context<T>) {
        self.terminal
            .update(cx, |terminal, cx| terminal.focus(window, cx));
    }

    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.terminal
            .update(cx, |terminal, _| terminal.set_visible(visible));
    }

    /// Captures Neovim's last completed native frame for temporary GPUI compositing.
    pub fn snapshot(&mut self, cx: &mut Context<Self>) -> Result<Arc<RenderImage>, String> {
        self.terminal.update(cx, |terminal, _| terminal.snapshot())
    }

    /// Opens `path` in the existing Neovim server without blocking the UI thread.
    pub fn open_file(&mut self, path: PathBuf, cx: &mut Context<Self>) -> Task<Result<(), String>> {
        self.open_file_at_line(path, None, cx)
    }

    /// Opens `path`, places the cursor at `line`, and completes when Neovim accepts the request.
    pub fn open_file_at_line(
        &mut self,
        path: PathBuf,
        line: Option<u64>,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        if !self.terminal.read(cx).is_alive() {
            return Task::ready(Err("the embedded Neovim process has exited".to_owned()));
        }

        let executable = self.executable.clone();
        let project = self.project.clone();
        let socket = self.socket.clone();
        let timeout = self.remote_timeout;
        let expression = open_file_expression(&path, line);
        let request = cx
            .background_executor()
            .spawn(async move { run_remote(&executable, &project, &socket, &expression, timeout) });

        cx.spawn(async move |editor, cx| {
            request.await?;
            editor
                .update(cx, |editor, _| editor.path = path)
                .map_err(|_| "the embedded Neovim editor was dropped".to_owned())?;
            Ok(())
        })
    }
}

fn run_remote(
    executable: &Path,
    project: &Path,
    socket: &Path,
    expression: &str,
    timeout: Duration,
) -> Result<(), String> {
    let started = Instant::now();
    let mut last_connection_error = String::new();
    loop {
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(format!(
                "connect to embedded Neovim at {} after {timeout:?}{}",
                socket.display(),
                error_detail(&last_connection_error)
            ));
        }

        let remote = remote_command(executable, project, socket, expression)
            .spawn()
            .map_err(|error| format!("contact embedded Neovim: {error}"))?;
        let (status, stderr) = wait_for_remote(remote, remaining)?;
        if status.success() {
            return Ok(());
        }
        if !server_unavailable(&stderr) {
            return Err(format!(
                "Neovim remote request exited with {status}{}",
                error_detail(&stderr)
            ));
        }
        last_connection_error = stderr;
        thread::sleep(REMOTE_RETRY_INTERVAL.min(remaining));
    }
}

fn wait_for_remote(mut remote: Child, timeout: Duration) -> Result<(ExitStatus, String), String> {
    let started = Instant::now();
    let mut stderr = Vec::new();
    let result = (|| {
        // Nonblocking reads avoid both a full-pipe deadlock and waiting forever
        // for EOF when a descendant inherits stderr from the remote process.
        if let Some(pipe) = remote.stderr.as_ref() {
            let flags = rustix::fs::fcntl_getfl(pipe)
                .map_err(|error| format!("read Neovim stderr flags: {error}"))?;
            rustix::fs::fcntl_setfl(pipe, flags | rustix::fs::OFlags::NONBLOCK)
                .map_err(|error| format!("set Neovim stderr nonblocking: {error}"))?;
        }
        loop {
            drain_remote_stderr(&mut remote, &mut stderr)?;
            if let Some(status) = remote
                .try_wait()
                .map_err(|error| format!("wait for embedded Neovim: {error}"))?
            {
                drain_remote_stderr(&mut remote, &mut stderr)?;
                return Ok((status, String::from_utf8_lossy(&stderr).into_owned()));
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(format!(
                    "embedded Neovim remote request timed out after {timeout:?}{}",
                    error_detail(&String::from_utf8_lossy(&stderr))
                ));
            }
            remote
                .wait_timeout(REMOTE_POLL_INTERVAL.min(remaining))
                .map_err(|error| format!("wait for embedded Neovim: {error}"))?;
        }
    })();
    if result.is_err() {
        // Also reap the child on setup/read/wait errors, not just timeout.
        let _ = remote.kill();
        let _ = remote.wait();
    }
    result
}

fn drain_remote_stderr(remote: &mut Child, captured: &mut Vec<u8>) -> Result<(), String> {
    let Some(pipe) = remote.stderr.as_mut() else {
        return Ok(());
    };
    let mut buffer = [0; 8192];
    // Bound work per poll even when the child continuously writes diagnostics.
    for _ in 0..MAX_REMOTE_STDERR / buffer.len() {
        match pipe.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                let keep = count.min(MAX_REMOTE_STDERR.saturating_sub(captured.len()));
                captured.extend_from_slice(&buffer[..keep]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("read embedded Neovim stderr: {error}")),
        }
    }
    Ok(())
}

fn server_unavailable(stderr: &str) -> bool {
    stderr.contains("E247:") || stderr.contains("Failed to connect")
}

fn error_detail(stderr: &str) -> String {
    let stderr = stderr.trim();
    if stderr.is_empty() {
        String::new()
    } else {
        format!(": {stderr}")
    }
}

fn remote_command(executable: &Path, project: &Path, socket: &Path, expression: &str) -> Command {
    let mut command = Command::new(executable);
    command
        .current_dir(project)
        .arg("--server")
        .arg(socket)
        .arg("--remote-expr")
        .arg(expression)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command
}

fn open_file_expression(path: &Path, line: Option<u64>) -> String {
    let path = path.to_string_lossy().replace('\'', "''");
    let edit = format!("execute('edit ' . fnameescape('{path}'))");
    match line {
        Some(line) => format!("[{edit}, cursor({line}, 1)]"),
        None => edit,
    }
}

impl Render for NvimEditor {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.terminal.clone()
    }
}

fn nvim_command(executable: &Path, socket: &Path, path: &Path, line: Option<u64>) -> String {
    let line = line.map_or_else(String::new, |line| format!(" +{line}"));
    format!(
        "{} --listen {}{} -- {}",
        shell_quote(executable),
        shell_quote(socket),
        line,
        shell_quote(path)
    )
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn socket_path() -> PathBuf {
    let id = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("gpui-neovim-{}-{id}.sock", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell_child(script: &str) -> Child {
        Command::new("sh")
            .args(["-c", script])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn test child")
    }

    #[test]
    fn remote_stderr_larger_than_a_pipe_is_drained_and_bounded() {
        let child = shell_child("head -c 1048576 /dev/zero >&2");
        let (status, stderr) =
            wait_for_remote(child, Duration::from_secs(5)).expect("large stderr must not deadlock");
        assert!(status.success());
        assert_eq!(stderr.len(), MAX_REMOTE_STDERR);
    }

    #[test]
    fn remote_timeout_kills_and_reaps_the_child() {
        let child = shell_child("while :; do :; done");
        let pid = child.id();
        let started = Instant::now();
        let error =
            wait_for_remote(child, Duration::from_millis(50)).expect_err("child should time out");
        assert!(error.contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(
            !Command::new("kill")
                .args(["-0", &pid.to_string()])
                .stderr(Stdio::null())
                .status()
                .expect("probe child")
                .success()
        );
    }

    #[test]
    fn inherited_stderr_does_not_extend_the_deadline() {
        let started = Instant::now();
        let child = shell_child("sleep 1 & exit 0");
        let (status, _) = wait_for_remote(child, Duration::from_millis(100))
            .expect("must not wait for descendant EOF");
        assert!(status.success());
        assert!(started.elapsed() < Duration::from_millis(800));
    }

    #[test]
    fn command_quotes_paths_at_the_ghostty_boundary() {
        assert_eq!(
            nvim_command(
                Path::new("/tmp/my nvim"),
                Path::new("/tmp/editor.sock"),
                Path::new("/tmp/it's.rs"),
                Some(42),
            ),
            "'/tmp/my nvim' --listen '/tmp/editor.sock' +42 -- '/tmp/it'\\''s.rs'"
        );
    }

    #[test]
    fn remote_commands_use_an_expression_instead_of_remote_fallback() {
        let command = remote_command(
            Path::new("/tmp/my nvim"),
            Path::new("/tmp/project"),
            Path::new("/tmp/editor.sock"),
            "cursor(42, 1)",
        );

        assert_eq!(command.get_current_dir(), Some(Path::new("/tmp/project")));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "--server",
                "/tmp/editor.sock",
                "--remote-expr",
                "cursor(42, 1)"
            ]
        );
    }

    #[test]
    fn open_expression_escapes_paths_and_moves_the_cursor_atomically() {
        assert_eq!(
            open_file_expression(Path::new("/tmp/it's.rs"), Some(42)),
            "[execute('edit ' . fnameescape('/tmp/it''s.rs')), cursor(42, 1)]"
        );
        assert_eq!(
            open_file_expression(Path::new("/tmp/plain.rs"), None),
            "execute('edit ' . fnameescape('/tmp/plain.rs'))"
        );
    }

    #[test]
    fn sockets_are_unique_within_the_process() {
        assert_ne!(socket_path(), socket_path());
    }
}
