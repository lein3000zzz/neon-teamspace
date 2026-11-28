//! Serving as an unprivileged user where the Docker socket lets one use it.
//!
//! The image starts the updater as root, as no user of its own can know the group the host
//! gives the socket. `neon-updater serve` then looks at the socket and, where its group may
//! read and write it, starts itself again as the unprivileged user [`UID`] in that group,
//! with no other groups, before it reads a request. Holding the socket is root on the host
//! either way; the process that parses requests just holds nothing else.

/// The unprivileged user serve runs as: distroless images' `nonroot`.
pub const UID: u32 = 65532;

/// That user's home in the image, for the docker CLI's own files.
pub const HOME: &str = "/home/neon-updater";

/// The socket's owner, group and permission bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Socket {
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
}

/// The user and group to serve as, given the socket: [`UID`] in its group where that group
/// may read and write it; its owner where it is not root, as rootless Docker's socket is;
/// otherwise `None`, to stay as it is.
#[must_use]
pub fn unprivileged(socket: Socket) -> Option<(u32, u32)> {
    if socket.mode & 0o060 == 0o060 {
        Some((UID, socket.gid))
    } else if socket.uid != 0 && socket.mode & 0o600 == 0o600 {
        Some((socket.uid, socket.gid))
    } else {
        None
    }
}

/// When running as root on Linux, starts this program again with the same arguments as the
/// user [`unprivileged`] picks, and never returns but with the error that kept it from it.
/// Otherwise, and where the socket allows no other user, returns at once.
///
/// # Errors
///
/// When the program cannot be started again.
#[cfg(target_os = "linux")]
pub fn shed_root() -> anyhow::Result<()> {
    use std::os::unix::fs::MetadataExt as _;
    use std::os::unix::process::CommandExt as _;

    use anyhow::Context as _;

    // The owner of /proc/self is the process's effective user.
    let root = std::fs::metadata("/proc/self").is_ok_and(|me| me.uid() == 0);
    let Ok(socket) = std::fs::metadata(crate::docker::SOCKET) else {
        return Ok(());
    };
    if !root {
        return Ok(());
    }
    let socket = Socket {
        uid: socket.uid(),
        gid: socket.gid(),
        mode: socket.mode(),
    };
    let Some((uid, gid)) = unprivileged(socket) else {
        return Ok(());
    };
    let home = if std::path::Path::new(HOME).is_dir() {
        HOME
    } else {
        "/tmp"
    };
    let program = std::env::current_exe().context("finding the updater's own binary")?;
    // Sets the group, drops every other group, then sets the user: std does it in that order.
    let error = std::process::Command::new(program)
        .args(std::env::args_os().skip(1))
        .uid(uid)
        .gid(gid)
        .env("HOME", home)
        .exec();
    Err(error).with_context(|| format!("serving as the user {uid} in the group {gid}"))
}

/// Elsewhere the updater runs as it was started.
///
/// # Errors
///
/// Never.
#[cfg(not(target_os = "linux"))]
pub fn shed_root() -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_socket_its_group_may_use_is_used_from_that_group() {
        // Docker Engine on Linux: root:docker 660.
        let docker = Socket {
            uid: 0,
            gid: 998,
            mode: 0o140_660,
        };
        assert_eq!(unprivileged(docker), Some((UID, 998)));
        // Docker Desktop: root:root 660.
        let desktop = Socket { gid: 0, ..docker };
        assert_eq!(unprivileged(desktop), Some((UID, 0)));
    }

    #[test]
    fn a_rootless_socket_is_used_as_its_owner() {
        let rootless = Socket {
            uid: 1000,
            gid: 1000,
            mode: 0o140_600,
        };
        assert_eq!(unprivileged(rootless), Some((1000, 1000)));
    }

    #[test]
    fn a_socket_only_root_may_use_keeps_root() {
        let only_root = Socket {
            uid: 0,
            gid: 0,
            mode: 0o140_600,
        };
        assert_eq!(unprivileged(only_root), None);
        let read_only_group = Socket {
            gid: 998,
            mode: 0o140_640,
            ..only_root
        };
        assert_eq!(unprivileged(read_only_group), None);
    }
}
