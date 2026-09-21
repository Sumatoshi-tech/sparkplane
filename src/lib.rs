//! Independent DGX Spark workstation and appliance implementation.
pub mod client_migration;
pub mod generated_files;
#[cfg(feature = "appliance")]
pub mod migration;
pub mod spark;

pub fn human_bytes(n: u64) -> String {
    const UNITS: &[&str] = &["B", "K", "M", "G", "T"];
    if n < 1024 {
        return format!("{n}B");
    }
    let mut v = n as f64;
    let mut idx = 0;
    while v >= 1024.0 && idx < UNITS.len() - 1 {
        v /= 1024.0;
        idx += 1;
    }
    if v >= 100.0 {
        format!("{:.0}{}", v, UNITS[idx])
    } else if v >= 10.0 {
        format!("{:.1}{}", v, UNITS[idx])
    } else {
        format!("{:.2}{}", v, UNITS[idx])
    }
}
