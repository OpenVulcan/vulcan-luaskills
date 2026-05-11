use super::*;
use crate::runtime::encoding::default_runtime_text_encoding;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

/// Build one long-running process request used to verify drop-based cleanup.
/// 构建一个用于验证析构清理的长时间运行进程请求。
fn make_drop_cleanup_request() -> ProcessSessionOpenRequest {
    let encoding = default_runtime_text_encoding();
    if cfg!(windows) {
        ProcessSessionOpenRequest {
            program: "powershell".to_string(),
            args: vec![
                "-NoProfile".to_string(),
                "-Command".to_string(),
                "Start-Sleep -Seconds 30".to_string(),
            ],
            cwd: None,
            stdout_encoding: encoding,
            stderr_encoding: encoding,
            stdin_encoding: encoding,
            buffer_limit_bytes: DEFAULT_SESSION_BUFFER_LIMIT_BYTES,
        }
    } else {
        ProcessSessionOpenRequest {
            program: "sleep".to_string(),
            args: vec!["30".to_string()],
            cwd: None,
            stdout_encoding: encoding,
            stderr_encoding: encoding,
            stdin_encoding: encoding,
            buffer_limit_bytes: DEFAULT_SESSION_BUFFER_LIMIT_BYTES,
        }
    }
}

/// Build one process request whose direct child exits after spawning one descendant.
/// 构建一个直接子进程在拉起后代后立即退出的进程请求。
fn make_descendant_cleanup_request() -> ProcessSessionOpenRequest {
    let encoding = default_runtime_text_encoding();
    if cfg!(windows) {
        ProcessSessionOpenRequest {
                program: "python".to_string(),
                args: vec![
                    "-c".to_string(),
                    "import subprocess, sys, time; child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)']); print(child.pid, flush=True); time.sleep(0.3)".to_string(),
                ],
                cwd: None,
                stdout_encoding: encoding,
                stderr_encoding: encoding,
                stdin_encoding: encoding,
                buffer_limit_bytes: DEFAULT_SESSION_BUFFER_LIMIT_BYTES,
            }
    } else {
        ProcessSessionOpenRequest {
            program: "sh".to_string(),
            args: vec![
                "-c".to_string(),
                "sleep 30 & echo $!; sleep 0.3; exit 0".to_string(),
            ],
            cwd: None,
            stdout_encoding: encoding,
            stderr_encoding: encoding,
            stdin_encoding: encoding,
            buffer_limit_bytes: DEFAULT_SESSION_BUFFER_LIMIT_BYTES,
        }
    }
}

/// Build one process request whose direct child exits immediately.
/// 构建一个直接子进程会立即退出的进程请求。
fn make_immediate_exit_request() -> ProcessSessionOpenRequest {
    let encoding = default_runtime_text_encoding();
    if cfg!(windows) {
        ProcessSessionOpenRequest {
            program: "cmd".to_string(),
            args: vec!["/c".to_string(), "exit 0".to_string()],
            cwd: None,
            stdout_encoding: encoding,
            stderr_encoding: encoding,
            stdin_encoding: encoding,
            buffer_limit_bytes: DEFAULT_SESSION_BUFFER_LIMIT_BYTES,
        }
    } else {
        ProcessSessionOpenRequest {
            program: "sh".to_string(),
            args: vec!["-c".to_string(), "exit 0".to_string()],
            cwd: None,
            stdout_encoding: encoding,
            stderr_encoding: encoding,
            stdin_encoding: encoding,
            buffer_limit_bytes: DEFAULT_SESSION_BUFFER_LIMIT_BYTES,
        }
    }
}

/// Return whether the selected process id is still alive on the current platform.
/// 返回当前平台上指定进程 id 是否仍然存活。
fn process_exists(pid: u32) -> bool {
    if cfg!(windows) {
        Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-Command",
                    &format!(
                        "if (Get-Process -Id {} -ErrorAction SilentlyContinue) {{ exit 0 }} else {{ exit 1 }}",
                        pid
                    ),
                ])
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
    } else {
        Command::new("sh")
            .args(["-c", &format!("kill -0 {} 2>/dev/null", pid)])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}

