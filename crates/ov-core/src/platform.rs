use crate::fs_util::atomic_write_with_mode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Spawn a subprocess without flashing a CMD console window on Windows.
#[allow(unused_mut)]
pub fn silent_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WslDistroInfo {
    pub name: String,
    pub home_unc: Option<PathBuf>,
}

/// Unified cross-platform path resolver for OpenViking Desktop and supported local Agents.
#[derive(Debug, Clone)]
pub struct PlatformPaths {
    pub home: PathBuf,
}

impl PlatformPaths {
    pub fn detect() -> Option<Self> {
        let home = dirs::home_dir()?;
        Some(Self { home })
    }

    pub fn openviking_dir(&self) -> PathBuf {
        self.home.join(".openviking")
    }

    pub fn ovcli_conf(&self) -> PathBuf {
        self.openviking_dir().join("ovcli.conf")
    }

    pub fn profiles_conf(&self) -> PathBuf {
        self.openviking_dir().join("profiles.json")
    }

    pub fn policy_toml(&self) -> PathBuf {
        self.openviking_dir().join("policy.toml")
    }

    pub fn desktop_db(&self) -> PathBuf {
        self.openviking_dir().join("desktop.db")
    }

    pub fn bridge_binary(&self) -> PathBuf {
        let bin_name = if cfg!(target_os = "windows") {
            "ov-bridge.exe"
        } else {
            "ov-bridge"
        };
        self.openviking_dir().join("bin").join(bin_name)
    }

    /// Ensure the bundled `ov-bridge` sidecar binary is installed into `~/.openviking/bin/`
    /// with executable permissions on Unix. Returns an error if neither a bundled source nor
    /// an already-installed binary exists.
    pub fn ensure_bridge_installed(&self, explicit_source: Option<&Path>) -> Result<PathBuf, String> {
        let dest = self.bridge_binary();
        let source = explicit_source
            .map(PathBuf::from)
            .or_else(find_bundled_bridge_binary);

        let Some(src) = source.filter(|p| p.is_file()) else {
            if dest.is_file() {
                return Ok(dest);
            }
            return Err(format!(
                "ov-bridge binary not found (expected bundled sidecar or existing {})",
                dest.display()
            ));
        };

        let source_bytes = fs::read(&src).map_err(|e| e.to_string())?;
        let same_hash = match fs::read(&dest) {
            Ok(bytes) => Sha256::digest(&source_bytes) == Sha256::digest(&bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(e.to_string()),
        };

        if !same_hash {
            atomic_write_with_mode(&dest, &source_bytes, Some(0o755))
                .map_err(|e| e.to_string())?;
        } else {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = fs::metadata(&dest).map_err(|e| e.to_string())?.permissions().mode();
                if mode & 0o7777 != 0o755 {
                    fs::set_permissions(&dest, fs::Permissions::from_mode(0o755))
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        Ok(dest)
    }

    /// Resolve a CLI executable across system PATH, Login Shell PATH (macOS/Linux GUI apps),
    /// and common user installation directories.
    pub fn resolve_binary(&self, name: &str) -> Option<PathBuf> {
        if let Ok(path) = which::which(name) {
            return Some(path);
        }

        for dir in self.extended_search_dirs() {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
            #[cfg(target_os = "windows")]
            {
                for ext in ["exe", "cmd", "bat"] {
                    let with_ext = dir.join(format!("{}.{}", name, ext));
                    if with_ext.is_file() {
                        return Some(with_ext);
                    }
                }
            }
        }
        None
    }

    fn extended_search_dirs(&self) -> Vec<PathBuf> {
        let mut dirs_list: Vec<PathBuf> = Vec::new();

        #[cfg(unix)]
        {
            if let Some(shell_path) = resolve_unix_login_path() {
                dirs_list.extend(env::split_paths(&shell_path));
            }
            dirs_list.push(self.home.join(".local/bin"));
            dirs_list.push(self.home.join(".cargo/bin"));
            dirs_list.push(self.home.join(".npm-global/bin"));
            dirs_list.push(PathBuf::from("/opt/homebrew/bin"));
            dirs_list.push(PathBuf::from("/usr/local/bin"));
        }

        #[cfg(target_os = "windows")]
        {
            if let Ok(local_app_data) = env::var("LOCALAPPDATA") {
                dirs_list.push(PathBuf::from(&local_app_data).join("agy").join("bin"));
                dirs_list.push(PathBuf::from(&local_app_data).join("Programs"));
            }
            if let Ok(app_data) = env::var("APPDATA") {
                dirs_list.push(PathBuf::from(app_data).join("npm"));
            }
            dirs_list.push(self.home.join(".cargo").join("bin"));
            dirs_list.push(self.home.join(".local").join("bin"));
        }

        dirs_list
    }

    /// Detect installed Windows Subsystem for Linux (WSL) distributions on Windows hosts.
    pub fn detect_wsl_distros() -> Vec<WslDistroInfo> {
        #[cfg(target_os = "windows")]
        {
            let output = silent_command("wsl.exe").args(["-l", "-q"]).output();
            let Ok(out) = output else {
                return Vec::new();
            };
            if !out.status.success() {
                return Vec::new();
            }
            let raw = String::from_utf8_lossy(&out.stdout).replace('\0', "");
            raw.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .map(|distro| {
                    let home_unc = resolve_wsl_home_unc(&distro);
                    WslDistroInfo {
                        name: distro,
                        home_unc,
                    }
                })
                .collect()
        }
        #[cfg(not(target_os = "windows"))]
        {
            Vec::new()
        }
    }

    pub fn expand_tilde(&self, p: &Path) -> PathBuf {
        let s = p.to_string_lossy();
        if let Some(rest) = s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\")) {
            self.home.join(rest)
        } else if s == "~" {
            self.home.clone()
        } else {
            p.to_path_buf()
        }
    }
}

fn find_bundled_bridge_binary() -> Option<PathBuf> {
    let exe = env::current_exe().ok()?;
    let dir = exe.parent()?;
    let bin_name = if cfg!(target_os = "windows") {
        "ov-bridge.exe"
    } else {
        "ov-bridge"
    };
    let direct = dir.join(bin_name);
    if direct.is_file() {
        return Some(direct);
    }
    if let Some(release_sibling) = dir.parent().map(|p| p.join("release").join(bin_name)) {
        if release_sibling.is_file() {
            return Some(release_sibling);
        }
    }
    None
}

#[cfg(unix)]
fn resolve_unix_login_path() -> Option<String> {
    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let output = Command::new(&shell)
        .args(["-ilc", "printf '%s' \"$PATH\""])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

#[cfg(target_os = "windows")]
fn resolve_wsl_home_unc(distro: &str) -> Option<PathBuf> {
    let out = silent_command("wsl.exe")
        .args(["-d", distro, "--", "sh", "-c", "printf '%s' \"$HOME\""])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let linux_home = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if linux_home.is_empty() {
        return None;
    }
    let rel = linux_home.trim_start_matches('/').replace('/', "\\");
    Some(PathBuf::from(format!("\\\\wsl$\\{}\\{}", distro, rel)))
}

pub fn probe_cli_version(binary: &Path) -> Option<String> {
    let bin_str = binary.to_string_lossy();
    let out = silent_command(&bin_str).arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().next()?.trim();
    if line.is_empty() {
        None
    } else {
        Some(line.to_string())
    }
}
