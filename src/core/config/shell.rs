pub(crate) fn quote_word(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_word_preserves_literal_shell_value() {
        let value = "dir'; touch /tmp/alef-shell-word; #";
        let quoted = quote_word(value);
        let output = std::process::Command::new("sh")
            .args(["-c", &format!("printf '%s' {quoted}")])
            .output()
            .expect("shell should start");

        assert!(output.status.success());
        assert_eq!(output.stdout, value.as_bytes());
        assert!(!std::path::Path::new("/tmp/alef-shell-word").exists());
    }
}
