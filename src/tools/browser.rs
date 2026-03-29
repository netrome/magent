use std::process::{Command, Stdio};

/// Trait for browser command execution.
///
/// The real implementation shells out to `agent-browser`. Tests use a fake
/// that returns canned responses.
pub trait RunBrowser {
    fn run_browser(&self, input: &str) -> String;
}

/// Check whether the `agent-browser` CLI is available on the system.
pub fn is_available() -> bool {
    Command::new("agent-browser")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Closes any open browser session. Best-effort — errors are ignored.
pub fn close_session() {
    let _ = Command::new("agent-browser")
        .arg("close")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Real browser executor that shells out to the `agent-browser` CLI.
pub struct AgentBrowser;

const ALLOWED_COMMANDS: &[&str] = &[
    "open",
    "snapshot",
    "click",
    "type",
    "fill",
    "select",
    "press",
    "scroll",
    "wait",
    "get",
    "screenshot",
    "back",
    "close",
];

impl RunBrowser for AgentBrowser {
    fn run_browser(&self, input: &str) -> String {
        let args = parse_command(input);

        if args.is_empty() {
            return "Error: empty browser command".to_string();
        }

        if !is_allowed_command(&args) {
            return format!("Error: command '{}' is not allowed", args[0]);
        }

        match Command::new("agent-browser").args(&args).output() {
            Ok(output) => {
                if output.status.success() {
                    String::from_utf8_lossy(&output.stdout).into_owned()
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    format!("Error: {}", stderr.trim())
                }
            }
            Err(e) => format!("Error: failed to run agent-browser: {e}"),
        }
    }
}

/// Parse a browser command string into args, respecting quoted strings.
fn parse_command(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut quote_char = '"';

    for ch in input.trim().chars() {
        match ch {
            '"' | '\'' if !in_quotes => {
                in_quotes = true;
                quote_char = ch;
            }
            c if c == quote_char && in_quotes => {
                in_quotes = false;
            }
            ' ' if !in_quotes => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        args.push(current);
    }

    args
}

/// Check whether the first arg is in the allowlist.
fn is_allowed_command(args: &[String]) -> bool {
    args.first()
        .is_some_and(|cmd| ALLOWED_COMMANDS.contains(&cmd.as_str()))
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    // --- parse_command ---

    #[test]
    fn parse_command__should_split_simple_command() {
        assert_eq!(
            parse_command("open https://example.com"),
            vec!["open", "https://example.com"]
        );
    }

    #[test]
    fn parse_command__should_handle_single_word() {
        assert_eq!(parse_command("snapshot"), vec!["snapshot"]);
    }

    #[test]
    fn parse_command__should_handle_quoted_strings() {
        assert_eq!(
            parse_command("type @e5 \"hello world\""),
            vec!["type", "@e5", "hello world"]
        );
    }

    #[test]
    fn parse_command__should_handle_single_quoted_strings() {
        assert_eq!(
            parse_command("fill @e3 'search query'"),
            vec!["fill", "@e3", "search query"]
        );
    }

    #[test]
    fn parse_command__should_handle_extra_whitespace() {
        assert_eq!(parse_command("  click   @e3  "), vec!["click", "@e3"]);
    }

    #[test]
    fn parse_command__should_return_empty_for_empty_input() {
        assert!(parse_command("").is_empty());
        assert!(parse_command("   ").is_empty());
    }

    #[test]
    fn parse_command__should_handle_element_refs() {
        assert_eq!(parse_command("click @e3"), vec!["click", "@e3"]);
    }

    #[test]
    fn parse_command__should_handle_multi_word_get_commands() {
        assert_eq!(parse_command("get text @e4"), vec!["get", "text", "@e4"]);
        assert_eq!(parse_command("get title"), vec!["get", "title"]);
        assert_eq!(parse_command("get url"), vec!["get", "url"]);
    }

    // --- is_allowed_command ---

    #[test]
    fn is_allowed_command__should_allow_all_listed_commands() {
        for cmd in ALLOWED_COMMANDS {
            let args = vec![cmd.to_string()];
            assert!(is_allowed_command(&args), "'{cmd}' should be allowed");
        }
    }

    #[test]
    fn is_allowed_command__should_reject_eval() {
        let args = vec!["eval".to_string(), "document.title".to_string()];
        assert!(!is_allowed_command(&args));
    }

    #[test]
    fn is_allowed_command__should_reject_network() {
        let args = vec!["network".to_string(), "route".to_string()];
        assert!(!is_allowed_command(&args));
    }

    #[test]
    fn is_allowed_command__should_reject_cookies_set() {
        let args = vec!["cookies".to_string(), "set".to_string()];
        assert!(!is_allowed_command(&args));
    }

    #[test]
    fn is_allowed_command__should_reject_set_credentials() {
        let args = vec!["set".to_string(), "credentials".to_string()];
        assert!(!is_allowed_command(&args));
    }

    #[test]
    fn is_allowed_command__should_reject_empty_args() {
        assert!(!is_allowed_command(&[]));
    }

    // --- AgentBrowser::execute (allowlist enforcement) ---
    // These tests verify that the real impl rejects disallowed commands
    // *before* trying to shell out (so they work without agent-browser installed).

    #[test]
    fn execute__should_reject_disallowed_command() {
        let browser = AgentBrowser;
        let result = browser.run_browser("eval document.title");
        assert_eq!(result, "Error: command 'eval' is not allowed");
    }

    #[test]
    fn execute__should_reject_empty_input() {
        let browser = AgentBrowser;
        assert_eq!(browser.run_browser(""), "Error: empty browser command");
        assert_eq!(browser.run_browser("   "), "Error: empty browser command");
    }

    #[test]
    fn execute__should_reject_network_route() {
        let browser = AgentBrowser;
        let result = browser.run_browser("network route **/* https://mock.example.com");
        assert_eq!(result, "Error: command 'network' is not allowed");
    }

    #[test]
    fn execute__should_reject_storage_manipulation() {
        let browser = AgentBrowser;
        let result = browser.run_browser("storage local set key value");
        assert_eq!(result, "Error: command 'storage' is not allowed");
    }

    #[test]
    fn execute__should_reject_upload() {
        let browser = AgentBrowser;
        let result = browser.run_browser("upload @e3 /path/to/file");
        assert_eq!(result, "Error: command 'upload' is not allowed");
    }

    #[test]
    fn execute__should_reject_set_headers() {
        let browser = AgentBrowser;
        let result = browser.run_browser("set headers X-Custom value");
        assert_eq!(result, "Error: command 'set' is not allowed");
    }
}
