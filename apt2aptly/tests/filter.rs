use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicUsize, Ordering},
};

use apt2aptly::{
    filter::{PackageFilter, PackageFilters},
    load_aptly_content, DistScanner,
};
use aptly_rest::{key::AptlyKey, AptlyRest};
use aptly_rest_mock::AptlyRestMock;
use reqwest::Client;
use serde_json::json;
use sha2::{Digest, Sha256};
use sync2aptly::{PoolPackagesCache, SyncAction};
use url::Url;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

const MD5: &str = "0123456789abcdef0123456789abcdef";
const SHA1: &str = "0123456789abcdef0123456789abcdef01234567";
const SHA256: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn binary(package: &str, source: &str, version: &str) -> String {
    format!(
        "Package: {package}\n\
         Source: {source}\n\
         Version: {version}\n\
         Architecture: amd64\n\
         Filename: pool/main/{package}_{version}_amd64.deb\n\
         Size: 1000\n\
         MD5sum: {MD5}\n\
         SHA1: {SHA1}\n\
         SHA256: {SHA256}\n"
    )
}

fn source(package: &str, version: &str) -> String {
    let dsc = format!("{package}_{version}.dsc");
    format!(
        "Package: {package}\n\
         Version: {version}\n\
         Directory: pool/main/{package}\n\
         Files:\n {MD5} 100 {dsc}\n\
         Checksums-Sha1:\n {SHA1} 100 {dsc}\n\
         Checksums-Sha256:\n {SHA256} 100 {dsc}\n"
    )
}

