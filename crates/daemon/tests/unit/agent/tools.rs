use std::sync::Arc;
use std::time::Duration;
use super::*;

struct Fixture {
    _root: tempfile::TempDir,
    config: Config,
    settings: Settings,
}

impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let config = Config {
            bind: "127.0.0.1:0".parse().unwrap(), transfer_bind: "127.0.0.1:0".parse().unwrap(), transfer_bind_v6: None,
            token: Arc::from("isolated-tool-test"), settings_path: root.path().join("settings.json"),
            import_pi_dir: None, codex_auth_source: None, cwd: root.path().into(), database_path: root.path().join("tau.sqlite3"),
            telemetry_path: root.path().join("crashes.jsonl"), attachment_root: root.path().join("outbox"), upload_root: root.path().join("uploads"),
        };
        Self { _root: root, config, settings: Settings::default() }
    }

    async fn run(&self, command: &str, seconds: Option<u64>, cancel: &CancellationToken) -> Result<String> {
        let mut args = json!({"command": command});
        if let Some(seconds) = seconds { args["timeout"] = json!(seconds); }
        let result = tokio::time::timeout(Duration::from_secs(10), execute(
            &self.config, &self.settings, "bash", &args, cancel,
        )).await.expect("bash tool did not finish")?;
        Ok(result["content"][0]["text"].as_str().unwrap().to_owned())
    }

    async fn output(&self, command: &str) -> String {
        self.run(command, Some(5), &CancellationToken::new()).await.unwrap()
    }
}

#[tokio::test]
async fn script_keeps_stdin_null_and_supports_heredocs_and_pipelines() {
    let fixture = Fixture::new().await;
    let output = fixture.output("cat >/dev/null\nif read -r line; then exit 42; fi\nprintf 'after stdin\\n'\ncat <<'EOF' | tr a-z A-Z\nheredoc input\nEOF\nprintf 'last line'").await;
    assert_eq!(output, "after stdin\nHEREDOC INPUT\nlast line\n\nExit status: exit status: 0");
}

#[tokio::test]
async fn script_preserves_prefix_cwd_environment_and_file_mode_identity() {
    let mut fixture = Fixture::new().await;
    fixture.settings.agent.shell_command_prefix = "set -eu\nvalue='prefix value'\nprintf 'prefix\\n'".into();
    let output = fixture.output("printf '%s\\n' \"$value\" \"$PWD\" \"$0\" \"${BASH_SOURCE[0]}\" \"$#\" \"${TAU_TOKEN-unset}\" \"${TAU_FLAG_TOKEN-unset}\"\nprintf 'stderr\\n' >&2").await;
    assert_eq!(output, format!("prefix\nprefix value\n{}\n/dev/fd/3\n/dev/fd/3\n0\nunset\nunset\nstderr\n\nExit status: exit status: 0", fixture.config.cwd.display()));
}

#[tokio::test]
async fn script_preserves_exit_syntax_and_errexit_statuses() {
    let mut fixture = Fixture::new().await;
    assert!(fixture.output("").await.ends_with("Exit status: exit status: 0"));
    assert!(fixture.output("exit 7").await.ends_with("Exit status: exit status: 7"));
    assert!(fixture.output("if then").await.ends_with("Exit status: exit status: 2"));
    fixture.settings.agent.shell_command_prefix = "set -e".into();
    let output = fixture.output("false\nprintf 'unreachable\\n'").await;
    assert_eq!(output, "\n\nExit status: exit status: 1");
}

#[tokio::test]
async fn script_supports_configured_posix_shell() {
    let mut fixture = Fixture::new().await;
    fixture.settings.agent.shell_path = "/bin/sh".into();
    assert_eq!(fixture.output("cat >/dev/null\nprintf 'sh works'").await, "sh works\n\nExit status: exit status: 0");
}

#[tokio::test]
async fn script_is_not_limited_by_argv_string_size() {
    let fixture = Fixture::new().await;
    let command = format!("# {}\nprintf 'large script works'", "x".repeat(256 * 1024));
    assert_eq!(fixture.output(&command).await, "large script works\n\nExit status: exit status: 0");
}

#[tokio::test]
async fn script_transport_fd_is_closed_and_parent_remains_cloexec() {
    use std::os::fd::AsRawFd;
    let mut process = tokio::process::Command::new("/bin/bash");
    let script = shell_script(&mut process, "if ( : <&3 ) 2>/dev/null; then exit 41; fi\n/bin/sh -c 'if ( : <&3 ) 2>/dev/null; then exit 42; fi' || exit $?\nprintf 'closed'").unwrap();
    let flags = || unsafe { libc::fcntl(script.as_raw_fd(), libc::F_GETFD) };
    assert_eq!(flags() & libc::FD_CLOEXEC, libc::FD_CLOEXEC);
    let output = process.stdin(Stdio::null()).output().await.unwrap();
    assert!(output.status.success(), "{:?}", output);
    assert_eq!(output.stdout, b"closed");
    assert_eq!(flags() & libc::FD_CLOEXEC, libc::FD_CLOEXEC, "Child inheritance must not change the parent's descriptor flags");
}

