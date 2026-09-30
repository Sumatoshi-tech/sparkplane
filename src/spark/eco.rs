//! Invocation-local agent hooks backed by the embedded RTK library.
use super::{
    cli::LaunchIntegration,
    economics::{self, Compression},
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
};
use tokio::io::AsyncReadExt;

const MAX_CAPTURE: usize = 8 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Metric {
    category: String,
    raw_bytes: u64,
    filtered_bytes: u64,
    error: bool,
}

pub struct EcoSession {
    path: PathBuf,
    enabled: bool,
    policy_enabled: bool,
}
impl EcoSession {
    pub fn create(id: &str, enabled: bool, mode: &str) -> Result<Self> {
        ensure!(id.parse::<ulid::Ulid>().is_ok(), "invalid session id");
        let path = std::env::temp_dir().join(format!("sparkplane-eco-{id}"));
        ensure!(!path.exists(), "eco session directory already exists");
        economics::private_dir(&path)?;
        Ok(Self {
            path,
            enabled,
            policy_enabled: mode == "auto",
        })
    }
    pub fn configure(&self, command: &mut Command, integration: LaunchIntegration) -> Result<()> {
        command.env("SPARKPLANE_ECO_SESSION_DIR", &self.path).env(
            "SPARKPLANE_ECO_REWRITE_ENABLED",
            if self.enabled && self.policy_enabled {
                "1"
            } else {
                "0"
            },
        );
        if !self.enabled || !self.policy_enabled {
            return Ok(());
        }
        let binary = std::env::current_exe()?;
        let hook = format!(
            "{} __eco hook {}",
            shell_quote(&binary.to_string_lossy()),
            integration.as_str()
        );
        let hooks = json!({"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":hook,"timeout":3}]}],"SessionStart":[{"hooks":[{"type":"command","command":hook,"timeout":3}]}]});
        match integration {
            LaunchIntegration::Codex => {
                for event in ["PreToolUse", "SessionStart"] {
                    let table: toml::Value = serde_json::from_value(hooks[event].clone())?;
                    command.arg("-c").arg(format!("hooks.{event}={}", table));
                }
            }
            LaunchIntegration::Claude => {
                // Auto mode owns invocation settings. Carry its permission settings into the
                // combined object; user settings files remain loaded by Claude itself.
                let args = command
                    .get_args()
                    .map(|v| v.to_string_lossy().into_owned())
                    .collect::<Vec<_>>();
                let mut settings = args
                    .windows(2)
                    .rfind(|pair| pair[0] == "--settings")
                    .and_then(|pair| serde_json::from_str::<Value>(&pair[1]).ok())
                    .unwrap_or_else(|| json!({}));
                settings["hooks"] = hooks;
                let retained = command.get_args().map(OsString::from).collect::<Vec<_>>();
                // Command has no remove-argument API. Rebuild with the same program,
                // environment and inherited stdio, replacing only owned settings.
                let mut replacement = Command::new(command.get_program());
                for (key, value) in command.get_envs() {
                    if let Some(value) = value {
                        replacement.env(key, value);
                    } else {
                        replacement.env_remove(key);
                    }
                }
                if let Some(dir) = command.get_current_dir() {
                    replacement.current_dir(dir);
                }
                let mut index = 0;
                while index < retained.len() {
                    if retained[index] == "--settings" {
                        index += 2;
                    } else {
                        replacement.arg(&retained[index]);
                        index += 1;
                    }
                }
                replacement
                    .arg("--settings")
                    .arg(serde_json::to_string(&settings)?);
                *command = replacement;
            }
            LaunchIntegration::Opencode => {
                let plugin = self.path.join("eco.ts");
                fs::write(&plugin, opencode_plugin(&binary))?;
                let content = command
                    .get_envs()
                    .find(|(k, _)| *k == "OPENCODE_CONFIG_CONTENT")
                    .and_then(|(_, v)| v)
                    .ok_or_else(|| anyhow::anyhow!("missing OpenCode session config"))?;
                let mut settings: Value = serde_json::from_str(&content.to_string_lossy())?;
                let plugins = settings
                    .as_object_mut()
                    .ok_or_else(|| anyhow::anyhow!("invalid OpenCode config"))?
                    .entry("plugin")
                    .or_insert_with(|| json!([]));
                plugins
                    .as_array_mut()
                    .ok_or_else(|| anyhow::anyhow!("invalid OpenCode plugins"))?
                    .push(json!(format!("file://{}", plugin.display())));
                command.env("OPENCODE_CONFIG_CONTENT", serde_json::to_string(&settings)?);
            }
        }
        Ok(())
    }
    pub fn summary(&self) -> Compression {
        let mut result = Compression {
            status: if !self.enabled {
                "disabled"
            } else if !self.policy_enabled {
                "inactive (inherited permissions)"
            } else {
                "inactive (hook unavailable or awaiting trust)"
            }
            .into(),
            ..Compression::default()
        };
        if !self.enabled || !self.policy_enabled {
            return result;
        }
        let Ok(entries) = fs::read_dir(&self.path) else {
            result.metrics_errors += 1;
            return result;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                result.metrics_errors += 1;
                continue;
            };
            let path = entry.path();
            if path.extension().is_none_or(|v| v != "json") {
                continue;
            }
            match economics::read_record::<Metric>(&path) {
                Ok(metric) if metric.category == "hook" => result.status = "active".into(),
                Ok(metric) => {
                    result.commands += 1;
                    result.status = "active".into();
                    result.raw_bytes = result.raw_bytes.saturating_add(metric.raw_bytes);
                    result.filtered_bytes =
                        result.filtered_bytes.saturating_add(metric.filtered_bytes);
                    if metric.error {
                        result.metrics_errors += 1;
                    }
                }
                Err(_) => result.metrics_errors += 1,
            }
        }
        result
    }
}
impl Drop for EcoSession {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path).is_ok() && economics::private_dir(&self.path).is_ok() {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn opencode_plugin(binary: &Path) -> String {
    format!(
        r#"export const SparkplaneEco = async () => {{
  const binary = {};
  const activate = Bun.spawn([binary, "__eco", "activate"], {{stdout:"ignore",stderr:"ignore"}});
  await activate.exited;
  return {{"tool.execute.before": async (input, output) => {{
    if (!["bash", "shell"].includes(String(input.tool).toLowerCase())) return;
    if (typeof output.args?.command !== "string") return;
    const child = Bun.spawn([binary, "__eco", "rewrite", output.args.command], {{stdout:"pipe",stderr:"ignore"}});
    const command = (await new Response(child.stdout).text()).trim();
    if (await child.exited === 0 && command) output.args.command = command;
  }}}};
}};
"#,
        serde_json::to_string(&binary.to_string_lossy()).expect("path JSON")
    )
}

pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn rewrite(command: &str) -> Result<Option<String>> {
    if command.len() > 64 * 1024
        || command.chars().any(|c| {
            matches!(
                c,
                '\n' | '\r'
                    | '\0'
                    | '$'
                    | '`'
                    | '|'
                    | '&'
                    | ';'
                    | '<'
                    | '>'
                    | '*'
                    | '?'
                    | '('
                    | ')'
            )
        })
    {
        return Ok(None);
    }
    let Some(args) = shlex::split(command) else {
        return Ok(None);
    };
    if sparkplane_rtk::select(&args).is_none() {
        return Ok(None);
    }
    let binary = std::env::current_exe()?;
    Ok(Some(format!(
        "{} __eco exec -- {}",
        shell_quote(&binary.to_string_lossy()),
        args.iter()
            .map(|a| shell_quote(a))
            .collect::<Vec<_>>()
            .join(" ")
    )))
}

fn metric(metric: &Metric) {
    metric_named(&uuid::Uuid::new_v4().to_string(), metric);
}

fn metric_named(id: &str, metric: &Metric) {
    let Some(path) = std::env::var_os("SPARKPLANE_ECO_SESSION_DIR").map(PathBuf::from) else {
        return;
    };
    if economics::private_dir(&path).is_err() {
        eprintln!("Sparkplane eco metrics unavailable");
        return;
    }
    if economics::write_record(&path.join(format!("{id}.json")), metric).is_err() {
        eprintln!("Sparkplane eco metrics unavailable");
    }
}