/// Serve a dist `test` with one component, `main`, and one architecture,
/// `amd64`
async fn serve_apt_repo(packages: &[String], sources: &[String]) -> MockServer {
    let server = MockServer::start().await;
    let packages = packages.join("\n");
    let sources = sources.join("\n");

    let entry = |content: &str, path: &str| {
        format!(
            " {:x} {} {path}\n",
            Sha256::digest(content.as_bytes()),
            content.len()
        )
    };
    let release = format!(
        "Codename: test\n\
         Architectures: amd64\n\
         Components: main\n\
         SHA256:\n{}{}",
        entry(&packages, "main/binary-amd64/Packages"),
        entry(&sources, "main/source/Sources"),
    );

    Mock::given(method("GET"))
        .and(path("/dists/test/InRelease"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    for (file, content) in [
        ("Release", release),
        ("main/binary-amd64/Packages", packages),
        ("main/source/Sources", sources),
    ] {
        Mock::given(method("GET"))
            .and(path(format!("/dists/test/{file}")))
            .respond_with(ResponseTemplate::new(200).set_body_string(content))
            .mount(&server)
            .await;
    }

    server
}

/// Start an aptly mock with a repo `test-main` holding packages with the
/// given keys
async fn start_aptly(keys: &[&str]) -> AptlyRestMock {
    let packages: Vec<_> = keys
        .iter()
        .map(|key| {
            let key: AptlyKey = key.parse().unwrap();
            json!({
                "Key": key.to_string(),
                "Package": key.package(),
                "Version": key.version().to_string(),
                "Architecture": key.arch(),
            })
        })
        .collect();
    let data = json!({
        "repositories": [{
            "Name": "test-main",
            "Comment": "",
            "DefaultDistribution": "test",
            "DefaultComponent": "main",
        }],
        "contents": [{"repository": "test-main", "packages": keys}],
        "packages": packages,
    });

    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let data_path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "apt2aptly-filter-{}-{}.json",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&data_path, data.to_string()).unwrap();

    let mock = AptlyRestMock::start().await;
    mock.load_data(&data_path);
    std::fs::remove_file(&data_path).unwrap();
    mock
}

/// Sync the apt repo into aptly with the given filters, returning the
/// changes as `+ARCH NAME VERSION` and `-ARCH NAME VERSION`
async fn sync(skip: &[&str], drop: &[&str]) -> BTreeSet<String> {
    let apt = serve_apt_repo(
        &[
            binary("libelf1", "elfutils", "1.0-2"),
            binary("elfutils", "elfutils", "1.0-2"),
            binary("foo", "foo", "1.0"),
        ],
        &[source("elfutils", "1.0-2"), source("foo", "1.0")],
    )
    .await;
    let mock = start_aptly(&[
        "Pamd64 libelf1 1.0-1 1111111111111111",
        "Pamd64 elfutils 1.0-1 2222222222222222",
        "Psource elfutils 1.0-1 3333333333333333",
        "Pamd64 bar 1.0 4444444444444444",
    ])
    .await;
    let aptly = AptlyRest::new(mock.url());

    let parse = |filters: &[&str]| -> Vec<PackageFilter> {
        filters.iter().map(|f| f.parse().unwrap()).collect()
    };
    let filters = PackageFilters {
        skip: parse(skip),
        drop: parse(drop),
    };

    let scanner = DistScanner::new(
        Client::new(),
        Url::parse(&format!("{}/", apt.uri())).unwrap(),
        "test",
    )
    .await
    .unwrap();
    let aptly_content = load_aptly_content(&aptly, "test-main".to_owned(), "main", &filters)
        .await
        .unwrap();
    let actions = scanner
        .sync_component(
            "main",
            aptly.clone(),
            aptly_content,
            PoolPackagesCache::new(aptly),
            &filters,
        )
        .await
        .unwrap();

    let describe = |sign: char, key: &AptlyKey| {
        format!("{sign}{} {} {}", key.arch(), key.package(), key.version())
    };
    actions
        .actions()
        .iter()
        .map(|action| match action {
            SyncAction::AddDeb { key, .. }
            | SyncAction::AddDsc { key, .. }
            | SyncAction::AddPoolPackage(key) => describe('+', key),
            SyncAction::RemoveAptly(key) => describe('-', key),
        })
        .collect()
}

const ALL_CHANGES: &[&str] = &[
    "+amd64 elfutils 1.0-2",
    "+amd64 foo 1.0",
    "+amd64 libelf1 1.0-2",
    "+source elfutils 1.0-2",
    "+source foo 1.0",
    "-amd64 bar 1.0",
    "-amd64 elfutils 1.0-1",
    "-amd64 libelf1 1.0-1",
    "-source elfutils 1.0-1",
];

fn all_changes_except(excluded: &[&str]) -> BTreeSet<String> {
    for e in excluded {
        assert!(ALL_CHANGES.contains(e), "'{e}' isn't a change");
    }
    ALL_CHANGES
        .iter()
        .filter(|c| !excluded.contains(c))
        .map(|c| c.to_string())
        .collect()
}

#[tokio::test]
async fn no_filters() {
    assert_eq!(sync(&[], &[]).await, all_changes_except(&[]));
}

#[tokio::test]
async fn skip_source() {
    assert_eq!(
        sync(&["main:elfutils=source"], &[]).await,
        all_changes_except(&["+source elfutils 1.0-2", "-source elfutils 1.0-1"])
    );
}

#[tokio::test]
async fn drop_source() {
    assert_eq!(
        sync(&[], &["main:elfutils=source"]).await,
        all_changes_except(&["+source elfutils 1.0-2"])
    );
}

#[tokio::test]
async fn skip_binary_arch() {
    assert_eq!(
        sync(&["libelf1=amd64"], &[]).await,
        all_changes_except(&["+amd64 libelf1 1.0-2", "-amd64 libelf1 1.0-1"])
    );
}

#[tokio::test]
async fn skip_binary_other_arch() {
    assert_eq!(sync(&["libelf1=arm64"], &[]).await, all_changes_except(&[]));
}

#[tokio::test]
async fn skip_name_matches_source_and_binary_only() {
    // Other binaries built from elfutils, like libelf1, are not selected
    assert_eq!(
        sync(&["elfutils"], &[]).await,
        all_changes_except(&[
            "+amd64 elfutils 1.0-2",
            "-amd64 elfutils 1.0-1",
            "+source elfutils 1.0-2",
            "-source elfutils 1.0-1",
        ])
    );
}

#[tokio::test]
async fn drop_binaries() {
    assert_eq!(
        sync(&[], &["elfutils=binaries"]).await,
        all_changes_except(&["+amd64 elfutils 1.0-2"])
    );
}

#[tokio::test]
async fn skip_other_component() {
    assert_eq!(
        sync(&["other:elfutils"], &[]).await,
        all_changes_except(&[])
    );
}
