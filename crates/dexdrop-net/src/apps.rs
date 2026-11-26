//! Apps on this device, to send their installers (Android).
//!
//! Android keeps every installed app's APK where any app may read it
//! (`ApplicationInfo.sourceDir`, plus split APKs for apps from bundles), so
//! an app can be sent like a file. The system's `PackageManager` lists the
//! apps and reads an APK file's name, version and icon, through a small
//! Java bridge (`android/dev/dexdrop/AppsBridge.java`). Elsewhere there is
//! nothing to list: [`installed`] says so.

use std::{
    io,
    path::{Path, PathBuf},
};

/// Separates an app's fields (see `AppsBridge.java`).
const SEP: char = '\u{1}';
/// Separates its APK paths.
const LIST_SEP: char = '\u{2}';

/// An installed app, or an APK file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppInfo {
    /// Its package name (`com.example.app`).
    pub package: String,
    /// The name people see.
    pub label: String,
    pub version: String,
    pub version_code: i64,
    /// Part of the system (not updated): usually not worth sending.
    pub system: bool,
    /// The bytes of all its APKs.
    pub size: u64,
    /// Its APKs: the base first, then any splits.
    pub apks: Vec<PathBuf>,
    /// A PNG of its icon, when one could be drawn.
    pub icon: Option<PathBuf>,
    /// When it was installed and last updated, Unix milliseconds (0 for a
    /// file).
    pub installed: u64,
    pub updated: u64,
    pub min_sdk: u32,
    pub target_sdk: u32,
}

impl AppInfo {
    /// Whether it is installed from a bundle, as several APKs.
    pub fn split(&self) -> bool {
        self.apks.len() > 1
    }

    /// A file name for it: `"Maps 11.2.apk"`, without characters file
    /// systems refuse.
    pub fn file_stem(&self) -> String {
        let name = if self.version.is_empty() {
            clean_name(&self.label)
        } else {
            clean_name(&format!("{} {}", self.label, self.version))
        };
        if name.is_empty() {
            self.package.clone()
        } else {
            name
        }
    }
}

/// `name` without characters file systems refuse, at most 80 characters.
fn clean_name(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    clean.trim().trim_matches('.').chars().take(80).collect()
}

/// Reads the bridge's lines; malformed ones are skipped.
pub fn parse(text: &str) -> Vec<AppInfo> {
    text.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<AppInfo> {
    let f: Vec<&str> = line.split(SEP).collect();
    if f.len() < 12 || f[0].is_empty() {
        return None;
    }
    let apks: Vec<PathBuf> = f[6]
        .split(LIST_SEP)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .collect();
    if apks.is_empty() {
        return None;
    }
    Some(AppInfo {
        package: f[0].to_owned(),
        label: if f[1].is_empty() { f[0] } else { f[1] }.to_owned(),
        version: f[2].to_owned(),
        version_code: f[3].parse().unwrap_or(0),
        system: f[4] == "1",
        size: f[5].parse().unwrap_or(0),
        apks,
        icon: (!f[7].is_empty()).then(|| PathBuf::from(f[7])),
        installed: f[8].parse().unwrap_or(0),
        updated: f[9].parse().unwrap_or(0),
        min_sdk: f[10].parse().unwrap_or(0),
        target_sdk: f[11].parse().unwrap_or(0),
    })
}

/// What to send for an app, under a readable name: `"Maps 11.2.apk"`, its
/// APK read where it is (a reference, `dexdrop_core::asset::LocalFiles`:
/// nothing copied); for an app installed as several APKs, a folder
/// `"Maps 11.2"` copied into `dir` with `"Maps.apk"` and its splits (install
/// it with an installer that takes split APKs).
pub fn stage(app: &AppInfo, dir: &Path) -> io::Result<PathBuf> {
    let stem = app.file_stem();
    if !app.split() {
        return Ok(dexdrop_core::asset::LocalFiles::reference(
            &app.apks[0],
            &format!("{stem}.apk"),
        ));
    }
    std::fs::create_dir_all(dir)?;
    let folder = dexdrop_core::naming::first_free(dir, &stem);
    std::fs::create_dir_all(&folder)?;
    for (i, apk) in app.apks.iter().enumerate() {
        let name = if i == 0 {
            let label = clean_name(&app.label);
            format!(
                "{}.apk",
                if label.is_empty() {
                    &app.package
                } else {
                    &label
                }
            )
        } else {
            apk.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| format!("split-{i}.apk"))
        };
        std::fs::copy(apk, folder.join(name))?;
    }
    Ok(folder)
}

#[cfg(target_os = "android")]
mod system {
    use std::path::Path;