fn activate() {
    metric(&Metric {
        category: "hook".into(),
        raw_bytes: 0,
        filtered_bytes: 0,
        error: false,
    });
}

pub fn run(args: Vec<OsString>) -> Result<i32> {
    let Some(action) = args.first().and_then(|a| a.to_str()) else {
        anyhow::bail!("missing eco action");
    };
    match action {
        "activate" => {
            activate();
            Ok(0)
        }
        "rewrite" => {
            ensure!(args.len() == 2, "rewrite expects one command");
            if std::env::var("SPARKPLANE_ECO_REWRITE_ENABLED")
                .ok()
                .as_deref()
                != Some("0")
                && let Some(command) = rewrite(&args[1].to_string_lossy())?
            {
                println!("{command}");
            }
            Ok(0)
        }
        "hook" => {
            let mut input = String::new();
            io::stdin()
                .take(1024 * 1024 + 1)
                .read_to_string(&mut input)?;
            if input.len() > 1024 * 1024 {
                return Ok(0);
            }
            let Ok(payload) = serde_json::from_str::<Value>(&input) else {
                return Ok(0);
            };
            if std::env::var("SPARKPLANE_ECO_REWRITE_ENABLED")
                .ok()
                .as_deref()
                != Some("1")
            {
                return Ok(0);
            }
            activate();
            if payload.get("hook_event_name").and_then(Value::as_str) == Some("SessionStart") {
                return Ok(0);
            }
            if payload.get("tool_name").and_then(Value::as_str) != Some("Bash") {
                return Ok(0);
            }
            let Some(command) = payload
                .pointer("/tool_input/command")
                .and_then(Value::as_str)
            else {
                return Ok(0);
            };
            if let Some(rewritten) = rewrite(command)? {
                println!(
                    "{}",
                    json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow","updatedInput":{"command":rewritten}}})
                );
            }
            Ok(0)
        }
        "exec" | "raw" => {
            let operands = &args[1..];
            let operands = operands
                .strip_prefix(&[OsString::from("--")])
                .unwrap_or(operands);
            ensure!(!operands.is_empty(), "missing command");
            let mut command = Command::new(&operands[0]);
            command.args(&operands[1..]);
            if action == "raw" {
                return Ok(exit_code(wait(command)?));
            }
            execute(command, operands)
        }
        _ => anyhow::bail!("unknown eco action"),
    }
}

pub fn exit_code(status: ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(1))
}

/// Keep the parent alive long enough to finish accounting when its child exits.
pub fn wait(command: Command) -> Result<ExitStatus> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let mut child = tokio::process::Command::from(command).spawn()?;
            wait_child(&mut child, false).await
        })
}

async fn wait_child(child: &mut tokio::process::Child, group: bool) -> Result<ExitStatus> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    loop {
        let signum = tokio::select! {status=child.wait()=>return Ok(status?),_=interrupt.recv()=>nix::sys::signal::Signal::SIGINT,_=terminate.recv()=>nix::sys::signal::Signal::SIGTERM};
        if let Some(pid) = child.id() {
            let pid = pid as i32;
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(if group { -pid } else { pid }),
                signum,
            );
        }
    }
}

async fn capture(
    mut reader: impl tokio::io::AsyncRead + Unpin,
    stderr: bool,
) -> io::Result<(Vec<u8>, u64, bool)> {
    let mut bytes = Vec::new();
    let mut count = 0u64;
    let mut streamed = false;
    let mut chunk = [0u8; 8192];
    loop {
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        count = count.saturating_add(n as u64);
        if !streamed && bytes.len() + n <= MAX_CAPTURE {
            bytes.extend_from_slice(&chunk[..n]);
        } else {
            if stderr {
                let mut out = io::stderr().lock();
                if !streamed {
                    out.write_all(&bytes)?;
                }
                out.write_all(&chunk[..n])?;
            } else {
                let mut out = io::stdout().lock();
                if !streamed {
                    out.write_all(&bytes)?;
                }
                out.write_all(&chunk[..n])?;
            }
            bytes.clear();
            streamed = true;
        }
    }
    Ok((bytes, count, streamed))
}

