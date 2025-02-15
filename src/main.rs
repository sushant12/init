use base64::{engine::general_purpose, Engine as _};
use core::net::Ipv4Addr;
use futures::TryStreamExt;
use log::{info, LevelFilter};
use nix::mount::{mount, MsFlags};
use nix::sys::stat::Mode;
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::{chdir, chroot, mkdir, sethostname};
use rtnetlink::new_connection;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs::write;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, Write};
use std::net::IpAddr;
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

#[derive(Deserialize, Debug)]
struct FileConfig {
    guest_path: String,
    raw_value: String,
}

#[derive(Deserialize, Debug)]
struct RunConfig {
    files: Vec<FileConfig>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let log_level = match env::var("RUST_LOG") {
        Ok(level) if level.to_lowercase() == "debug" => LevelFilter::Debug,
        _ => LevelFilter::Info,
    };
    env_logger::builder().filter_level(log_level).init();

    let file = File::open("/fly/run.json")?;
    let reader = BufReader::new(file);
    let run_config: RunConfig = serde_json::from_reader(reader)?;
    info!("Run configuration: {:?}", run_config);

    info!("Creating /dev directory...");
    mkdir("/dev", Mode::S_IRWXU)?;

    info!("Mounting devtmpfs inside /dev...");
    mount(
        Some("devtmpfs"),
        "/dev",
        Some("devtmpfs"),
        MsFlags::empty(),
        None::<&str>,
    )?;

    info!("Creating /newroot directory...");
    mkdir("/newroot", Mode::S_IRWXU)?;

    info!("Mounting the root filesystem...");
    mount(
        Some("/dev/vdb"),
        "/newroot",
        Some("ext4"),
        MsFlags::empty(),
        None::<&str>,
    )?;

    // Move /dev so we don't have to re-mount it
    info!("Mounting (move) /dev");
    mkdir("/newroot/dev", Mode::S_IRWXU).ok();
    mount::<_, _, [u8], [u8]>(Some("/dev"), "/newroot/dev", None, MsFlags::MS_MOVE, None)?;

    info!("Switching the root filesystem...");
    chdir("/newroot")?;
    mount::<_, _, [u8], [u8]>(Some("."), "/", None, MsFlags::MS_MOVE, None)?;
    // Change root to the current directory (new root)
    chroot(".")?;
    chdir("/")?;
    for file_config in run_config.files {
        let decoded_data = general_purpose::STANDARD.decode(&file_config.raw_value)?;
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&file_config.guest_path)?;
        file.write_all(&decoded_data)?;
        info!("Saved file: {}", file_config.guest_path);
    }

    // let output = Command::new("cat").arg("file1.txt").output().await?;
    // info!(
    //     "Directory listing:\n{}",
    //     String::from_utf8_lossy(&output.stdout)
    // );
    info!("Creating /etc directory...");
    mkdir("/etc", Mode::S_IRWXU).ok();

    info!("Creating /etc/resolv.conf for DNS resolution...");
    write("/etc/resolv.conf", "nameserver 8.8.8.8\n")?;

    info!("Creating /etc/hosts for local network resolution...");
    write("/etc/hosts", "127.0.0.1 localhost\n")?;
    info!("Setting hostname...");
    match sethostname("hostname-1") {
        Err(e) => info!("error setting hostname: {}", e),
        Ok(_) => {}
    };
    configure_networking().await?;

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

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
    }

    // Ok(())
}

async fn configure_networking() -> Result<(), Box<dyn std::error::Error>> {
    let (connection, handle, _) = new_connection().unwrap();
    tokio::spawn(connection);

    info!("netlink: getting lo link");
    let lo = handle
        .link()
        .get()
        .match_name("lo".into())
        .execute()
        .try_next()
        .await?
        .expect("no lo link found");

    info!("netlink: setting lo link \"up\"");
    handle.link().set(lo.header.index).up().execute().await?;

    info!("netlink: getting eth0 link");
    let eth0 = handle
        .link()
        .get()
        .match_name("eth0".into())
        .execute()
        .try_next()
        .await?
        .expect("no eth0 link found");

    info!("netlink: setting eth0 link \"up\"");
    handle
        .link()
        .set(eth0.header.index)
        .up()
        .mtu(1420)
        .execute()
        .await?;

    let ip_address: IpAddr = "172.16.0.2".parse()?;
    let gateway: Ipv4Addr = "172.16.0.1".parse()?;
    info!("netlink: adding IP address to eth0");
    handle
        .address()
        .add(eth0.header.index, ip_address, 24)
        .execute()
        .await?;

    info!("netlink: adding default route via gateway");
    handle.route().add().v4().gateway(gateway).execute().await?;

    Ok(())
}

async fn handle_exec(req: ExecRequest) -> Result<impl warp::Reply, warp::Rejection> {
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
