//! Spawn external programs from `{path}`-templated command strings (IDE /
//! terminal coding agent). Children are launched detached from the UI.

use std::process::{Child, Command, Stdio};

/// Spawn `tokens[0]` with the remaining tokens as args, detached, in `dir`.
fn spawn_tokens(tokens: Vec<String>, dir: &str) -> Result<Child, String> {
    let mut iter = tokens.into_iter();
    let program = iter.next().ok_or("empty command template")?;
    let args: Vec<String> = iter.collect();

    Command::new(&program)
        .args(&args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to launch `{program}`: {e}"))
}

/// Substitute `{path}` into the template and spawn the program detached in
/// `path`, returning the child handle. Each whitespace-separated token is
/// substituted, so both `code {path}` and `term --cwd={path} -- claude` work.
pub fn spawn(template: &str, path: &str) -> Result<Child, String> {
    let tokens = template
        .split_whitespace()
        .map(|t| t.replace("{path}", path))
        .collect();
    spawn_tokens(tokens, path)
}

/// Spawn the agent command with a task prompt appended: the `dispatch_args`
/// template's tokens are added after `template`'s, with `{prompt}` substituted
/// *after* tokenizing — so a multi-word prompt stays one argv entry (the
/// default `{prompt}` becomes `claude "fix the tests"`, not four args).
pub fn spawn_with_prompt(
    template: &str,
    dispatch_args: &str,
    path: &str,
    prompt: &str,
) -> Result<Child, String> {
    spawn_tokens(dispatch_tokens(template, dispatch_args, path, prompt), path)
}

/// Build the argv for [`spawn_with_prompt`] (pure, so it's unit-testable).
fn dispatch_tokens(template: &str, dispatch_args: &str, path: &str, prompt: &str) -> Vec<String> {
    template
        .split_whitespace()
        .map(|t| t.replace("{path}", path))
        .chain(
            dispatch_args
                .split_whitespace()
                .map(|t| t.replace("{path}", path).replace("{prompt}", prompt)),
        )
        .collect()
}

/// Fire-and-forget launch (IDE etc.).
pub fn launch(template: &str, path: &str) -> Result<(), String> {
    spawn(template, path).map(|_| ())
}

/// Open a folder path or URL in the system default handler (xdg-open), detached.
pub fn open(target: &str) -> Result<(), String> {
    Command::new("xdg-open")
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("failed to open `{target}`: {e}"))
}

/// Open the folder containing `path` in the system file manager.
/// If `path` is an existing file, first attempts to highlight/select it via D-Bus
/// `org.freedesktop.FileManager1.ShowItems`. Falls back to `open` on the parent directory.
pub fn open_location(path: &str) -> Result<(), String> {
    let p = std::path::Path::new(path);
    if p.is_dir() {
        return open(path);
    }
    let parent = p.parent().unwrap_or(p);
    let parent_str = parent.to_string_lossy().to_string();

    if p.is_file() {
        let abs = if p.is_absolute() {
            p.to_path_buf()
        } else if let Ok(canon) = p.canonicalize() {
            canon
        } else {
            p.to_path_buf()
        };
        let uri = format!("file://{}", abs.to_string_lossy());
        let res = Command::new("dbus-send")
            .args([
                "--session",
                "--dest=org.freedesktop.FileManager1",
                "--type=method_call",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
                &format!("array:string:{uri}"),
                "string:",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if let Ok(status) = res {
            if status.success() {
                return Ok(());
            }
        }
    }

    open(&parent_str)
}

#[cfg(test)]
mod tests {
    use super::dispatch_tokens;

    #[test]
    fn prompt_stays_one_token() {
        let argv = dispatch_tokens(
            "kitty --working-directory {path} -e claude",
            "{prompt}",
            "/repo",
            "fix the flaky tests",
        );
        assert_eq!(
            argv,
            vec![
                "kitty",
                "--working-directory",
                "/repo",
                "-e",
                "claude",
                "fix the flaky tests",
            ]
        );
    }

    #[test]
    fn dispatch_args_can_carry_flags_and_path() {
        let argv = dispatch_tokens("claude", "--cwd {path} {prompt}", "/r", "do x");
        assert_eq!(argv, vec!["claude", "--cwd", "/r", "do x"]);
    }

    #[test]
    fn empty_dispatch_args_appends_nothing() {
        let argv = dispatch_tokens("claude", "", "/r", "ignored");
        assert_eq!(argv, vec!["claude"]);
    }
}
