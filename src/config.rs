// === CONFIG ===

use std::path::{PathBuf};
use serde::{Deserialize, Serialize};
use cfg_if::cfg_if;
use std::fs::OpenOptions;
use once_cell::sync::Lazy;
use std::fs;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    browser_profile_dir: Option<PathBuf>,
    browser_binary: Option<PathBuf>
}

impl Config {
    pub fn browser_profile_dir(&self) -> PathBuf {
        self.browser_profile_dir.clone()
            .unwrap_or_else(|| get_default_browser_profile_folder().clone())
    }
    pub fn browser_binary(&self) -> Option<&PathBuf> {
        self.browser_binary.as_ref()
    }

    pub fn profiles_ini_path(&self) -> PathBuf {
        let mut profiles_ini = self.browser_profile_dir();
        profiles_ini.push("profiles.ini");
        return profiles_ini;
    }
    pub fn installs_ini_path(&self) -> PathBuf {
        let mut installs_ini = self.browser_profile_dir();
        installs_ini.push("installs.ini");
        return installs_ini;
    }
}

// Detect if Firefox is installed from Microsoft Store
#[cfg(target_os = "windows")]
static MSIX_PACKAGE: Lazy<Result<String, String>> = Lazy::new(|| {
    get_parent_proc_path()
        .map_err(|e| format!("get_parent_proc_path failed: {:?}", e))
        .and_then(|p| {
            // Windows path looks like this:
            // [Prefix(PrefixComponent { raw: "C:", parsed: Disk(67) }), RootDir, Normal("Program Files"), Normal("WindowsApps"), Normal("Mozilla.Firefox_97.0.1.0_x64__n80bbvh6b1yt2"), Normal("VFS"), Normal("ProgramFiles"), Normal("Firefox Package Root"), Normal("firefox.exe")]
            let components: Vec<Component> = p.components()
                // Skip beginning of path until we get to the root dir (e.g. the C: prefix)
                .skip_while(|c| !matches!(c, Component::RootDir))
                .skip(1) // Now skip the root dir
                .take(3) // Take the "Program Files", "WindowsApps" and package entries
                .collect();

            if let [
                Component::Normal(p1),
                Component::Normal(p2),
                Component::Normal(package)
            ] = components[..] {
                if p1 == "Program Files" && p2 == "WindowsApps" {
                    if let Some(package) = package.to_str() {
                        if let [Some(pname_sep), Some(pid_sep)] = [package.find("_"), package.rfind("_")] {
                            return Ok(format!("{}_{}", &package[..pname_sep], &package[pid_sep + 1..]))
                        }
                    }
                }
            }

            Err(format!("Browser path is not in MSIX format, components: {:?}!", components))
        })
});
#[cfg(target_os = "windows")]
pub fn get_msix_package() -> Result<&'static String, &'static String> {
    MSIX_PACKAGE.as_ref()
}

// Define Firefox fork directory names
const FIREFOX_DIRS: [&str; 4] = ["firefox", "librewolf", "waterfox", "zen-browser"];

// Check if a directory exists and contains a profiles.ini file
fn is_valid_browser_dir(dir: &PathBuf) -> bool {
    let profiles_ini = dir.join("profiles.ini");
    profiles_ini.exists()
}

// Define Flatpak app IDs for supported browsers
const FLATPAK_APP_IDS: [(&str, &str); 4] = [
    ("firefox", "org.mozilla.firefox"),
    ("librewolf", "io.gitlab.librewolf-community"),
    ("waterfox", "net.waterfox.waterfox"),
    ("zen-browser", "org.mozilla.firefox.zen")  // Adjust if Zen has a different Flatpak ID
];