fn execute(mut command: Command, args: &[OsString]) -> Result<i32> {
    use std::os::unix::process::CommandExt;
    let args = args
        .iter()
        .map(|v| v.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let filter = sparkplane_rtk::select(&args);
    let metric_id = uuid::Uuid::new_v4().to_string();
    // An interrupted wrapper or failed final write leaves an incomplete record.
    metric_named(
        &metric_id,
        &Metric {
            category: "command".into(),
            raw_bytes: 0,
            filtered_bytes: 0,
            error: true,
        },
    );
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let mut child = match tokio::process::Command::from(command).spawn() {
                Ok(child) => child,
                Err(error) => {
                    eprintln!("could not execute command: {error}");
                    return Ok(if error.kind() == io::ErrorKind::NotFound {
                        127
                    } else {
                        126
                    });
                }
            };
            let stdout = child.stdout.take().expect("stdout pipe");
            let stderr = child.stderr.take().expect("stderr pipe");
            let (status, out, err) = tokio::join!(
                wait_child(&mut child, true),
                capture(stdout, false),
                capture(stderr, true)
            );
            let status = exit_code(status?);
            let (out, out_count, out_streamed) = out?;
            let (err, err_count, err_streamed) = err?;
            let filtered_out = filter
                .as_ref()
                .map(|f| sparkplane_rtk::filter(f, &out, status))
                .unwrap_or_else(|| out.clone());
            let filtered_err = match &filter {
                Some(sparkplane_rtk::Filter::Cargo(label)) => sparkplane_rtk::filter(
                    &sparkplane_rtk::Filter::Cargo(if *label == "test" { "build" } else { label }),
                    &err,
                    status,
                ),
                _ => err.clone(),
            };
            if !out_streamed {
                io::stdout().write_all(&filtered_out)?;
            }
            if !err_streamed {
                io::stderr().write_all(&filtered_err)?;
            }
            let filtered_count = if out_streamed {
                out_count
            } else {
                filtered_out.len() as u64
            } + if err_streamed {
                err_count
            } else {
                filtered_err.len() as u64
            };
            metric_named(
                &metric_id,
                &Metric {
                    category: "command".into(),
                    raw_bytes: out_count.saturating_add(err_count),
                    filtered_bytes: filtered_count,
                    error: false,
                },
            );
            Ok(status)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hooks_preserve_permissions_and_stay_invocation_local() {
        let session = EcoSession::create(&ulid::Ulid::new().to_string(), true, "auto").unwrap();
        let mut claude = Command::new("claude");
        claude
            .args([
                "--permission-mode",
                "bypassPermissions",
                "--settings",
                r#"{"sandbox":{"enabled":false},"skipDangerousModePermissionPrompt":true}"#,
            ])
            .env("FIXTURE", "kept");
        session
            .configure(&mut claude, LaunchIntegration::Claude)
            .unwrap();
        let args = claude
            .get_args()
            .map(|s| s.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            args.iter().filter(|s| s.as_str() == "--settings").count(),
            1
        );
        let settings: Value = serde_json::from_str(&args[3]).unwrap();
        assert_eq!(settings["sandbox"]["enabled"], false);
        assert_eq!(settings["skipDangerousModePermissionPrompt"], true);
        assert!(settings["hooks"]["PreToolUse"].is_array());
        assert!(claude.get_envs().any(|(key,value)|key=="FIXTURE"&&value==Some(std::ffi::OsStr::new("kept"))));
        let mut codex = Command::new("codex");
        session
            .configure(&mut codex, LaunchIntegration::Codex)
            .unwrap();
        let args = codex
            .get_args()
            .map(|s| s.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        for arg in args.iter().skip(1).step_by(2) {
            let _: toml::Value = toml::from_str(arg).unwrap();
        }
        assert!(!args.iter().any(|s| s.contains("bypass-hook-trust")));
        let inherited =
            EcoSession::create(&ulid::Ulid::new().to_string(), true, "inherit").unwrap();
        let mut native = Command::new("codex");
        native.args(["-c", "approval_policy=\"on-request\""]);
        inherited
            .configure(&mut native, LaunchIntegration::Codex)
            .unwrap();
        assert_eq!(native.get_args().count(), 2);
        assert!(inherited.summary().status.contains("inherited"));
    }
}
