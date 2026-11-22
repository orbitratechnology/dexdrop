//! Human-readable sizes, rates and durations, in decimal units (PRD 7.4).

/// `1234567` → `"1.2 MB"`.
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 6] = ["B", "kB", "MB", "GB", "TB", "PB"];
    if n < 1000 {
        return format!("{n} B");
    }
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else if value >= 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

/// Bytes per second → `"12.5 MB/s"`.
pub fn rate(bytes_per_second: f64) -> String {
    if !bytes_per_second.is_finite() || bytes_per_second <= 0.0 {
        return "—".to_owned();
    }
    format!("{}/s", bytes(bytes_per_second as u64))
}

/// Seconds → `"1 h 4 min"`, `"3 min 20 s"`, `"12 s"`.
pub fn duration(seconds: u64) -> String {
    let h = seconds / 3600;
    let m = (seconds % 3600) / 60;
    let s = seconds % 60;
    if h > 0 {
        format!("{h} h {m} min")
    } else if m > 0 {
        format!("{m} min {s} s")
    } else {
        format!("{s} s")
    }
}

/// `"3 files, 1 folder"`.
pub fn item_count(files: u64, folders: u64) -> String {
    let f = if files == 1 { "file" } else { "files" };
    if folders == 0 {
        format!("{files} {f}")
    } else {
        let d = if folders == 1 { "folder" } else { "folders" };
        format!("{files} {f}, {folders} {d}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sizes() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(999), "999 B");
        assert_eq!(bytes(1_000), "1.00 kB");
        assert_eq!(bytes(12_345_678), "12.3 MB");
        assert_eq!(bytes(250_000_000_000), "250 GB");
        assert_eq!(rate(0.0), "—");
        assert_eq!(rate(2_500_000.0), "2.50 MB/s");
        assert_eq!(duration(3_840), "1 h 4 min");
        assert_eq!(duration(200), "3 min 20 s");
        assert_eq!(item_count(1, 0), "1 file");
        assert_eq!(item_count(3, 2), "3 files, 2 folders");
    }
}
