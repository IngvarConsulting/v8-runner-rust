mod support;

#[test]
fn web_status_of_a_declared_cluster_writes_nothing() {
    let project = support::temp_workspace();
    std::fs::write(
        project.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set: []\n",
    )
    .unwrap();
    std::fs::write(
        project.path().join("v8project.local.yaml"),
        "infobases:\n  origin:\n    cluster:\n      managed: {}\n    web:\n      managed: true\n      wsdir: demo\n",
    )
    .unwrap();
    support::hold_workspace_lock(&project.path().join(".build/v8-runner"));
    let output = support::v8_runner_command()
        .current_dir(project.path())
        .env_remove("V8TR_CONFIG")
        .args(["--json-message", "web", "status"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let reply: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(reply["command"], "web status");
    assert_eq!(reply["data"]["state"], "not_running");
    assert!(!project.path().join(".local").exists());
}

#[test]
fn web_preview_reports_its_apache_and_private_files_without_writing() {
    let project = support::temp_workspace();
    std::fs::write(project.path().join("httpd"), "").unwrap();
    std::fs::write(
        project.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set: []\ntools:\n  apache:\n    path: httpd\n",
    )
    .unwrap();
    std::fs::write(project.path().join("v8project.local.yaml"),
        "infobases:\n  origin:\n    cluster:\n      managed: {}\n    web:\n      managed: true\n      wsdir: demo\n").unwrap();
    let output = support::v8_runner_command()
        .current_dir(project.path())
        .env_remove("V8TR_CONFIG")
        .args(["--json-message", "--dry-run", "web", "start"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let reply: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(reply["data"]["action"], "planned");
    assert!(reply["data"]["program"]
        .as_str()
        .unwrap()
        .ends_with("httpd"));
    assert!(reply["data"]["config_path"]
        .as_str()
        .unwrap()
        .ends_with("httpd.conf"));
    assert!(!project.path().join(".local").exists());
}
