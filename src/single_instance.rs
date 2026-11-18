use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::{self, Read, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use directories::ProjectDirs;
use interprocess::{
    ConnectWaitMode,
    local_socket::{ConnectOptions, GenericNamespaced, ListenerOptions, Stream, prelude::*},
};
use serde::{Deserialize, Serialize};

const RETRY_INTERVAL: Duration = Duration::from_millis(40);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Deserialize, Serialize)]
struct Endpoint {
    pid: u32,
    socket: String,
}

pub struct Instance {
    _lock: File,
    socket: String,
    stopping: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Instance {
    /// A secondary process returns None once the primary has accepted its activation request.
    pub fn acquire(activate: impl Fn() + Send + 'static) -> io::Result<Option<Self>> {
        let directories = ProjectDirs::from("", "", "Screacher Radio")
            .ok_or_else(|| io::Error::other("Application data directory not found"))?;
        Self::acquire_in(directories.data_local_dir(), activate)
    }

    fn acquire_in(
        directory: &Path,
        activate: impl Fn() + Send + 'static,
    ) -> io::Result<Option<Self>> {
        fs::create_dir_all(directory)?;
        // Keep this inode in place. Removing a lock file can allow two processes to lock different files.
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join("instance.lock"))?;
        let endpoint_path = directory.join("instance.json");
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        loop {
            match lock.try_lock() {
                Ok(()) => return Self::start(lock, &endpoint_path, activate).map(Some),
                Err(TryLockError::Error(error)) => return Err(error),
                Err(TryLockError::WouldBlock) => {}
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "The running instance did not respond",
                ));
            }
            // The primary may still be publishing its endpoint, or may have just exited.
            if let Ok(bytes) = fs::read(&endpoint_path)
                && let Ok(endpoint) = serde_json::from_slice::<Endpoint>(&bytes)
                && activate_existing(&endpoint, deadline).is_ok()
            {
                return Ok(None);
            }
            thread::sleep(RETRY_INTERVAL);
        }
    }

    fn start(
        lock: File,
        endpoint_path: &Path,
        activate: impl Fn() + Send + 'static,
    ) -> io::Result<Self> {
        let pid = std::process::id();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let socket = format!("screacher-radio-{pid}-{timestamp}");
        let listener = ListenerOptions::new()
            .name(socket.as_str().to_ns_name::<GenericNamespaced>()?)
            .create_sync()?;
        let endpoint = Endpoint {
            pid,
            socket: socket.clone(),
        };
        fs::write(endpoint_path, serde_json::to_vec(&endpoint)?)?;
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = stopping.clone();
        let worker = thread::Builder::new()
            .name("instance-activation".into())
            .spawn(move || {
                loop {
                    match listener.accept() {
                        Ok(mut connection) => {
                            if stop.load(Ordering::Acquire) {
                                break;
                            }
                            activate();
                            let _ = connection.write_all(b"1");
                        }
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
            })?;
        Ok(Self {
            _lock: lock,
            socket,
            stopping,
            worker: Some(worker),
        })
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        // Wake the blocking accept before releasing the lock and allowing the next primary.
        let wakeup = connect(&self.socket);
        if wakeup.is_ok()
            && let Some(worker) = self.worker.take()
        {
            let _ = worker.join();
        }
    }
}

fn connect(socket: &str) -> io::Result<Stream> {
    ConnectOptions::new()
        .name(socket.to_ns_name::<GenericNamespaced>()?)
        .wait_mode(ConnectWaitMode::Timeout(RETRY_INTERVAL))
        .nonblocking_stream(true)
        .connect_sync()
}

fn activate_existing(endpoint: &Endpoint, deadline: Instant) -> io::Result<()> {
    if endpoint
        .socket
        .strip_prefix(&format!("screacher-radio-{}-", endpoint.pid))
        .and_then(|timestamp| timestamp.parse::<u128>().ok())
        .is_none()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid instance endpoint",
        ));
    }
    #[cfg(windows)]
    // The newly launched process can pass its foreground permission to the running player.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(endpoint.pid);
    }
    let mut connection = connect(&endpoint.socket)?;
    let deadline = deadline.min(Instant::now() + Duration::from_millis(500));
    let mut acknowledgement = [0];
    loop {
        match connection.read(&mut acknowledgement) {
            Ok(1) if acknowledgement == *b"1" => return Ok(()),
            Ok(0) if cfg!(windows) && Instant::now() < deadline => {
                thread::sleep(RETRY_INTERVAL);
            }
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Activation was not acknowledged",
                ));
            }
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(RETRY_INTERVAL);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        process::{Command, Stdio},
        sync::{Barrier, atomic::AtomicUsize, mpsc},
    };

    use super::*;

    #[test]
    fn secondary_activates_primary_and_a_later_launch_can_take_over() {
        let directory = tempfile::tempdir().unwrap();
        let (activated, requests) = mpsc::channel();
        let primary = Instance::acquire_in(directory.path(), move || {
            activated.send(()).unwrap();
        })
        .unwrap()
        .unwrap();
        assert!(
            Instance::acquire_in(directory.path(), || panic!("unexpected primary"))
                .unwrap()
                .is_none()
        );
        requests.recv_timeout(Duration::from_secs(1)).unwrap();
        drop(primary);
        let replacement = Instance::acquire_in(directory.path(), || {}).unwrap();
        assert!(replacement.is_some());
    }

    #[test]
    fn simultaneous_launches_elect_one_primary() {
        let directory = tempfile::tempdir().unwrap();
        let start = Barrier::new(9);
        let finished = Barrier::new(9);
        let release = Barrier::new(9);
        let primaries = AtomicUsize::new(0);
        let activations = Arc::new(AtomicUsize::new(0));
        thread::scope(|scope| {
            for _ in 0..8 {
                let activations = activations.clone();
                let (directory, start, finished, release, primaries) =
                    (directory.path(), &start, &finished, &release, &primaries);
                scope.spawn(move || {
                    start.wait();
                    let instance = Instance::acquire_in(directory, move || {
                        activations.fetch_add(1, Ordering::Relaxed);
                    });
                    if matches!(instance, Ok(Some(_))) {
                        primaries.fetch_add(1, Ordering::Relaxed);
                    }
                    finished.wait();
                    release.wait();
                    drop(instance.unwrap());
                });
            }
            start.wait();
            finished.wait();
            let primary_count = primaries.load(Ordering::Relaxed);
            let activation_count = activations.load(Ordering::Relaxed);
            release.wait();
            assert_eq!(primary_count, 1);
            assert_eq!(activation_count, 7);
        });
    }

    #[test]
    fn different_users_data_directories_are_independent() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let one = Instance::acquire_in(first.path(), || {}).unwrap();
        let two = Instance::acquire_in(second.path(), || {}).unwrap();
        assert!(one.is_some() && two.is_some());
    }

    #[test]
    fn an_aborted_process_releases_the_lock() {
        let directory = tempfile::tempdir().unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "single_instance::tests::process_probe",
                "--ignored",
            ])
            .env("SCREACHER_INSTANCE_TEST_DIR", directory.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !directory.path().join("ready").exists() {
            if Instant::now() >= deadline || child.try_wait().unwrap().is_some() {
                let _ = child.kill();
                let _ = child.wait();
                panic!("instance test process did not become ready");
            }
            thread::sleep(RETRY_INTERVAL);
        }
        child.kill().unwrap();
        child.wait().unwrap();
        let replacement = Instance::acquire_in(directory.path(), || {})
            .unwrap()
            .unwrap();
        assert!(
            Instance::acquire_in(directory.path(), || {})
                .unwrap()
                .is_none()
        );
        drop(replacement);
    }

    #[test]
    #[ignore = "helper process started by an_aborted_process_releases_the_lock"]
    fn process_probe() {
        let Some(directory) = std::env::var_os("SCREACHER_INSTANCE_TEST_DIR") else {
            return;
        };
        let directory = std::path::PathBuf::from(directory);
        let _instance = Instance::acquire_in(&directory, || {}).unwrap().unwrap();
        fs::write(directory.join("ready"), b"ready").unwrap();
        loop {
            thread::park();
        }
    }
}
