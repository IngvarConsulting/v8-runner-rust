use std::path::Path;

pub fn format_ibcmd_failure_details(
    action: &str,
    target_kind: &str,
    target: &str,
    exit_code: i32,
    stdout: &str,
    stderr: &str,
    platform_log: Option<&str>,
    platform_log_path: Option<&Path>,
) -> String {
    format_failure_evidence(
        format!("{action} failed for {target_kind} '{target}' with exit code {exit_code}"),
        stdout,
        stderr,
        platform_log,
        platform_log_path,
    )
}

/// Улики отказа команды платформы: вывод и журнал идут за `headline`, который называет, что
/// не удалось и с каким кодом.
pub fn format_failure_evidence(
    headline: String,
    stdout: &str,
    stderr: &str,
    platform_log: Option<&str>,
    platform_log_path: Option<&Path>,
) -> String {
    let mut details = vec![headline];
    if !stdout.trim().is_empty() {
        details.push(format!("stdout: {}", stdout.trim()));
    }
    if !stderr.trim().is_empty() {
        details.push(format!("stderr: {}", stderr.trim()));
    }
    if let Some(log) = platform_log.filter(|log| !log.trim().is_empty()) {
        details.push(format!("platform log: {}", log.trim()));
    }
    if let Some(path) = platform_log_path {
        details.push(format!("platform log path: {}", path.display()));
    }
    details.join("; ")
}

#[cfg(test)]
mod tests {
    use super::format_ibcmd_failure_details;
    use std::path::Path;

    #[test]
    fn format_ibcmd_failure_details_includes_required_context() {
        let message = format_ibcmd_failure_details(
            "dump",
            "source-set",
            "main",
            17,
            "stdout line",
            "stderr line",
            Some("platform line"),
            Some(Path::new("/tmp/platform.log")),
        );

        assert!(message.contains("dump failed for source-set 'main' with exit code 17"));
        assert!(message.contains("stdout: stdout line"));
        assert!(message.contains("stderr: stderr line"));
        assert!(message.contains("platform log: platform line"));
        assert!(message.contains("platform log path: /tmp/platform.log"));
    }
}
