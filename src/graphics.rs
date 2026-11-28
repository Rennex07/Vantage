#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Auto,
    Full,
    Safe,
}

impl Profile {
    pub fn from_environment() -> Self {
        match std::env::var("VANTAGE_GRAPHICS").as_deref() {
            Ok("full") => Self::Full,
            Ok("safe") => Self::Safe,
            Ok("auto") | Err(std::env::VarError::NotPresent) => Self::Auto,
            _ => {
                eprintln!("Ignoring invalid VANTAGE_GRAPHICS (expected auto, full or safe)");
                Self::Auto
            }
        }
    }

    pub fn safe(self, driver: &str, renderer: &str) -> bool {
        match self {
            Self::Safe => true,
            Self::Full => false,
            Self::Auto => {
                let renderer = renderer.to_ascii_lowercase();
                (driver == "vmwgfx" && renderer.contains("unknown"))
                    || ["llvmpipe", "softpipe", "swrast", "software rasterizer"]
                        .iter()
                        .any(|name| renderer.contains(name))
            }
        }
    }
}

/// Work around GTK4 rendering defects on VMware-compatible virtual graphics
/// without disabling the compositor's independent GLES acceleration.
pub fn configure_client(command: &mut std::process::Command) {
    if std::env::var_os("GSK_RENDERER").is_some() {
        return;
    }
    let virtual_gpu = std::fs::read_dir("/sys/class/drm")
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .any(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.strip_prefix("card")
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                && std::fs::read_link(entry.path().join("device/driver"))
                    .ok()
                    .is_some_and(|driver| driver.file_name().is_some_and(|name| name == "vmwgfx"))
        });
    if virtual_gpu {
        command.env("GSK_RENDERER", "cairo");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_profile_handles_virtual_and_software_renderers() {
        assert!(!Profile::Auto.safe("vmwgfx", "SVGA3D"));
        assert!(Profile::Auto.safe("vmwgfx", "unknown GLES renderer"));
        assert!(Profile::Auto.safe("vmwgfx", "llvmpipe"));
        assert!(Profile::Auto.safe("virtio_gpu", "Mesa LLVMpipe"));
        assert!(!Profile::Auto.safe("i915", "Mesa Intel"));
        assert!(!Profile::Full.safe("vmwgfx", "llvmpipe"));
        assert!(Profile::Safe.safe("i915", "Mesa Intel"));
    }
}
