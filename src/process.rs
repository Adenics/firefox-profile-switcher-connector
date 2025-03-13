use std::{io, env, fs};
use std::env::VarError;
use cfg_if::cfg_if;
use std::path::PathBuf;
use std::process::{exit, Child, Command, Stdio};
use once_cell::sync::Lazy;
use crate::state::AppState;
use crate::profiles::ProfileEntry;

cfg_if! {
    if #[cfg(target_family = "unix")] {
        use nix::unistd::ForkResult;
        use nix::sys::wait::waitpid;
    } else if #[cfg(target_family = "windows")] {
        use windows::Win32::System::Threading as win_threading;
        use windows::Win32::UI::Shell::{ApplicationActivationManager, IApplicationActivationManager, AO_NONE};
        use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
        use windows::Win32::Foundation::PWSTR;
        use std::os::windows::process::CommandExt;
        use crate::config::get_msix_package;
    } else {
        compile_error!("Unknown OS!");
    }
}


#[derive(Debug)]
pub enum ForkBrowserProcError {
    BadExitCode,
    ForkError { error_message: String },
    ProcessLaunchError(io::Error),
    MSIXProcessLaunchError { error_message: String },
    BinaryNotFound,
    BinaryDoesNotExist,
    COMError { error_message: String }
}

// List of known browser executable names
const BROWSER_EXECUTABLES: [&str; 4] = ["firefox", "librewolf", "waterfox", "zen-browser"];

// Find browser binary by looking in common locations
fn find_browser_binary() -> Option<PathBuf> {
    cfg_if! {
        if #[cfg(target_family = "unix")] {
            // Check common paths on Linux
            for browser in BROWSER_EXECUTABLES.iter() {
                // Check standard locations
                let standard_paths = [
                    format!("/usr/bin/{}", browser),
                    format!("/usr/local/bin/{}", browser),
                    format!("/snap/bin/{}", browser),
                ];
                
                for path in standard_paths.iter() {
                    let path_buf = PathBuf::from(path);
                    if path_buf.exists() {
                        log::info!("Found browser binary at: {}", path);
                        return Some(path_buf);
                    }
                }
                
                // Check flatpak locations
                if browser == &"firefox" {
                    let flatpak_path = "/var/lib/flatpak/app/org.mozilla.firefox/current/active/files/bin/firefox";
                    if PathBuf::from(flatpak_path).exists() {
                        log::info!("Found Flatpak Firefox at: {}", flatpak_path);
                        return Some(PathBuf::from(flatpak_path));
                    }
                } else if browser == &"librewolf" {
                    let flatpak_path = "/var/lib/flatpak/app/io.gitlab.librewolf-community/current/active/files/bin/librewolf";
                    if PathBuf::from(flatpak_path).exists() {
                        log::info!("Found Flatpak LibreWolf at: {}", flatpak_path);
                        return Some(PathBuf::from(flatpak_path));
                    }
                } else if browser == &"waterfox" {
                    let flatpak_path = "/var/lib/flatpak/app/net.waterfox.waterfox/current/active/files/bin/waterfox";
                    if PathBuf::from(flatpak_path).exists() {
                        log::info!("Found Flatpak Waterfox at: {}", flatpak_path);
                        return Some(PathBuf::from(flatpak_path));
                    }
                }
            }
        } else if #[cfg(target_os = "macos")] {
            // Check common paths on macOS
            let browser_paths = [
                "/Applications/Firefox.app/Contents/MacOS/firefox",
                "/Applications/LibreWolf.app/Contents/MacOS/librewolf",
                "/Applications/Waterfox.app/Contents/MacOS/waterfox",
                "/Applications/Zen Browser.app/Contents/MacOS/zen-browser",
            ];
            
            for path in browser_paths.iter() {
                let path_buf = PathBuf::from(path);
                if path_buf.exists() {
                    log::info!("Found browser binary at: {}", path);
                    return Some(path_buf);
                }
            }
        } else if #[cfg(target_os = "windows")] {
            // Check common paths on Windows
            let program_files = env::var("ProgramFiles").unwrap_or_else(|_| String::from("C:\\Program Files"));
            let program_files_x86 = env::var("ProgramFiles(x86)").unwrap_or_else(|_| String::from("C:\\Program Files (x86)"));
            
            let browser_paths = [
                format!("{}\\Mozilla Firefox\\firefox.exe", program_files),
                format!("{}\\LibreWolf\\librewolf.exe", program_files),
                format!("{}\\Waterfox\\waterfox.exe", program_files),
                format!("{}\\Zen Browser\\zen-browser.exe", program_files),
                format!("{}\\Mozilla Firefox\\firefox.exe", program_files_x86),
                format!("{}\\LibreWolf\\librewolf.exe", program_files_x86),
                format!("{}\\Waterfox\\waterfox.exe", program_files_x86),
                format!("{}\\Zen Browser\\zen-browser.exe", program_files_x86),
            ];
            
            for path in browser_paths.iter() {
                let path_buf = PathBuf::from(path);
                if path_buf.exists() {
                    log::info!("Found browser binary at: {}", path);
                    return Some(path_buf);
                }
            }
        }
    }
    
    None
}

