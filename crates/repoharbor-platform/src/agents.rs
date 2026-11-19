//! Detect terminal coding-agent sessions running on the system by scanning
//! `/proc` — not just ones RepoHarbor launched. A process counts as an agent session
//! if its program (argv[0]) is a known agent CLI *and* its working directory is
//! inside one of the scanned repos. Terminate is a plain signal by pid, so no
//! child-process bookkeeping is needed.

use std::path::Path;

/// A detected agent process.
pub struct RunningAgent {
    pub pid: u32,
    /// The repo path whose tree the process is running in.
    pub repo: String,
    /// The full command line.
    pub command: String,
    /// Unix start time of the process (for a runtime readout).
    pub started_unix: i64,
}

/// Last path segment of `s` (the program basename), stripping `.exe` on Windows.
fn basename(s: &str) -> &str {
    let seg = s.rsplit(['/', '\\']).next().unwrap_or(s);
    seg.strip_suffix(".exe")
        .or_else(|| seg.strip_suffix(".EXE"))
        .unwrap_or(seg)
}

/// Boot time in unix seconds, from `/proc/stat`'s `btime` line.
#[cfg(target_os = "linux")]
fn boot_time() -> i64 {
    std::fs::read_to_string("/proc/stat")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("btime ").and_then(|n| n.trim().parse().ok()))
        })
        .unwrap_or(0)
}

/// Process start time in unix seconds, from `/proc/<pid>/stat` field 22.
#[cfg(target_os = "linux")]
fn start_time(pid: u32, btime: i64) -> i64 {
    // Clock ticks per second; 100 on effectively all Linux configs.
    const HZ: i64 = 100;
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return btime;
    };
    match starttime_ticks(&stat) {
        Some(ticks) => btime + ticks / HZ,
        None => btime,
    }
}

/// Parse the `starttime` field (clock ticks since boot, field 22) out of a
/// `/proc/<pid>/stat` line. Pure, so it's unit-testable. Returns `None` if the
/// line is malformed. `comm` (field 2, parenthesized) can itself contain spaces
/// and parens, so we anchor on the *last* ')' and count from there: starttime is
/// the 20th whitespace-separated field after it (index 19).
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn starttime_ticks(stat: &str) -> Option<i64> {
    let rparen = stat.rfind(')')?;
    stat[rparen + 1..]
        .split_whitespace()
        .nth(19)
        .and_then(|s| s.parse::<i64>().ok())
}

/// Scan running processes for agent sessions inside `repos`. `programs` is the
/// set of agent CLI basenames to match (e.g. `claude`, `aider`).
#[cfg(target_os = "linux")]
pub fn detect(repos: &[String], programs: &[String]) -> Vec<RunningAgent> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return out;
    };
    let btime = boot_time();

    for entry in dir.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };

        // argv, NUL-separated.
        let Ok(raw) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
            continue;
        };
        let args: Vec<String> = raw
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect();
        let Some(argv0) = args.first() else {
            continue;
        };
        if !programs.iter().any(|p| basename(argv0) == p.as_str()) {
            continue;
        }

        // Working directory (own processes only — fine for the user's agents).
        let Ok(cwd) = std::fs::read_link(format!("/proc/{pid}/cwd")) else {
            continue;
        };
        let Some(repo) = repos.iter().find(|r| {
            let r = Path::new(r);
            cwd == r || cwd.starts_with(r)
        }) else {
            continue;
        };

        out.push(RunningAgent {
            pid,
            repo: repo.clone(),
            command: args.join(" "),
            started_unix: start_time(pid, btime),
        });
    }
    out.sort_by_key(|a| std::cmp::Reverse(a.started_unix));
    out
}

#[cfg(target_os = "macos")]
pub fn detect(repos: &[String], programs: &[String]) -> Vec<RunningAgent> {
    let mut out = Vec::new();
    let Ok(ps_out) = std::process::Command::new("ps")
        .args(["-eo", "pid=,command="])
        .output()
    else {
        return out;
    };
    let text = String::from_utf8_lossy(&ps_out.stdout);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    for line in text.lines() {
        let trimmed = line.trim();
        let Some((pid_str, cmd_str)) = trimmed.split_once(char::is_whitespace) else {
            continue;
        };
        let Ok(pid) = pid_str.trim().parse::<u32>() else {
            continue;
        };
        let cmd_str = cmd_str.trim();
        let argv0 = cmd_str.split_whitespace().next().unwrap_or("");
        if !programs.iter().any(|p| basename(argv0) == p.as_str()) {
            continue;
        }
        let Ok(lsof_out) = std::process::Command::new("lsof")
            .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])
            .output()
        else {
            continue;
        };
        let lsof_text = String::from_utf8_lossy(&lsof_out.stdout);
        let Some(cwd_str) = lsof_text.lines().find_map(|l| l.strip_prefix('n')) else {
            continue;
        };
        let cwd = Path::new(cwd_str);
        let Some(repo) = repos.iter().find(|r| {
            let r = Path::new(r);
            cwd == r || cwd.starts_with(r)
        }) else {
            continue;
        };
        out.push(RunningAgent {
            pid,
            repo: repo.clone(),
            command: cmd_str.to_string(),
            started_unix: now,
        });
    }
    out
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn detect(_repos: &[String], _programs: &[String]) -> Vec<RunningAgent> {
    let _ = Path::new("");
    let _ = basename("");
    Vec::new()
}

/// Send SIGTERM to a process (or `taskkill` on Windows).
#[cfg(not(target_os = "windows"))]
pub fn terminate(pid: u32) -> bool {
    std::process::Command::new("kill")
        .arg(pid.to_string())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
pub fn terminate(pid: u32) -> bool {
    std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{basename, starttime_ticks};

    #[test]
    fn basename_takes_trailing_segment() {
        assert_eq!(basename("/usr/bin/claude"), "claude");
        assert_eq!(basename("claude"), "claude");
        assert_eq!(basename(""), "");
    }

    #[test]
    fn starttime_ticks_parses_field_22() {
        // Synthetic /proc/<pid>/stat: `pid (comm) <fields 3..=22>`, where each
        // token's value equals its field number, so field 22 reads back as 22.
        let after_comm: String = (3..=22)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let stat = format!("1234 (bash) {after_comm}\n");
        assert_eq!(starttime_ticks(&stat), Some(22));
    }

    #[test]
    fn starttime_ticks_handles_comm_with_spaces_and_parens() {
        // A program named "(weird) name" must not break field counting.
        let after_comm: String = (3..=22)
            .map(|n| (n * 10).to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let stat = format!("42 ((weird) name) {after_comm}");
        assert_eq!(starttime_ticks(&stat), Some(220));
    }

    #[test]
    fn starttime_ticks_rejects_malformed() {
        assert_eq!(starttime_ticks("no parens here"), None);
        assert_eq!(starttime_ticks("1 (x) S 1 2 3"), None, "too few fields");
    }
}
