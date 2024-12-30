use log::{debug, info};
use nix::mount::{mount as nix_mount, MsFlags};
use nix::sys::stat::Mode;
use nix::unistd::{chdir as nix_chdir, chroot as nix_chroot, mkdir as nix_mkdir, symlinkat};
use nix::NixPath;
use std::env;

#[derive(Debug, thiserror::Error)]
enum InitError {
    #[error("couldn't mkdir {}, because: {}", path, error)]
    Mkdir {
        path: String,
        #[source]
        error: nix::Error,
    },

    #[error("couldn't mount {} onto {}, because: {}", source, target, error)]
    Mount {
        source: String,
        target: String,
        #[source]
        error: nix::Error,
    },

    #[error("couldn't chdir to {}, because: {}", path, error)]
    Chdir {
        path: String,
        #[source]
        error: nix::Error,
    },

    #[error("couldn't chroot to {}, because: {}", path, error)]
    Chroot {
        path: String,
        #[source]
        error: nix::Error,
    },
}

pub fn log_init() {
    // default to "info" level, just for this bin
    let level = env::var("LOG_FILTER").unwrap_or_else(|_| "init=info".into());

    env_logger::builder()
        .parse_filters(&level)
        .write_style(env_logger::WriteStyle::Never)
        .format_level(false)
        .format_module_path(false)
        .format_timestamp(None)
        .init();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    log_init();
    let chmod_0755: Mode =
        Mode::S_IRWXU | Mode::S_IRGRP | Mode::S_IXGRP | Mode::S_IROTH | Mode::S_IXOTH;
    let chmod_0555: Mode = Mode::S_IRUSR
        | Mode::S_IXUSR
        | Mode::S_IRGRP
        | Mode::S_IXGRP
        | Mode::S_IROTH
        | Mode::S_IXOTH;
    let chmod_1777: Mode = Mode::S_IRWXU | Mode::S_IRWXG | Mode::S_IRWXO | Mode::S_ISVTX;
    // let chmod_0777 = Mode::S_IRWXU | Mode::S_IRWXG | Mode::S_IRWXO;
    let common_mnt_flags: MsFlags = MsFlags::MS_NODEV | MsFlags::MS_NOEXEC | MsFlags::MS_NOSUID;
    info!("Starting init...");
    debug!("Mounting /dev");
    mkdir("/dev", chmod_0755).ok();
    mount(
        Some("devtmpfs"),
        "/dev",
        Some("devtmpfs"),
        MsFlags::MS_NOSUID,
        Some("mode=0755"),
    )?;

    mkdir("/newroot", chmod_0755)?;
    let root_device = "/dev/vdb".to_owned();
    debug!("Mounting newroot fs");
    mount::<_, _, _, [u8]>(
        Some(root_device.as_str()),
        "/newroot",
        Some("ext4"),
        MsFlags::MS_RELATIME,
        None,
    )?;

    // Move /dev so we don't have to re-mount it
    debug!("Mounting (move) /dev");
    mkdir("/newroot/dev", chmod_0755).ok();
    mount::<_, _, [u8], [u8]>(Some("/dev"), "/newroot/dev", None, MsFlags::MS_MOVE, None)?;

    // Our own hacky switch_root
    debug!("Switching root");
    // Change directory to the new root
    chdir("/newroot")?;
    // Mount the new root over /
    mount::<_, _, [u8], [u8]>(Some("."), "/", None, MsFlags::MS_MOVE, None)?;
    // Change root to the current directory (new root)
    chroot(".")?;
    // Change directory to /
    chdir("/")?;

    debug!("Mounting /dev/pts");
    mkdir("/dev/pts", chmod_0755).ok();
    mount(
        Some("devpts"),
        "/dev/pts",
        Some("devpts"),
        MsFlags::MS_NOEXEC | MsFlags::MS_NOSUID | MsFlags::MS_NOATIME,
        Some("mode=0620,gid=5,ptmxmode=666"),
    )?;

    debug!("Mounting /dev/mqueue");
    mkdir("/dev/mqueue", chmod_0755).ok();
    mount::<_, _, _, [u8]>(
        Some("mqueue"),
        "/dev/mqueue",
        Some("mqueue"),
        common_mnt_flags,
        None,
    )?;

    debug!("Mounting /dev/shm");
    mkdir("/dev/shm", chmod_1777).ok();
    mount::<_, _, _, [u8]>(
        Some("shm"),
        "/dev/shm",
        Some("tmpfs"),
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        None,
    )?;

    debug!("Mounting /dev/hugepages");
    mkdir("/dev/hugepages", chmod_0755).ok();
    mount(
        Some("hugetlbfs"),
        "/dev/hugepages",
        Some("hugetlbfs"),
        MsFlags::MS_RELATIME,
        Some("pagesize=2M"),
    )?;

    debug!("Mounting /proc");
    mkdir("/proc", chmod_0555).ok();
    mount::<_, _, _, [u8]>(Some("proc"), "/proc", Some("proc"), common_mnt_flags, None)?;
    mount::<_, _, _, [u8]>(
        Some("binfmt_misc"),
        "/proc/sys/fs/binfmt_misc",
        Some("binfmt_misc"),
        common_mnt_flags | MsFlags::MS_RELATIME,
        None,
    )?;

    debug!("Mounting /sys");
    mkdir("/sys", chmod_0555).ok();
    mount::<_, _, _, [u8]>(Some("sys"), "/sys", Some("sysfs"), common_mnt_flags, None)?;

    debug!("Mounting /run");
    mkdir("/run", chmod_0755).ok();
    mount(
        Some("run"),
        "/run",
        Some("tmpfs"),
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        Some("mode=0755"),
    )?;
    mkdir("/run/lock", Mode::all()).ok();

    symlinkat("/proc/self/fd", None, "/dev/fd").ok();
    symlinkat("/proc/self/fd/0", None, "/dev/stdin").ok();
    symlinkat("/proc/self/fd/1", None, "/dev/stdout").ok();
    symlinkat("/proc/self/fd/2", None, "/dev/stderr").ok();

    mkdir("/root", Mode::S_IRWXU).ok();

    let common_cgroup_mnt_flags =
        MsFlags::MS_NODEV | MsFlags::MS_NOEXEC | MsFlags::MS_NOSUID | MsFlags::MS_RELATIME;

    debug!("Mounting cgroup");
    mount(
        Some("tmpfs"),
        "/sys/fs/cgroup",
        Some("tmpfs"),
        MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV, // | MsFlags::MS_RDONLY,
        Some("mode=755"),
    )?;

    debug!("Mounting cgroup2");
    mkdir("/sys/fs/cgroup/unified", chmod_0555)?;
    mount(
        Some("cgroup2"),
        "/sys/fs/cgroup/unified",
        Some("cgroup2"),
        common_mnt_flags | MsFlags::MS_RELATIME,
        Some("nsdelegate"),
    )?;

    debug!("Mounting /sys/fs/cgroup/net_cls,net_prio");
    mkdir("/sys/fs/cgroup/net_cls,net_prio", chmod_0555)?;
    mount(
        Some("cgroup"),
        "/sys/fs/cgroup/net_cls,net_prio",
        Some("cgroup"),
        common_cgroup_mnt_flags,
        Some("net_cls,net_prio"),
    )?;

    debug!("Mounting /sys/fs/cgroup/hugetlb");
    mkdir("/sys/fs/cgroup/hugetlb", chmod_0555)?;
    mount(
        Some("cgroup"),
        "/sys/fs/cgroup/hugetlb",
        Some("cgroup"),
        common_cgroup_mnt_flags,
        Some("hugetlb"),
    )?;

    debug!("Mounting /sys/fs/cgroup/pids");
    mkdir("/sys/fs/cgroup/pids", chmod_0555)?;
    mount(
        Some("cgroup"),
        "/sys/fs/cgroup/pids",
        Some("cgroup"),
        common_cgroup_mnt_flags,
        Some("pids"),
    )?;

    debug!("Mounting /sys/fs/cgroup/freezer");
    mkdir("/sys/fs/cgroup/freezer", chmod_0555)?;
    mount(
        Some("cgroup"),
        "/sys/fs/cgroup/freezer",
        Some("cgroup"),
        common_cgroup_mnt_flags,
        Some("freezer"),
    )?;

    debug!("Mounting /sys/fs/cgroup/cpu,cpuacct");
    mkdir("/sys/fs/cgroup/cpu,cpuacct", chmod_0555)?;
    mount(
        Some("cgroup"),
        "/sys/fs/cgroup/cpu,cpuacct",
        Some("cgroup"),
        common_cgroup_mnt_flags,
        Some("cpu,cpuacct"),
    )?;

    debug!("Mounting /sys/fs/cgroup/devices");
    mkdir("/sys/fs/cgroup/devices", chmod_0555)?;
    mount(
        Some("cgroup"),
        "/sys/fs/cgroup/devices",
        Some("cgroup"),
        common_cgroup_mnt_flags,
        Some("devices"),
    )?;

    debug!("Mounting /sys/fs/cgroup/blkio");
    mkdir("/sys/fs/cgroup/blkio", chmod_0555)?;
    mount(
        Some("cgroup"),
        "/sys/fs/cgroup/blkio",
        Some("cgroup"),
        common_cgroup_mnt_flags,
        Some("blkio"),
    )?;

    debug!("Mounting cgroup/memory");
    mkdir("/sys/fs/cgroup/memory", chmod_0555)?;
    mount(
        Some("cgroup"),
        "/sys/fs/cgroup/memory",
        Some("cgroup"),
        common_cgroup_mnt_flags,
        Some("memory"),
    )?;

    debug!("Mounting /sys/fs/cgroup/perf_event");
    mkdir("/sys/fs/cgroup/perf_event", chmod_0555)?;
    mount(
        Some("cgroup"),
        "/sys/fs/cgroup/perf_event",
        Some("cgroup"),
        common_cgroup_mnt_flags,
        Some("perf_event"),
    )?;

    debug!("Mounting /sys/fs/cgroup/cpuset");
    mkdir("/sys/fs/cgroup/cpuset", chmod_0555)?;
    mount(
        Some("cgroup"),
        "/sys/fs/cgroup/cpuset",
        Some("cgroup"),
        common_cgroup_mnt_flags,
        Some("cpuset"),
    )?;
    Ok(())
}