#[test]
fn script_rejects_nul_instead_of_silently_changing_source() {
    let mut process = tokio::process::Command::new("/bin/bash");
    assert!(shell_script(&mut process, "printf first\0printf second").unwrap_err().to_string().contains("NUL"));
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn prefix_and_command_are_absent_from_process_arguments_and_pgrep() {
    let mut fixture = Fixture::new().await;
    let prefix_marker = format!("tau_prefix_{}", uuid::Uuid::new_v4().simple());
    let command_marker = format!("tau_command_{}", uuid::Uuid::new_v4().simple());
    fixture.settings.agent.shell_command_prefix = format!("# {prefix_marker}");
    let output = fixture.output(&format!("tr '\\0' '\\n' </proc/$$/cmdline\npgrep -f -- '{prefix_marker}|{command_marker}'\nstatus=$?\nprintf 'pgrep=%s\\n' \"$status\"")).await;
    assert_eq!(output, "/bin/bash\n/dev/fd/3\npgrep=1\n\nExit status: exit status: 0");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn pkill_full_match_kills_only_the_disposable_target_not_the_tool_shell() {
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    let fixture = Fixture::new().await;
    // Generated at runtime: no ancestor's argv contains this pattern. The only
    // intended signal recipient is this test-owned child, never a real service.
    let marker = format!("tau_pkill_{}", uuid::Uuid::new_v4().simple());
    let mut target = tokio::process::Command::new("/bin/sleep");
    target.as_std_mut().arg0(&marker);
    let mut target = target.arg("60").kill_on_drop(true).spawn().unwrap();
    let output = fixture.output(&format!("pkill -f -- '{marker}'\nprintf 'pkill=%s\\nsurvived\\n' \"$?\"")).await;
    assert_eq!(output, "pkill=0\nsurvived\n\nExit status: exit status: 0");
    let status = tokio::time::timeout(Duration::from_secs(2), target.wait()).await.unwrap().unwrap();
    assert_eq!(status.signal(), Some(libc::SIGTERM));
}

async fn wait_for_file(path: &Path) -> String {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(text) = tokio::fs::read_to_string(path).await && !text.is_empty() { return text; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("shell did not create its readiness file")
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn concurrent_tool_scripts_do_not_match_each_other() {
    let fixture = Fixture::new().await;
    let marker = format!("tau_sibling_{}", uuid::Uuid::new_v4().simple());
    let cancel = CancellationToken::new();
    let command = format!("# {marker}\nprintf ready >ready\nsleep 60\nprintf 'done'");
    let running = fixture.run(&command, None, &cancel);
    let inspecting = async {
        wait_for_file(&fixture.config.cwd.join("ready")).await;
        let output = fixture.output(&format!("pgrep -f -- '{marker}'\nprintf 'pgrep=%s\\n' \"$?\"")).await;
        cancel.cancel();
        output
    };
    let (running, inspecting) = tokio::join!(running, inspecting);
    assert_eq!(inspecting, "pgrep=1\n\nExit status: exit status: 0");
    assert!(running.unwrap().ends_with("Cancelled"));
}

#[cfg(target_os = "linux")]
async fn assert_processes_stopped(path: &Path) {
    let pids = tokio::fs::read_to_string(path).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let mut alive = false;
            for pid in pids.split_whitespace() {
                assert!(pid.parse::<u32>().is_ok());
                if let Ok(stat) = tokio::fs::read_to_string(format!("/proc/{pid}/stat")).await {
                    // An orphaned, killed child may briefly await reaping.
                    let state = stat.rsplit_once(") ").unwrap().1.as_bytes()[0];
                    alive |= state != b'Z' && state != b'X';
                }
            }
            if !alive { return; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("tool process group was left running");
}

#[cfg(target_os = "linux")]
const WAIT_WITH_CHILD: &str = "sleep 60 &\nprintf '%s %s\\n' \"$$\" \"$!\" >pids\nwait";

#[cfg(target_os = "linux")]
#[tokio::test]
async fn timeout_still_kills_the_shell_and_its_children() {
    let fixture = Fixture::new().await;
    let output = fixture.run(WAIT_WITH_CHILD, Some(1), &CancellationToken::new()).await.unwrap();
    assert!(output.ends_with("Timed out"));
    assert_processes_stopped(&fixture.config.cwd.join("pids")).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn cancellation_still_kills_the_shell_and_its_children() {
    let fixture = Fixture::new().await;
    let cancel = CancellationToken::new();
    let run = fixture.run(WAIT_WITH_CHILD, None, &cancel);
    let cancelling = async { wait_for_file(&fixture.config.cwd.join("pids")).await; cancel.cancel(); };
    let (output, ()) = tokio::join!(run, cancelling);
    assert!(output.unwrap().ends_with("Cancelled"));
    assert_processes_stopped(&fixture.config.cwd.join("pids")).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn dropping_the_tool_future_still_kills_its_process_group() {
    let fixture = Fixture::new().await;
    let cancel = CancellationToken::new();
    let mut run = Box::pin(fixture.run(WAIT_WITH_CHILD, None, &cancel));
    let path = fixture.config.cwd.join("pids");
    tokio::select! {
        result = &mut run => panic!("shell exited before cancellation: {result:?}"),
        _ = wait_for_file(&path) => {},
    }
    drop(run);
    assert_processes_stopped(&path).await;
}
