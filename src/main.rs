use log::{debug, info, LevelFilter};
use nix::mount::{mount, MsFlags};
use nix::sys::stat::Mode;
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::{chdir, chroot, mkdir};
use serde::{Deserialize, Serialize};
use std::env;
use tokio::process::Command;
use tokio::signal::unix::{signal, SignalKind};
use tokio_vsock::{VsockAddr, VsockListener};
use warp::Filter;

#[derive(Deserialize, Debug)]
struct ExecRequest {
    cmd: Vec<String>,
}

#[derive(Serialize)]
struct ExecResponse {
    output: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    let log_level = match env::var("RUST_LOG") {
        Ok(level) if level.to_lowercase() == "debug" => LevelFilter::Debug,
        _ => LevelFilter::Info,
    };
    env_logger::builder().filter_level(log_level).init();

    // Example logs
    info!("This is an info log.");
    debug!("This is a debug log.");

    // Create /dev directory
    info!("Creating /dev directory...");
    mkdir("/dev", Mode::S_IRWXU)?;

    // Mount devtmpfs inside /dev
    info!("Mounting devtmpfs inside /dev...");
    mount(
        Some("devtmpfs"),
        "/dev",
        Some("devtmpfs"),
        MsFlags::empty(),
        None::<&str>,
    )?;

    // Create /newroot directory for new root filesystem
    info!("Creating /newroot directory...");
    mkdir("/newroot", Mode::S_IRWXU)?;

    // Mount the root filesystem
    info!("Mounting the root filesystem...");
    mount(
        Some("/dev/vdb"),
        "/newroot",
        Some("ext4"), // Specify the filesystem type
        MsFlags::empty(),
        None::<&str>,
    )?;

    // Move /dev so we don't have to re-mount it
    info!("Mounting (move) /dev");
    mkdir("/newroot/dev", Mode::S_IRWXU).ok();
    mount::<_, _, [u8], [u8]>(Some("/dev"), "/newroot/dev", None, MsFlags::MS_MOVE, None)?;

    // Switch the root filesystem
    info!("Switching the root filesystem...");
    // Change directory to the new root
    chdir("/newroot")?;
    // Mount the new root over /
    mount::<_, _, [u8], [u8]>(Some("."), "/", None, MsFlags::MS_MOVE, None)?;
    // Change root to the current directory (new root)
    chroot(".")?;
    // Change directory to /
    chdir("/")?;

    // Start the vsock listener
    let listener = VsockListener::bind(VsockAddr::new(3, 10000))?;
    info!("Listening on vsock CID 3, port 10000");

    let routes = warp::path("v1")
        .and(warp::path("exec"))
        .and(warp::post())
        .and(warp::body::json())
        .and_then(handle_exec);

    tokio::spawn(async move {
        warp::serve(routes).run_incoming(listener.incoming()).await;
    });

    // Spawn a task to reap zombie processes
    tokio::spawn(async {
        let mut sigchld = signal(SignalKind::child()).expect("Failed to create signal handler");
        loop {
            sigchld.recv().await;
            while let Ok(WaitStatus::Exited(pid, _)) = waitpid(None, None) {
                info!("Reaped zombie process with PID: {}", pid);
            }
        }
    });

    // Keep the init process running
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
    }

    // Ok(())
}

async fn handle_exec(req: ExecRequest) -> Result<impl warp::Reply, warp::Rejection> {
    // Log the received request
    info!("Received request: {:?}", req);

    let output = if req.cmd.len() > 0 {
        let mut cmd = Command::new(&req.cmd[0]);
        if req.cmd.len() > 1 {
            cmd.args(&req.cmd[1..]);
        }
        match cmd.output().await {
            Ok(output) => String::from_utf8_lossy(&output.stdout).to_string(),
            Err(e) => format!("Failed to execute command: {}", e),
        }
    } else {
        "No command provided".to_string()
    };

    let response = ExecResponse { output };
    Ok(warp::reply::json(&response))
}
