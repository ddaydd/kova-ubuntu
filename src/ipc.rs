use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc;

fn socket_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{}/kova.sock", uid))
}

/// Try to send a directory path to an already-running Kova instance.
/// Returns true if successfully sent (caller should exit).
pub fn try_send(dir: &str) -> bool {
    let path = socket_path();
    if let Ok(mut stream) = UnixStream::connect(&path) {
        if writeln!(stream, "{}", dir).is_ok() {
            return true;
        }
    }
    false
}

/// Start listening for incoming project-open requests.
/// Returns a receiver that yields directory paths.
pub fn start_listener() -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    let path = socket_path();

    // Clean up stale socket
    let _ = std::fs::remove_file(&path);

    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            log::warn!("Failed to bind IPC socket {}: {}", path.display(), e);
            return rx;
        }
    };

    log::info!("IPC listening on {}", path.display());

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let reader = BufReader::new(stream);
                    for line in reader.lines() {
                        if let Ok(dir) = line {
                            let dir = dir.trim().to_string();
                            if !dir.is_empty() {
                                log::info!("IPC received: {}", dir);
                                let _ = tx.send(dir);
                            }
                        }
                    }
                }
                Err(e) => log::warn!("IPC accept error: {}", e),
            }
        }
    });

    rx
}

/// Clean up the socket file on shutdown.
pub fn cleanup() {
    let _ = std::fs::remove_file(socket_path());
}
