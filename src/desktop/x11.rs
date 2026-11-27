use std::{
    env,
    ffi::OsStr,
    fs::{self, File},
    io::{self, Read, Write},
    os::{
        fd::OwnedFd,
        unix::{fs::MetadataExt, net::UnixListener},
    },
    path::PathBuf,
    process::{Child, Command, Stdio},
};

pub struct X11 {
    display: String,
    authority: tempfile::NamedTempFile,
    socket: PathBuf,
    socket_inode: u64,
    satellite: Option<Child>,
}

impl X11 {
    pub fn start(wayland: &OsStr) -> io::Result<Self> {
        let runtime = env::var_os("XDG_RUNTIME_DIR")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR"))?;
        let authority = tempfile::NamedTempFile::new_in(runtime)?;
        let mut cookie = [0; 16];
        File::open("/dev/urandom")?.read_exact(&mut cookie)?;

        // Reserve the listening socket before exporting DISPLAY. Clients can connect
        // during startup and Xwayland receives every connection from the same socket.
        let (number, socket, listener) = (1..=63)
            .filter(|n| !PathBuf::from(format!("/tmp/.X{n}-lock")).exists())
            .find_map(|number| {
                let socket = PathBuf::from(format!("/tmp/.X11-unix/X{number}"));
                UnixListener::bind(&socket)
                    .ok()
                    .map(|listener| (number, socket, listener))
            })
            .ok_or_else(|| io::Error::new(io::ErrorKind::AddrNotAvailable, "No free X11 socket"))?;
        let socket_inode = fs::symlink_metadata(&socket)?.ino();
        let display = number.to_string();
        // Xauthority family Wild matches any local Unix transport for this display.
        let mut x11 = Self {
            display: format!(":{number}"),
            authority,
            socket,
            socket_inode,
            satellite: None,
        };
        x11.authority.write_all(&u16::MAX.to_be_bytes())?;
        for field in [
            b"".as_slice(),
            display.as_bytes(),
            b"MIT-MAGIC-COOKIE-1",
            &cookie,
        ] {
            let length = u16::try_from(field.len()).expect("small Xauthority field");
            x11.authority.write_all(&length.to_be_bytes())?;
            x11.authority.write_all(field)?;
        }

        let fd: OwnedFd = listener.into();
        x11.satellite = Some(
            Command::new("xwayland-satellite")
                .arg(&x11.display)
                .args(["-listenfd", "0", "-nolisten", "tcp", "-auth"])
                .arg(x11.authority.path())
                .env("WAYLAND_DISPLAY", wayland)
                .env_remove("WAYLAND_SOCKET")
                .stdin(Stdio::from(fd))
                .spawn()?,
        );
        Ok(x11)
    }

    pub fn configure(&self, command: &mut Command) {
        command
            .env("DISPLAY", &self.display)
            .env("XAUTHORITY", self.authority.path());
    }
}

impl Drop for X11 {
    fn drop(&mut self) {
        if let Some(child) = &mut self.satellite {
            let _ = child.kill();
            let _ = child.wait();
        }
        if fs::symlink_metadata(&self.socket)
            .is_ok_and(|metadata| metadata.ino() == self.socket_inode)
        {
            let _ = fs::remove_file(&self.socket);
        }
    }
}