fn mkdir<P: ?Sized + NixPath>(path: &P, mode: Mode) -> Result<(), InitError> {
    nix_mkdir(path, mode).map_err(|error| InitError::Mkdir {
        path: path
            .with_nix_path(|cs| {
                cs.to_owned()
                    .into_string()
                    .ok()
                    .unwrap_or_else(|| String::new())
            })
            .unwrap_or_else(|_| String::new()),
        error,
    })
}

fn mount<P1: ?Sized + NixPath, P2: ?Sized + NixPath, P3: ?Sized + NixPath, P4: ?Sized + NixPath>(
    source: Option<&P1>,
    target: &P2,
    fstype: Option<&P3>,
    flags: MsFlags,
    data: Option<&P4>,
) -> Result<(), InitError> {
    nix_mount(source, target, fstype, flags, data).map_err(|error| InitError::Mount {
        source: source
            .map(|p| {
                p.with_nix_path(|cs| {
                    cs.to_owned()
                        .into_string()
                        .ok()
                        .unwrap_or_else(|| String::new())
                })
                .unwrap_or_else(|_| String::new())
            })
            .unwrap_or_else(|| String::new()),
        target: target
            .with_nix_path(|cs| {
                cs.to_owned()
                    .into_string()
                    .ok()
                    .unwrap_or_else(|| String::new())
            })
            .unwrap_or_else(|_| String::new()),
        error,
    })
}

fn chdir<P: ?Sized + NixPath>(path: &P) -> Result<(), InitError> {
    nix_chdir(path).map_err(|error| InitError::Chdir {
        path: path
            .with_nix_path(|cs| {
                cs.to_owned()
                    .into_string()
                    .ok()
                    .unwrap_or_else(|| String::new())
            })
            .unwrap_or_else(|_| String::new()),
        error,
    })
}

fn chroot<P: ?Sized + NixPath>(path: &P) -> Result<(), InitError> {
    nix_chroot(path).map_err(|error| InitError::Chroot {
        path: path
            .with_nix_path(|cs| {
                cs.to_owned()
                    .into_string()
                    .ok()
                    .unwrap_or_else(|| String::new())
            })
            .unwrap_or_else(|_| String::new()),
        error,
    })
}