static DEFAULT_BROWSER_PROFILE_FOLDER: Lazy<PathBuf> = Lazy::new(|| {
    let user_dirs = directories::UserDirs::new()
        .expect("Unable to determine user folder!");

    let mut result = PathBuf::new();
    
    cfg_if! {
        if #[cfg(target_os = "linux")] {
            // First check for Flatpak installations
            let home_dir = user_dirs.home_dir().to_path_buf();
            
            // Try each Firefox fork in Flatpak first
            for (dir_name, app_id) in FLATPAK_APP_IDS.iter() {
                let browser_dir_path;
                if *dir_name == "firefox" {
                    // Firefox uses .mozilla/firefox subfolder
                    browser_dir_path = home_dir.join(format!(".var/app/{0}/.mozilla/firefox", app_id));
                } else {
                    // Other forks typically use .{name} directly
                    browser_dir_path = home_dir.join(format!(".var/app/{0}/.{1}", app_id, dir_name));
                }
                
                if is_valid_browser_dir(&browser_dir_path) {
                    log::info!("Found Flatpak {} profile dir: {:?}", dir_name, browser_dir_path);
                    return browser_dir_path;
                }
            }
            
            // Check for standard installations
            result = user_dirs.home_dir().to_path_buf();
            
            // Try to find the first valid Firefox-like browser directory
            for dir_name in &FIREFOX_DIRS {
                let mozilla_dir = result.join(".mozilla").join(dir_name);
                let direct_dir = result.join(format!(".{}", dir_name));
                
                if is_valid_browser_dir(&mozilla_dir) {
                    log::info!("Found profile dir for: {}", dir_name);
                    return mozilla_dir;
                } else if is_valid_browser_dir(&direct_dir) {
                    log::info!("Found profile dir for: {}", dir_name);
                    return direct_dir;
                }
            }
            
            // Default fallback to Firefox
            result.push(".mozilla");
            result.push("firefox");
        } else if #[cfg(target_os = "macos")] {
            result = user_dirs.home_dir().to_path_buf();
            result.push("Library");
            result.push("Application Support");
            
            // Try each supported browser
            for dir_name in &FIREFOX_DIRS {
                let capitalized = dir_name.chars().next().unwrap().to_uppercase().collect::<String>() + &dir_name[1..];
                let browser_dir = result.join(&capitalized);
                
                if is_valid_browser_dir(&browser_dir) {
                    log::info!("Found profile dir for: {}", capitalized);
                    return browser_dir;
                }
            }
            
            // Default fallback to Firefox
            result.push("Firefox");
        } else if #[cfg(target_os = "windows")] {
            match MSIX_PACKAGE.as_ref() {
                Ok(msix_package) => {
                    log::trace!("Detected MSIX package: {}", msix_package);

                    result = user_dirs.home_dir().to_path_buf();
                    result.push("AppData");
                    result.push("Local");
                    result.push("Packages");
                    result.push(msix_package);
                    result.push("LocalCache");
                }
                Err(e) => {
                    log::trace!("Did not detect MSIX package: {}", e);

                    result = user_dirs.home_dir().to_path_buf();
                    result.push("AppData");
                }
            }
            result.push("Roaming");
            result.push("Mozilla");
            
            // Try each supported browser on Windows
            for dir_name in &FIREFOX_DIRS {
                let capitalized = dir_name.chars().next().unwrap().to_uppercase().collect::<String>() + &dir_name[1..];
                let browser_dir = result.join(&capitalized);
                
                if is_valid_browser_dir(&browser_dir) {
                    log::info!("Found profile dir for: {}", capitalized);
                    return browser_dir;
                }
            }
            
            // Default fallback to Firefox
            result.push("Firefox");
        } else {
            compile_error!("Unknown OS!");
        }
    }
    
    log::trace!("Found default browser profile dir: {:?}", result);
    return result;
});

fn get_default_browser_profile_folder() -> &'static PathBuf {
    &DEFAULT_BROWSER_PROFILE_FOLDER
}

impl Default for Config {
    fn default() -> Self {
        Config {
            browser_profile_dir: None,
            browser_binary: None
        }
    }
}

pub fn read_configuration(path: &PathBuf) -> Config {
    if let Ok(file) = OpenOptions::new().read(true).open(path) {
        if let Ok(config) = serde_json::from_reader(file) {
            return config;
        }
    }

    // Config doesn't exist or is invalid, load default config
    Config::default()
}