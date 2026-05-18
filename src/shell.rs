#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_visible_bang() {
        assert_eq!(
            parse_shell_prefix("! ls"),
            Some(ShellPrefix::Visible("ls".into()))
        );
    }

    #[test]
    fn test_invisible_bang() {
        assert_eq!(
            parse_shell_prefix("!! ls"),
            Some(ShellPrefix::Invisible("ls".into()))
        );
    }

    #[test]
    fn test_no_bang() {
        assert_eq!(parse_shell_prefix("ls"), None);
    }

    #[test]
    fn test_bang_without_space() {
        assert_eq!(
            parse_shell_prefix("!ls"),
            Some(ShellPrefix::Visible("ls".into()))
        );
    }

    #[test]
    fn test_double_bang_without_space() {
        assert_eq!(
            parse_shell_prefix("!!ls"),
            Some(ShellPrefix::Invisible("ls".into()))
        );
    }

    #[test]
    fn test_block_cd() {
        assert_eq!(parse_shell_prefix("! cd /tmp"), None);
        assert_eq!(parse_shell_prefix("!! cd /tmp"), None);
        assert_eq!(parse_shell_prefix("!cd /tmp"), None);
    }
}

#[derive(Debug, PartialEq)]
pub enum ShellPrefix {
    Visible(String),
    Invisible(String),
}

pub fn parse_shell_prefix(text: &str) -> Option<ShellPrefix> {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix("!!") {
        let cmd = rest.trim().to_string();
        if cmd.is_empty() || is_forbidden(&cmd) {
            return None;
        }
        return Some(ShellPrefix::Invisible(cmd));
    }
    if let Some(rest) = trimmed.strip_prefix('!') {
        let cmd = rest.trim().to_string();
        if cmd.is_empty() || is_forbidden(&cmd) {
            return None;
        }
        return Some(ShellPrefix::Visible(cmd));
    }
    None
}

fn is_forbidden(cmd: &str) -> bool {
    let first = cmd.split_whitespace().next().unwrap_or("");
    matches!(
        first.to_ascii_lowercase().as_str(),
        "cd" | "pushd" | "popd" | "exit" | "exec"
    )
}