    use super::{AppInfo, parse};
    use crate::jvm::Bridge;

    static BRIDGE: Bridge = Bridge::new("dev.dexdrop.AppsBridge");

    fn call(method: &str, args: &[&str]) -> Result<String, String> {
        BRIDGE.call(method, args)
    }

    pub fn installed(icon_dir: &Path) -> Result<Vec<AppInfo>, String> {
        let mut apps = parse(&call("installed", &[&icon_dir.to_string_lossy()])?);
        apps.sort_by_key(|a| (a.system, a.label.to_lowercase()));
        Ok(apps)
    }

    pub fn archive(path: &Path, icon_dir: &Path) -> Option<AppInfo> {
        let text = call(
            "archive",
            &[&path.to_string_lossy(), &icon_dir.to_string_lossy()],
        )
        .ok()?;
        parse(&text).into_iter().next()
    }
}

/// The apps installed on this device that people open (launcher apps),
/// other than DexDrop, user apps first, by name. Their icons are drawn as
/// PNGs into `icon_dir`.
#[cfg(target_os = "android")]
pub fn installed(icon_dir: &Path) -> Result<Vec<AppInfo>, String> {
    system::installed(icon_dir)
}

/// The details of the APK file at `path`, if it is one.
#[cfg(target_os = "android")]
pub fn archive(path: &Path, icon_dir: &Path) -> Option<AppInfo> {
    system::archive(path, icon_dir)
}

#[cfg(not(target_os = "android"))]
pub fn installed(_icon_dir: &Path) -> Result<Vec<AppInfo>, String> {
    Err("Sending apps works on Android.".into())
}

#[cfg(not(target_os = "android"))]
pub fn archive(_path: &Path, _icon_dir: &Path) -> Option<AppInfo> {
    None
}

/// Whether this platform can list its apps.
pub const AVAILABLE: bool = cfg!(target_os = "android");

#[cfg(test)]
mod tests {
    use super::*;

    fn line(fields: &[&str]) -> String {
        fields.join("\u{1}")
    }

    #[test]
    fn reads_apps_and_skips_junk() {
        let text = [
            line(&[
                "com.example.maps",
                "Maps",
                "11.2",
                "1102",
                "0",
                "5000",
                "/data/app/maps/base.apk\u{2}/data/app/maps/split_config.arm64_v8a.apk",
                "/cache/icons/com.example.maps-1102.png",
                "1700000000000",
                "1700000001000",
                "26",
                "34",
            ]),
            "garbage".to_owned(),
            line(&["", "x", "", "", "", "", "", "", "", "", "", ""]),
            line(&[
                "com.example.notes",
                "",
                "",
                "3",
                "1",
                "10",
                "/a.apk",
                "",
                "0",
                "0",
                "0",
                "0",
            ]),
        ]
        .join("\n");
        let apps = parse(&text);
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].label, "Maps");
        assert!(apps[0].split());
        assert_eq!(apps[0].apks.len(), 2);
        assert_eq!(apps[0].version_code, 1102);
        assert_eq!(
            apps[0].icon.as_deref(),
            Some(Path::new("/cache/icons/com.example.maps-1102.png"))
        );
        assert_eq!(apps[1].label, "com.example.notes", "no label: the package");
        assert!(apps[1].system);
        assert_eq!(apps[1].icon, None);
    }

    #[test]
    fn stages_under_readable_names() {
        let src = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("base.apk"), b"base").unwrap();
        std::fs::write(src.path().join("split_config.en.apk"), b"split").unwrap();
        let mut app = parse(&line(&[
            "com.example.maps",
            "Maps: City/Road",
            "2.0",
            "2",
            "0",
            "9",
            &src.path().join("base.apk").to_string_lossy(),
            "",
            "0",
            "0",
            "0",
            "0",
        ]))
        .remove(0);
        let single = stage(&app, out.path()).unwrap();
        assert_eq!(single.file_name().unwrap(), "Maps- City-Road 2.0.apk");
        // Sent from where it is: nothing copied.
        assert!(dexdrop_core::asset::is_reference(&single));
        assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 0);
        let mut read = Vec::new();
        std::io::Read::read_to_end(&mut dexdrop_core::source::open(&single).unwrap(), &mut read)
            .unwrap();
        assert_eq!(read, b"base");

        app.apks.push(src.path().join("split_config.en.apk"));
        let folder = stage(&app, out.path()).unwrap();
        assert!(folder.is_dir());
        assert_eq!(
            std::fs::read(folder.join("Maps- City-Road.apk")).unwrap(),
            b"base"
        );
        assert_eq!(
            std::fs::read(folder.join("split_config.en.apk")).unwrap(),
            b"split"
        );
    }
}
