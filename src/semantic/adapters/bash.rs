#[cfg(all(test, feature = "semantic-bash"))]
mod tests {
    use crate::semantic::adapters::bash::{parse_bash_segments, parse_bash_segments_full};

    #[test]
    fn test_simple_command() {
        let segments = parse_bash_segments("cargo test --all");
        assert_eq!(segments, vec!["cargo test --all"]);
    }

    #[test]
    fn test_double_ampersand_splits() {
        let segments = parse_bash_segments("cargo test && echo done");
        assert_eq!(segments, vec!["cargo test", "echo done"]);
    }

    #[test]
    fn test_semicolon_splits() {
        let segments = parse_bash_segments("echo a; echo b");
        assert_eq!(segments, vec!["echo a", "echo b"]);
    }

    #[test]
    fn test_pipe_splits() {
        let segments = parse_bash_segments("cat file | grep foo | wc -l");
        assert_eq!(segments, vec!["cat file", "grep foo", "wc -l"]);
    }

    #[test]
    fn test_mixed_separators() {
        let segments = parse_bash_segments("a && b | c");
        assert_eq!(segments, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_command_substitution_is_complex() {
        let (segments, complex) = parse_bash_segments_full("echo $(rm -rf /)").unwrap();
        assert!(complex, "command substitution should be marked complex");
        assert_eq!(segments.len(), 1);
    }

    #[test]
    fn test_single_quotes_are_safe() {
        let (segments, complex) = parse_bash_segments_full("echo 'safe $(not expanded)'").unwrap();
        assert!(!complex, "single quotes should not trigger complex");
        assert_eq!(segments.len(), 1);
    }

    #[test]
    fn test_double_quotes_are_complex() {
        let (_segments, complex) =
            parse_bash_segments_full("echo \"dangerous $(expanded)\"").unwrap();
        assert!(complex, "double quotes with substitution should be complex");
    }

    #[test]
    fn test_git_commands_parse() {
        let segments = parse_bash_segments("git diff --staged && git status");
        assert_eq!(segments, vec!["git diff --staged", "git status"]);
    }

    #[test]
    fn test_parse_error_fallback() {
        let segments = parse_bash_segments("for i in");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0], "for i in");
    }
}

#[allow(dead_code)]
pub fn parse_bash_segments(command: &str) -> Vec<String> {
    parse_bash_segments_full(command)
        .map(|(segs, _)| segs)
        .unwrap_or_else(|_| vec![command.to_string()])
}

pub fn parse_bash_segments_full(command: &str) -> Result<(Vec<String>, bool), String> {
    #[cfg(feature = "semantic-bash")]
    {
        use tree_sitter::Parser;

        let lang: tree_sitter::Language = tree_sitter_bash::LANGUAGE.into();
        let mut parser = Parser::new();
        parser
            .set_language(&lang)
            .map_err(|e| format!("Failed to set bash language: {e}"))?;

        let tree = parser
            .parse(command, None)
            .ok_or("Failed to parse bash command")?;

        let root = tree.root_node();
        let source = command.as_bytes();

        let mut segments = Vec::new();
        let mut is_complex = false;

        if has_complex_constructs(root) {
            is_complex = true;
            segments.push(command.to_string());
            return Ok((segments, is_complex));
        }

        if root.has_error() {
            segments.push(command.to_string());
            return Ok((segments, is_complex));
        }

        collect_segments(root, source, &mut segments);
        if segments.is_empty() {
            segments.push(command.to_string());
        }

        Ok((segments, is_complex))
    }
    #[cfg(not(feature = "semantic-bash"))]
    {
        Ok((vec![command.to_string()], false))
    }
}

#[cfg(feature = "semantic-bash")]
fn has_complex_constructs(node: tree_sitter::Node) -> bool {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "command_substitution"
                | "process_substitution"
                | "subshell"
                | "arithmetic_expansion" => return true,
                _ => {
                    if has_complex_constructs(child) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

#[cfg(feature = "semantic-bash")]
fn collect_segments(node: tree_sitter::Node, source: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "program" | "list" => {
            for i in 0..node.named_child_count() {
                if let Some(child) = node.named_child(i) {
                    collect_segments(child, source, out);
                }
            }
        }
        "pipeline" => {
            for i in 0..node.named_child_count() {
                if let Some(child) = node.named_child(i) {
                    let text = child.utf8_text(source).unwrap_or("").trim().to_string();
                    if !text.is_empty() {
                        out.push(text);
                    }
                }
            }
        }
        "command"
        | "redirected_statement"
        | "compound_statement"
        | "if_statement"
        | "while_statement"
        | "for_statement"
        | "case_statement"
        | "function_definition"
        | "c_style_for_statement" => {
            let text = node.utf8_text(source).unwrap_or("").trim().to_string();
            if !text.is_empty() {
                out.push(text);
            }
        }
        _ => {
            for i in 0..node.named_child_count() {
                if let Some(child) = node.named_child(i) {
                    collect_segments(child, source, out);
                }
            }
        }
    }
}
