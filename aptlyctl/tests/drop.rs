use std::process::{Output, Stdio};

use serde_json::json;
use tokio::{io::AsyncWriteExt, process::Command};
use wiremock::{
    matchers::{body_json, method, path, query_param, query_param_is_missing},
    Mock, MockServer, ResponseTemplate,
};

async fn aptlyctl_with_stdin(server: &MockServer, args: &[&str], input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_aptlyctl"))
        .arg("--api-url")
        .arg(server.uri())
        .args(args)
        .env_remove("APTLY_API_TOKEN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(input.as_bytes()).await.unwrap();
    drop(stdin);

    child.wait_with_output().await.unwrap()
}

async fn aptlyctl(server: &MockServer, args: &[&str]) -> Output {
    aptlyctl_with_stdin(server, args, "").await
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "aptlyctl failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn published(prefix: &str, distribution: &str) -> serde_json::Value {
    json!({
        "Storage": "",
        "Prefix": prefix,
        "Distribution": distribution,
        "SourceKind": "snapshot",
        "Sources": [{"Name": format!("{distribution}-snap"), "Component": "main"}],
        "Architectures": ["amd64"],
        "Label": "",
        "Origin": "",
        "NotAutomatic": "",
        "ButAutomaticUpgrades": "",
        "AcquireByHash": false,
    })
}

#[tokio::test]
async fn snapshot_drop_single() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/snapshots/snap1"))
        .and(query_param("force", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;

    assert_success(&aptlyctl(&server, &["snapshot", "drop", "--force", "snap1"]).await);
}

#[tokio::test]
async fn snapshot_drop_many() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/snapshots"))
        .and(body_json(
            json!({"Names": ["snap1", "snap2"], "Force": false}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;

    assert_success(&aptlyctl(&server, &["snapshot", "drop", "snap1", "snap2"]).await);
}

#[tokio::test]
async fn publish_drop_single() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/publish/apertis/v2023"))
        .and(query_param_is_missing("SkipCleanup"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;

    assert_success(&aptlyctl(&server, &["publish", "drop", "apertis", "v2023"]).await);
}

#[tokio::test]
async fn publish_drop_single_skip_cleanup() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/publish/apertis/v2023"))
        .and(query_param("SkipCleanup", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;

    assert_success(
        &aptlyctl(
            &server,
            &["publish", "drop", "--skip-cleanup", "apertis", "v2023"],
        )
        .await,
    );
}

#[tokio::test]
async fn publish_drop_many() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/publish/apertis"))
        .and(body_json(json!({
            "Distributions": ["v2023/snapshots/1", "v2023/snapshots/2"],
            "Force": false,
            "SkipCleanup": true,
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;

    assert_success(
        &aptlyctl(
            &server,
            &[
                "publish",
                "drop",
                "--skip-cleanup",
                "apertis",
                "v2023/snapshots/1",
                "v2023/snapshots/2",
            ],
        )
        .await,
    );
}

#[tokio::test]
async fn publish_drop_many_needs_skip_cleanup() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(0)
        .mount(&server)
        .await;

    let output = aptlyctl(
        &server,
        &[
            "publish",
            "drop",
            "apertis",
            "v2023/snapshots/1",
            "v2023/snapshots/2",
        ],
    )
    .await;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--skip-cleanup"));
}

#[tokio::test]
async fn publish_drop_many_ignore_if_missing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/publish"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            published("apertis", "v2023/snapshots/1"),
            published("apertis", "v2023/snapshots/3"),
            published("other", "v2023/snapshots/2"),
        ])))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/publish/apertis"))
        .and(body_json(json!({
            "Distributions": ["v2023/snapshots/1", "v2023/snapshots/3"],
            "Force": false,
            "SkipCleanup": true,
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;

    assert_success(
        &aptlyctl(
            &server,
            &[
                "publish",
                "drop",
                "--skip-cleanup",
                "--ignore-if-missing",
                "apertis",
                "v2023/snapshots/1",
                "v2023/snapshots/2",
                "v2023/snapshots/3",
            ],
        )
        .await,
    );
}

#[tokio::test]
async fn snapshot_drop_many_from_stdin() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/snapshots"))
        .and(body_json(
            json!({"Names": ["snap1", "snap2"], "Force": false}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;

    assert_success(
        &aptlyctl_with_stdin(&server, &["snapshot", "drop", "-"], "snap1\n\n  snap2  \n").await,
    );
}

#[tokio::test]
async fn snapshot_drop_single_from_stdin() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/snapshots/snap1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;

    assert_success(&aptlyctl_with_stdin(&server, &["snapshot", "drop", "-"], "snap1\n").await);
}

#[tokio::test]
async fn snapshot_drop_empty_stdin() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(0)
        .mount(&server)
        .await;

    let output = aptlyctl_with_stdin(&server, &["snapshot", "drop", "-"], "\n").await;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("No names given on stdin"));
}

#[tokio::test]
async fn publish_drop_many_from_stdin() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/publish/apertis"))
        .and(body_json(json!({
            "Distributions": ["v2023/snapshots/1", "v2023/snapshots/2"],
            "Force": false,
            "SkipCleanup": true,
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;

    assert_success(
        &aptlyctl_with_stdin(
            &server,
            &["publish", "drop", "--skip-cleanup", "apertis", "-"],
            "v2023/snapshots/1\nv2023/snapshots/2\n",
        )
        .await,
    );
}
