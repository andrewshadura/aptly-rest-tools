use std::{
    process::Output,
    sync::atomic::{AtomicUsize, Ordering},
};

use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::process::Command;
use wiremock::{
    matchers::{method, path, path_regex},
    Mock, MockServer, Request, ResponseTemplate,
};

/// Serve an apt repo with dist `v2023/snapshots/S1`, holding one empty
/// component, `main`, for `amd64`
async fn serve_apt_repo() -> MockServer {
    let server = MockServer::start().await;
    let release = format!(
        "Codename: v2023\n\
         Architectures: amd64\n\
         Components: main\n\
         SHA256:\n {:x} 0 main/binary-amd64/Packages\n {:x} 0 main/source/Sources\n",
        Sha256::digest(b""),
        Sha256::digest(b""),
    );

    let dist = "/dists/v2023/snapshots/S1";
    Mock::given(method("GET"))
        .and(path(format!("{dist}/InRelease")))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    for (file, content) in [
        ("Release", release.as_str()),
        ("main/binary-amd64/Packages", ""),
        ("main/source/Sources", ""),
    ] {
        Mock::given(method("GET"))
            .and(path(format!("{dist}/{file}")))
            .respond_with(ResponseTemplate::new(200).set_body_string(content))
            .mount(&server)
            .await;
    }

    server
}

/// Serve the aptly API calls apt2aptly makes up to syncing the snapshot, with
/// the snapshot `main-S1` existing and published at `prefix/v2023/snapshots/S1`
async fn serve_aptly() -> MockServer {
    let server = MockServer::start().await;
    let ok = || ResponseTemplate::new(200).set_body_json(json!({}));

    Mock::given(method("GET"))
        .and(path("/api/publish"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "Storage": "",
            "Prefix": "prefix",
            "Distribution": "v2023/snapshots/S1",
            "SourceKind": "snapshot",
            "Sources": [{"Name": "main-S1", "Component": "main"}],
            "Architectures": ["amd64", "source"],
            "Label": "",
            "Origin": "",
            "NotAutomatic": "",
            "ButAutomaticUpgrades": "",
            "AcquireByHash": false,
        }])))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path_regex("^/api/publish/prefix/"))
        .respond_with(ok())
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/snapshots/main-S1"))
        .respond_with(ok())
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/repos/test-main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "Name": "test-main",
            "Comment": "",
            "DefaultDistribution": "",
            "DefaultComponent": "",
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/repos/test-main/packages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;

    server
}

async fn apt2aptly(aptly: &MockServer, apt: &MockServer, args: &[&str]) -> Output {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let snapshots = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "apt2aptly-republish-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&snapshots, "S1\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_apt2aptly"))
        .arg("--api-url")
        .arg(aptly.uri())
        .arg("test-{component}")
        .arg(format!("{}/", apt.uri()))
        .arg("v2023")
        .arg("--publish-to=prefix")
        .arg("--apt-snapshots")
        .arg(&snapshots)
        .arg("--aptly-snapshot-template={component}-{apt-snapshot}")
        .args(args)
        .env_remove("APTLY_API_TOKEN")
        .output()
        .await
        .unwrap();
    std::fs::remove_file(&snapshots).unwrap();
    output
}

fn is_delete(request: &Request, prefix: &str) -> bool {
    request.method.as_str() == "DELETE" && request.url.path().starts_with(prefix)
}

#[tokio::test]
async fn publish_deleted_before_snapshot() {
    let apt = serve_apt_repo().await;
    let aptly = serve_aptly().await;

    // The run stops after the deletions, at creating the snapshot, which
    // isn't mocked; only the order of the deletions matters here
    let output = apt2aptly(
        &aptly,
        &apt,
        &[
            "--delete-existing-snapshot",
            "--delete-existing-snapshot-publish",
        ],
    )
    .await;
    let stderr = String::from_utf8_lossy(&output.stderr);

    let requests = aptly.received_requests().await.unwrap();
    let publish = requests
        .iter()
        .position(|r| is_delete(r, "/api/publish/prefix/"))
        .unwrap_or_else(|| panic!("publish not deleted: {stderr}"));
    let snapshot = requests
        .iter()
        .position(|r| is_delete(r, "/api/snapshots/main-S1"))
        .unwrap_or_else(|| panic!("snapshot not deleted: {stderr}"));
    assert!(
        publish < snapshot,
        "snapshot deleted before its publish: {stderr}"
    );
}

#[tokio::test]
async fn publish_kept_without_delete_existing_snapshot_publish() {
    let apt = serve_apt_repo().await;
    let aptly = serve_aptly().await;

    let output = apt2aptly(&aptly, &apt, &["--delete-existing-snapshot"]).await;
    let stderr = String::from_utf8_lossy(&output.stderr);

    let requests = aptly.received_requests().await.unwrap();
    assert!(
        !requests
            .iter()
            .any(|r| is_delete(r, "/api/publish/prefix/")),
        "publish deleted: {stderr}"
    );
}
