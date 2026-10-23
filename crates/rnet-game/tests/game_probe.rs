//! Real child-process contract tests for the independent game probe.
#![cfg(target_os = "linux")]

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[path = "game_probe/faults.rs"]
mod faults;

fn binary() -> PathBuf {
    std::env::var_os("RNET_GAME_PROBE_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_game_probe")))
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "rnet-probe-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn keys(&self) -> PathBuf {
        let path = self.0.join("keys");
        let output = Command::new(binary())
            .args(["keygen", "--dir"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "keygen: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Process {
    child: Child,
    log: PathBuf,
}

impl Process {
    fn start(command: &mut Command, directory: &Path, name: &str) -> Self {
        let log = directory.join(format!("{name}.log"));
        let file = File::create(&log).unwrap();
        Self {
            child: command
                .stdin(Stdio::null())
                .stdout(file.try_clone().unwrap())
                .stderr(file)
                .spawn()
                .unwrap(),
            log,
        }
    }

    fn text(&self) -> String {
        fs::read_to_string(&self.log).unwrap()
    }

    fn wait(&mut self, success: bool) -> String {
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                let text = self.text();
                assert_eq!(status.success(), success, "unexpected child exit: {text}");
                return text;
            }
            assert!(Instant::now() < deadline, "child deadline: {}", self.text());
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn address(&mut self) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(line) = self
                .text()
                .lines()
                .find(|line| line.starts_with("status=listening "))
            {
                return line
                    .split_whitespace()
                    .find_map(|field| field.strip_prefix("address="))
                    .unwrap()
                    .to_owned();
            }
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "server exited: {}",
                self.text()
            );
            assert!(
                Instant::now() < deadline,
                "server readiness timeout: {}",
                self.text()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn resources_available() -> bool {
    let available = fs::read_to_string("/proc/meminfo")
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("MemAvailable:").map(str::to_owned))
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    if available < 19 * 1024 * 1024 {
        eprintln!("SKIPPED game probe smoke: startup needs 19 GiB available");
        return false;
    }
    true
}

fn server(keys: &Path, transport: &str) -> Command {
    let mut command = Command::new(binary());
    command
        .args([
            "server",
            "--transport",
            transport,
            "--bind",
            "127.0.0.1:0",
            "--run-id",
            "73",
            "--duration-ms",
            "3000",
            "--drain-ms",
            "1000",
            "--connections",
            "8",
            "--payload-bytes",
            "64",
            "--report-ms",
            "200",
            "--key-file",
        ])
        .arg(keys.join("server.key"))
        .arg("--token-file")
        .arg(keys.join("token"));
    command
}

fn client(keys: &Path, transport: &str, address: &str, generator: &str) -> Command {
    let port = address.rsplit_once(':').unwrap().1;
    let mut command = Command::new(binary());
    command
        .args([
            "client",
            "--transport",
            transport,
            "--host",
            "localhost",
            "--port",
            port,
            "--run-id",
            "73",
            "--generator-id",
            generator,
            "--connections",
            "2",
            "--duration-ms",
            "300",
            "--drain-ms",
            "1000",
            "--join-timeout-ms",
            "1500",
            "--payload-bytes",
            "64",
            "--rate",
            "50",
            "--pin-file",
        ])
        .arg(keys.join("server.pub"))
        .arg("--token-file")
        .arg(keys.join("token"));
    command
}

fn count(line: &str, key: &str) -> u64 {
    let prefix = format!("{key}=");
    line.split_whitespace()
        .find_map(|part| part.strip_prefix(&prefix))
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn independent_probe_rejects_bad_arguments_and_never_overwrites_credentials() {
    let help = Command::new(binary()).arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("--pin-file"));
    let fixture = Fixture::new();
    let keys = fixture.keys();
    let before = fs::read(keys.join("server.key")).unwrap();
    assert!(!Command::new(binary())
        .args(["keygen", "--dir"])
        .arg(&keys)
        .output()
        .unwrap()
        .status
        .success());
    assert_eq!(fs::read(keys.join("server.key")).unwrap(), before);
    for args in [
        vec!["server"],
        vec!["client", "--duration-ms", "0"],
        vec!["client", "--unknown", "1"],
    ] {
        assert!(!Command::new(binary())
            .args(args)
            .output()
            .unwrap()
            .status
            .success());
    }
}

#[test]
fn independent_processes_cover_all_transports_and_generator_isolation() {
    if !resources_available() {
        return;
    }
    let fixture = Fixture::new();
    let keys = fixture.keys();
    for transport in ["tcp", "udp", "kcp"] {
        let mut server = Process::start(
            &mut server(&keys, transport),
            &fixture.0,
            &format!("server-{transport}"),
        );
        let address = server.address();
        let mut a = Process::start(
            &mut client(&keys, transport, &address, "11"),
            &fixture.0,
            &format!("a-{transport}"),
        );
        let mut b = Process::start(
            &mut client(&keys, transport, &address, "12"),
            &fixture.0,
            &format!("b-{transport}"),
        );
        let mut total = 0;
        for child in [&mut a, &mut b] {
            let text = child.wait(true);
            let line = text
                .lines()
                .find(|line| line.starts_with("status=completed role=client "))
                .unwrap();
            let admitted = count(line, "admitted");
            assert!(admitted > 0, "{text}");
            assert_eq!(count(line, "received"), admitted, "{text}");
            assert_eq!(count(line, "offered"), admitted + count(line, "rejected"));
            assert_eq!(count(line, "finished_flows"), 2);
            assert!(line.contains("slo=not_evaluated") && line.contains("rtt_p99_us="));
            total += admitted;
        }
        let text = server.wait(true);
        let line = text
            .lines()
            .find(|line| line.starts_with("status=completed role=server "))
            .unwrap();
        assert_eq!(count(line, "received"), total, "{text}");
        assert_eq!(count(line, "echoed"), total, "{text}");
        assert_eq!(count(line, "finished_flows"), 4);
    }
}

#[test]
fn independent_client_fails_wrong_pin_and_wrong_credential() {
    if !resources_available() {
        return;
    }
    let fixture = Fixture::new();
    let keys = fixture.keys();
    let mut server = Process::start(&mut server(&keys, "tcp"), &fixture.0, "server");
    let address = server.address();
    let original = fs::read(keys.join("server.pub")).unwrap();
    fs::write(keys.join("server.pub"), [0_u8; 32]).unwrap();
    let mut wrong = Process::start(
        &mut client(&keys, "tcp", &address, "11"),
        &fixture.0,
        "wrong-pin",
    );
    let text = wrong.wait(false);
    assert!(
        text.contains("status=failed") && text.contains("join"),
        "{text}"
    );
    fs::write(keys.join("server.pub"), original).unwrap();
    fs::write(keys.join("token"), [0x91; 32]).unwrap();
    let mut wrong = Process::start(
        &mut client(&keys, "tcp", &address, "12"),
        &fixture.0,
        "wrong-token",
    );
    let text = wrong.wait(false);
    assert!(
        text.contains("status=failed") && text.contains("join"),
        "{text}"
    );
}