pub fn fork_browser_proc(app_state: &AppState, profile: &ProfileEntry, url: Option<String>) -> Result<(), ForkBrowserProcError> {
    // Special case on Windows when FF is installed from Microsoft Store
    cfg_if! {
        if #[cfg(target_family = "windows")] {
            if let Ok(msix_package) = get_msix_package() {
                let aam: IApplicationActivationManager = unsafe {
                    CoCreateInstance(
                        &ApplicationActivationManager,
                        None,
                        CLSCTX_ALL
                    )
                }.map_err(|e| ForkBrowserProcError::COMError {
                    error_message: e.message().to_string_lossy()
                })?;

                let browser_args = build_browser_args(&profile.name, url)
                    .iter()
                    // Surround each arg with quotes and escape quotes with triple quotes
                    // See: https://stackoverflow.com/questions/7760545/escape-double-quotes-in-parameter
                    .map(|a| format!(r#""{}""#, a.replace(r#"""#, r#"""""#)))
                    .collect::<Vec<String>>()
                    .join(" ");

                log::trace!("Browser args: {:?}", browser_args);

                let aumid = format!("{}!App", msix_package);
                unsafe {
                    aam.ActivateApplication(
                        aumid.as_str(),
                        browser_args.as_str(),
                        AO_NONE
                    )
                }.map_err(|e| ForkBrowserProcError::MSIXProcessLaunchError {
                    error_message: e.message().to_string_lossy()
                })?;

                return Ok(());
            }
        }
    }

    // Try to get browser binary from various sources
    let parent_proc = match app_state.config.browser_binary() {
        Some(v) => v.clone(),
        None => match get_parent_proc_path() {
            Ok(v) => v.clone(),
            Err(_) => match find_browser_binary() {
                Some(binary) => binary,
                None => return Err(ForkBrowserProcError::BinaryNotFound)
            }
        }
    };

    if !parent_proc.exists() {
        // Try to find an alternative browser if the original one doesn't exist
        match find_browser_binary() {
            Some(alt_binary) => {
                log::info!("Original browser binary not found, using alternative: {:?}", alt_binary);
                if !alt_binary.exists() {
                    return Err(ForkBrowserProcError::BinaryDoesNotExist);
                }
                
                let browser_args = build_browser_args(&profile.name, url);
                log::trace!("Browser args: {:?}", browser_args);
                
                return launch_browser_process(&alt_binary, browser_args);
            }
            None => return Err(ForkBrowserProcError::BinaryDoesNotExist)
        }
    }

    log::trace!("Browser binary found: {:?}", parent_proc);

    let browser_args = build_browser_args(&profile.name, url);

    log::trace!("Browser args: {:?}", browser_args);
    
    launch_browser_process(&parent_proc, browser_args)
}

// Extract the process launching logic to a separate function
fn launch_browser_process(browser_path: &PathBuf, args: Vec<String>) -> Result<(), ForkBrowserProcError> {
    cfg_if! {
        if #[cfg(target_family = "unix")] {
            match unsafe { nix::unistd::fork() } {
                Ok(ForkResult::Parent {child}) => {
                    match waitpid(child, None) {
                        Ok(nix::sys::wait::WaitStatus::Exited(_, 0)) => Ok(()),
                        _ => Err(ForkBrowserProcError::BadExitCode)
                    }
                },
                Ok(ForkResult::Child) => exit(match nix::unistd::setsid() {
                    Ok(_) => {
                        // Close stdout, stderr and stdin
                        /*unsafe {
                            libc::close(0);
                            libc::close(1);
                            libc::close(2);
                        }*/
                        match spawn_browser_proc(browser_path, args) {
                            Ok(_) => 0,
                            Err(_) => 1
                        }
                    },
                    Err(_) => 2
                }),
                Err(e) => Err(ForkBrowserProcError::ForkError { error_message: format!("{:?}", e) })
            }
        } else if #[cfg(target_family = "windows")] {
            // TODO Change app ID to separate on taskbar?
            match spawn_browser_proc(browser_path, args) {
                Ok(_) => Ok(()),
                Err(e) => Err(ForkBrowserProcError::ProcessLaunchError(e))
            }
        } else {
            compile_error!("Unknown OS!");
        }
    }
}

fn build_browser_args(profile_name: &str, url: Option<String>) -> Vec<String> {
    let mut vec = vec![
        "-P".to_owned(),
        profile_name.to_owned()
    ];
    if let Some(url) = url {
        vec.push("--new-tab".to_owned());
        vec.push(url);
    }
    vec
}

fn spawn_browser_proc(bin_path: &PathBuf, args: Vec<String>) -> io::Result<Child> {
    let mut command = Command::new(bin_path);
    cfg_if! {
        if #[cfg(target_family = "windows")] {
            command.creation_flags((win_threading::DETACHED_PROCESS | win_threading::CREATE_BREAKAWAY_FROM_JOB).0);
        }
    }
    command.args(args);
    log::trace!("Executing command: {:?}", command);
    return command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

#[derive(Debug)]
pub enum GetParentProcError {
    NoCrashReporterEnvVar(VarError),
    LinuxOpenCurProcFailed(io::Error),
    LinuxFailedToParsePidString(String),
    LinuxCouldNotFindPPid,
    LinuxResolveParentExeFailed(io::Error)
}

static PARENT_PROC: Lazy<Result<PathBuf, GetParentProcError>> = Lazy::new(|| {
    // Get browser binary by reading crash-reporter env var
    let crash_reporter_result = env::var("MOZ_CRASHREPORTER_RESTART_ARG_0")
        .map(PathBuf::from)
        .map_err(GetParentProcError::NoCrashReporterEnvVar);
    
    // If crash reporter env var is available, use it
    if let Ok(path) = &crash_reporter_result {
        if path.exists() {
            return crash_reporter_result;
        }
    }
    
    // Otherwise, try to find a browser binary
    if let Some(browser_path) = find_browser_binary() {
        Ok(browser_path)
    } else {
        // If no browser binary found, return the original result
        crash_reporter_result
    }
});

pub fn get_parent_proc_path() -> Result<&'static PathBuf, &'static GetParentProcError> {
    PARENT_PROC.as_ref()
}