//! Embedded RTK output filters. Command execution and metrics belong to Sparkplane.
// Vendored modules retain upstream style and public helpers for future filter imports.
#[allow(dead_code, clippy::all)]
mod cargo;
#[allow(dead_code, clippy::all)]
mod git;
#[allow(dead_code, clippy::all)]
mod pytest;
#[allow(dead_code, clippy::all)]
mod core {
    pub mod constants;
    pub mod guard;
    pub mod stream;
    pub mod toml_filter;
    pub mod truncate;
    pub mod utils;
    pub mod tracking {
        pub fn estimate_tokens(value: &str) -> usize {
            value.len().div_ceil(4)
        }
    }
    pub mod tee {
        pub fn force_tee_hint(_: &str, _: &str) -> Option<String> {
            None
        }
        pub fn force_tee_tail_hint(_: &str, _: &str, _: usize) -> Option<String> {
            None
        }
    }
}

pub const REVISION: &str = "6d4b77eadee1c66dc1f68466ad77e96d1b6e4989";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filter {
    GitStatus,
    Cargo(&'static str),
    Pytest,
    Declarative(String),
}

/// Conservative shell-only profile. Explicit output formats must stay byte exact.
pub fn select(args: &[String]) -> Option<Filter> {
    if args.iter().any(|a| {
        a.starts_with('/')
            || a.contains('=')
            || [
                "--json",
                "--porcelain",
                "--format",
                "--output",
                "--raw",
                "--null",
                "-z",
                "--message-format",
                "--xml",
                "--csv",
                "--help",
                "--version",
            ]
            .iter()
            .any(|flag| a == flag || a.starts_with(&format!("{flag}=")))
    }) {
        return None;
    }
    match (args.first()?.as_str(), args.get(1).map(String::as_str)) {
        ("git", Some("status")) => Some(Filter::GitStatus),
        ("cargo", Some("test")) => Some(Filter::Cargo("test")),
        ("cargo", Some("build")) => Some(Filter::Cargo("build")),
        ("cargo", Some("check")) => Some(Filter::Cargo("check")),
        ("cargo", Some("clippy")) => Some(Filter::Cargo("clippy")),
        ("pytest", _) => Some(Filter::Pytest),
        ("jq", _) => None,
        _ => {
            let command = args.join(" ");
            core::toml_filter::find_matching_filter(&command)
                .map(|f| Filter::Declarative(f.name.clone()))
        }
    }
}

/// No truncation of unsuccessful executions: retain every diagnostic.
pub fn filter(profile: &Filter, output: &[u8], exit_code: i32) -> Vec<u8> {
    let Ok(raw) = std::str::from_utf8(output) else {
        return output.to_vec();
    };
    if exit_code != 0 || output.contains(&0) {
        return output.to_vec();
    }
    let compact = match profile {
        Filter::GitStatus => git::filter_status_with_args(raw),
        Filter::Cargo("test") => cargo::filter_cargo_test(raw),
        Filter::Cargo(label) => cargo::filter_cargo_build_labeled(raw, label, exit_code),
        Filter::Pytest => pytest::filter_pytest_output(raw),
        Filter::Declarative(name) => {
            let registry = core::toml_filter::registry();
            let Some(filter) = registry.filters.iter().find(|f| &f.name == name) else {
                return output.to_vec();
            };
            core::toml_filter::apply_filter(filter, raw)
        }
    };
    let compact = if compact.ends_with('\n') {
        compact
    } else {
        format!("{compact}\n")
    };
    if compact.len() >= output.len() {
        output.to_vec()
    } else {
        compact.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cargo_success_compresses_and_failure_is_exact() {
        let raw = b"running 2 tests\ntest first ... ok\ntest second ... ok\ntest result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n";
        let compact = filter(&Filter::Cargo("test"), raw, 0);
        assert!(compact.len() < raw.len());
        assert!(String::from_utf8(compact).unwrap().contains("2 passed"));
        assert_eq!(filter(&Filter::Cargo("test"), raw, 1), raw);
    }
    #[test]
    fn builtins_compile_and_binary_is_exact() {
        let registry = core::toml_filter::registry();
        assert!(registry.filters.len() > 50);
        assert_eq!(filter(&Filter::GitStatus, b"\xff\x00", 0), b"\xff\x00");
    }
}