/// Wait for one process id to disappear within the expected timeout.
/// 在预期超时时间内等待某个进程 id 消失。
fn assert_process_exits(pid: u32, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !process_exists(pid) {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("process {pid} should have exited after session drop");
}

/// Wait for one session to publish a descendant pid to stdout.
/// 等待某个会话把后代进程 pid 输出到 stdout。
fn wait_for_descendant_pid(session: &ManagedProcessSession, timeout: Duration) -> u32 {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        #[cfg(windows)]
        {
            let root_pid = session
                .state
                .child
                .lock()
                .expect("lock child process for descendant snapshot")
                .id();
            if let Ok(descendants) = collect_windows_descendant_processes(root_pid) {
                if let Some(descendant) = descendants.into_iter().map(|entry| entry.pid).next() {
                    return descendant;
                }
            }
        }
        let stdout = session
            .state
            .stdout_buffer
            .lock()
            .expect("lock stdout buffer");
        if !stdout.is_empty() {
            let pid_lines = stdout
                .iter()
                .filter_map(|byte| match byte {
                    b'0'..=b'9' => Some(char::from(*byte)),
                    b'\r' | b'\n' => Some('\n'),
                    _ => None,
                })
                .collect::<String>();
            drop(stdout);
            for pid_text in pid_lines
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
            {
                if let Ok(pid) = pid_text.parse::<u32>() {
                    return pid;
                }
            }
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("descendant pid should be published before cleanup");
}

/// Verify dropping the final session handle kills the child process.
/// 验证释放最后一个会话句柄时会杀掉子进程。
#[test]
fn dropping_process_session_kills_child_process() {
    let session = ManagedProcessSession::open(make_drop_cleanup_request())
        .expect("open drop cleanup session");
    let pid = session.state.child.lock().expect("lock child process").id();
    assert!(
        process_exists(pid),
        "child process should be running before drop"
    );

    drop(session);

    assert_process_exits(pid, Duration::from_secs(5));
}

/// Verify explicit teardown kills spawned descendants and releases reader threads promptly.
/// 验证显式清理会杀掉派生后代，并及时释放 reader 线程。
#[test]
fn killing_process_session_terminates_descendants_and_releases_readers() {
    let session = ManagedProcessSession::open(make_descendant_cleanup_request())
        .expect("open descendant cleanup session");
    let descendant_pid = wait_for_descendant_pid(&session, Duration::from_secs(15));
    assert!(
        process_exists(descendant_pid),
        "descendant process should be running before cleanup"
    );

    session
        .mark_closed("process.session.test")
        .expect("mark process session closed");
    let start = Instant::now();
    session
        .kill_child()
        .expect("kill descendant process tree cleanly");
    session
        .join_reader_threads("process.session.test")
        .expect("join process session readers");
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "process session cleanup should not block after tree termination"
    );

    assert_process_exits(descendant_pid, Duration::from_secs(5));
}

/// Verify explicit tree teardown becomes idempotent after the direct child is reaped once.
/// 验证显式进程树清理在直接子进程完成一次回收后会变成幂等操作。
#[test]
fn process_session_tree_teardown_is_idempotent_after_explicit_kill() {
    let session =
        ManagedProcessSession::open(make_drop_cleanup_request()).expect("open idempotent session");
    session
        .mark_closed("process.session.test")
        .expect("mark idempotent session closed");

    let first = session
        .kill_child()
        .expect("first process tree teardown should succeed");
    let second = session
        .kill_child()
        .expect("second process tree teardown should reuse cached final status");

    assert_eq!(first, second);
}

/// Verify reader timeout keeps the reader handle available for later retry.
/// 验证 reader 超时后仍保留句柄，方便后续重试清理。
#[test]
fn join_one_reader_timeout_preserves_reader_handle() {
    let (release_tx, release_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let done = Arc::new(AtomicBool::new(false));
    let done_flag = done.clone();
    let handle = thread::spawn(move || {
        release_rx.recv().expect("release test reader");
        done_flag.store(true, Ordering::Release);
        let _ = done_tx.send(());
    });
    let reader_slot = Mutex::new(Some(SessionPipeReader {
        handle,
        done_rx,
        done,
    }));

    let error = ManagedProcessSessionState::join_one_reader(&reader_slot, "test")
        .expect_err("reader join should time out before release");
    assert!(
        error.contains("timed out"),
        "timeout error should mention shutdown timeout, got: {error}"
    );
    assert!(
        reader_slot
            .lock()
            .expect("lock reader slot after timeout")
            .is_some(),
        "reader handle should stay available after timeout"
    );

    release_tx.send(()).expect("release test reader thread");
    ManagedProcessSessionState::join_one_reader(&reader_slot, "test")
        .expect("reader join should succeed after release");
    assert!(
        reader_slot
            .lock()
            .expect("lock reader slot after join")
            .is_none(),
        "reader handle should be removed after successful join"
    );
}

/// Verify close() keeps the child unreaped until tree cleanup completes.
/// 验证 close() 会在进程树清理完成前保持子进程未被提前 reap。
#[test]
fn closing_process_session_after_child_exit_still_cleans_descendants() {
    let lua = Lua::new();
    let session = ManagedProcessSession::open(make_descendant_cleanup_request())
        .expect("open close descendant cleanup session");
    let descendant_pid = wait_for_descendant_pid(&session, Duration::from_secs(15));
    assert!(
        process_exists(descendant_pid),
        "descendant process should be running before close cleanup"
    );

    let start = Instant::now();
    let status = session
        .close(&lua, MultiValue::new())
        .expect("close descendant cleanup session");
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "process.session.close should not block after descendant cleanup"
    );
    let exited: bool = status.get("exited").expect("read close exited flag");
    assert!(exited, "close should report one exited process status");
    assert_process_exits(descendant_pid, Duration::from_secs(5));
}

/// Verify read() keeps waiting for descendant output even after the root process exits.
/// 验证 read() 会在根进程退出后继续等待后代进程输出。
#[test]
fn read_waits_for_descendant_output_after_root_exit() {
    let lua = Lua::new();
    let session = ManagedProcessSession::open(make_immediate_exit_request())
        .expect("open immediate exit session");
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if session
            .state
            .peek_status_snapshot()
            .expect("peek immediate exit status")
            .exited
        {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        session
            .state
            .peek_status_snapshot()
            .expect("recheck immediate exit status")
            .exited,
        "immediate exit process should finish before read regression check"
    );
    session
        .state
        .join_reader_threads()
        .expect("join real readers before installing test readers");

    let install_test_reader = || -> (SessionPipeReader, mpsc::Sender<()>, Arc<AtomicBool>) {
        let (release_tx, release_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let done = Arc::new(AtomicBool::new(false));
        let done_flag = done.clone();
        let handle = thread::spawn(move || {
            release_rx.recv().expect("release synthetic session reader");
            done_flag.store(true, Ordering::Release);
            let _ = done_tx.send(());
        });
        (
            SessionPipeReader {
                handle,
                done_rx,
                done: done.clone(),
            },
            release_tx,
            done,
        )
    };
    let (stdout_reader, stdout_release_tx, _) = install_test_reader();
    let (stderr_reader, stderr_release_tx, _) = install_test_reader();
    *session
        .state
        .stdout_reader
        .lock()
        .expect("lock stdout reader slot for synthetic install") = Some(stdout_reader);
    *session
        .state
        .stderr_reader
        .lock()
        .expect("lock stderr reader slot for synthetic install") = Some(stderr_reader);

    let stdout_buffer = session.state.stdout_buffer.clone();
    let release_producer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(250));
        let mut buffer = stdout_buffer
            .lock()
            .expect("lock stdout buffer for synthetic descendant output");
        append_bounded(
            &mut buffer,
            b"child-ready\n",
            DEFAULT_SESSION_BUFFER_LIMIT_BYTES,
        );
        drop(buffer);
        stdout_release_tx
            .send(())
            .expect("release synthetic stdout reader");
        stderr_release_tx
            .send(())
            .expect("release synthetic stderr reader");
    });
    let options = lua.create_table().expect("create read options");
    options.set("timeout_ms", 3_000).expect("set read timeout");
    options
        .set("until_text", "child-ready")
        .expect("set read marker");

    let mut args = MultiValue::new();
    args.push_back(LuaValue::Table(options));
    let result = session.read(&lua, args).expect("read descendant output");
    let stdout: String = result.get("stdout").expect("read stdout text");
    let timed_out: bool = result.get("timed_out").expect("read timed_out flag");

    assert!(
        !timed_out,
        "read should finish from descendant output instead of timing out"
    );
    assert!(
        stdout.contains("child-ready"),
        "read should capture descendant output after root exit, got: {stdout:?}"
    );

    release_producer
        .join()
        .expect("join synthetic descendant output producer");
    session
        .state
        .join_reader_threads()
        .expect("join synthetic session readers");
}

#[cfg(windows)]
/// Verify snapshot-time identity filtering rejects processes created after the caller cutoff.
/// 验证快照时间身份过滤会拒绝截止时间之后才创建的进程。
#[test]
fn windows_snapshot_open_rejects_future_identity() {
    let handle = try_open_windows_process_for_snapshot(std::process::id(), 0)
        .expect("open current process for snapshot identity test");
    assert!(
        handle.is_none(),
        "process created after cutoff should be rejected to avoid PID reuse confusion"
    );
}

#[cfg(windows)]
/// Verify snapshot identity filtering still accepts one process that clearly predates the cutoff.
/// 验证快照身份过滤仍会接受那些明显早于截止时间创建的进程。
#[test]
fn windows_snapshot_open_accepts_existing_identity_before_cutoff() {
    let cutoff = current_windows_time_ticks().expect("capture current windows cutoff");
    let handle = try_open_windows_process_for_snapshot(std::process::id(), cutoff)
        .expect("open current process before cutoff");
    assert!(
        handle.is_some(),
        "existing process should be accepted when it predates the snapshot cutoff"
    );
}
