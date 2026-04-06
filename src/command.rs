use std::{
    io,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

const COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(25);
const MAX_OUTPUT_CHARS: usize = 400;

#[derive(Debug, Clone)]
pub struct OneShotCommand {
    program: String,
    args: Vec<String>,
    context: &'static str,
    timeout: Duration,
}

impl OneShotCommand {
    pub fn new<I, S>(
        program: impl Into<String>,
        args: I,
        context: &'static str,
        timeout: Duration,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            context,
            timeout,
        }
    }

    pub fn run_blocking(&self) -> Result<Output, String> {
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| self.spawn_error(&err))?;

        let deadline = Instant::now() + self.timeout;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => {
                    return child
                        .wait_with_output()
                        .map_err(|err| self.wait_error(&err));
                }
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(self.timeout_error());
                    }
                    thread::sleep(COMMAND_POLL_INTERVAL);
                }
                Err(err) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(self.wait_error(&err));
                }
            }
        }
    }

    pub async fn run(&self) -> Result<Output, String> {
        let task = self.clone();
        let display = self.display();
        let context = self.context;

        tokio::task::spawn_blocking(move || task.run_blocking())
            .await
            .map_err(|err| {
                format!("Failed to join command task for `{display}` while {context}: {err}")
            })?
    }

    pub fn ensure_success(&self, output: Output) -> Result<Output, String> {
        if output.status.success() {
            Ok(output)
        } else {
            Err(self.status_error(&output))
        }
    }

    pub fn status_error(&self, output: &Output) -> String {
        let status = match output.status.code() {
            Some(code) => format!("exit code {code}"),
            None => "termination by signal".to_string(),
        };
        match render_output_details(&output.stdout, &output.stderr) {
            Some(details) => format!(
                "`{}` failed while {} with {}: {}",
                self.display(),
                self.context,
                status,
                details
            ),
            None => format!(
                "`{}` failed while {} with {}",
                self.display(),
                self.context,
                status
            ),
        }
    }

    pub fn display(&self) -> String {
        let mut parts = Vec::with_capacity(self.args.len() + 1);
        parts.push(render_arg(&self.program));
        parts.extend(self.args.iter().map(|arg| render_arg(arg)));
        parts.join(" ")
    }

    fn spawn_error(&self, err: &io::Error) -> String {
        format!(
            "Failed to start `{}` while {}: {}",
            self.display(),
            self.context,
            err
        )
    }

    fn wait_error(&self, err: &io::Error) -> String {
        format!(
            "Failed while waiting for `{}` while {}: {}",
            self.display(),
            self.context,
            err
        )
    }

    fn timeout_error(&self) -> String {
        format!(
            "Timed out after {} while {} with `{}`",
            render_duration(self.timeout),
            self.context,
            self.display()
        )
    }
}

fn render_arg(arg: &str) -> String {
    if arg.is_empty()
        || arg
            .chars()
            .any(|ch| ch.is_whitespace() || matches!(ch, '"' | '\''))
    {
        format!("{arg:?}")
    } else {
        arg.to_string()
    }
}

fn render_duration(duration: Duration) -> String {
    if duration.subsec_nanos() == 0 {
        format!("{}s", duration.as_secs())
    } else if duration.as_millis() > 0 {
        format!("{}ms", duration.as_millis())
    } else {
        format!("{}ns", duration.as_nanos())
    }
}

fn render_output_details(stdout: &[u8], stderr: &[u8]) -> Option<String> {
    match (trim_output(stdout), trim_output(stderr)) {
        (Some(stdout), Some(stderr)) => Some(format!("stdout: {stdout}; stderr: {stderr}")),
        (Some(stdout), None) => Some(format!("stdout: {stdout}")),
        (None, Some(stderr)) => Some(format!("stderr: {stderr}")),
        (None, None) => None,
    }
}

fn trim_output(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes).trim().to_string();
    if text.is_empty() {
        return None;
    }

    let mut compact = String::with_capacity(text.len());
    for (idx, line) in text.lines().enumerate() {
        if idx > 0 {
            compact.push_str(" | ");
        }
        compact.push_str(line.trim());
    }

    let compact_len = compact.chars().count();
    if compact_len > MAX_OUTPUT_CHARS {
        let truncated: String = compact.chars().take(MAX_OUTPUT_CHARS).collect();
        Some(format!("{truncated}…"))
    } else {
        Some(compact)
    }
}

#[cfg(test)]
mod tests {
    use super::{OneShotCommand, render_duration, render_output_details};

    use std::time::Duration;
    #[cfg(unix)]
    use std::{os::unix::process::ExitStatusExt, process::Output};

    #[test]
    fn renders_timeout_errors_with_context() {
        let command = OneShotCommand::new(
            "adb",
            ["devices", "-l"],
            "listing connected Android devices",
            Duration::from_secs(5),
        );

        let err = command.run_blocking_timeout_for_test();

        assert_eq!(
            err,
            "Timed out after 5s while listing connected Android devices with `adb devices -l`"
        );
    }

    #[test]
    fn renders_output_details_from_stdout_and_stderr() {
        assert_eq!(
            render_output_details(b"hello\nworld\n", b"warning"),
            Some("stdout: hello | world; stderr: warning".to_string())
        );
    }

    #[cfg(unix)]
    #[test]
    fn renders_status_errors_with_exit_details() {
        let command = OneShotCommand::new(
            "xcrun",
            ["simctl", "list", "devices", "--json"],
            "listing available iOS simulators",
            Duration::from_secs(5),
        );
        let output = Output {
            status: std::process::ExitStatus::from_raw(1 << 8),
            stdout: Vec::new(),
            stderr: b"simctl unavailable".to_vec(),
        };

        assert_eq!(
            command.status_error(&output),
            "`xcrun simctl list devices --json` failed while listing available iOS simulators with exit code 1: stderr: simctl unavailable"
        );
    }

    #[test]
    fn renders_subsecond_durations_in_millis() {
        assert_eq!(render_duration(Duration::from_millis(250)), "250ms");
    }

    impl OneShotCommand {
        fn run_blocking_timeout_for_test(&self) -> String {
            self.timeout_error()
        }
    }
}
