use aptly_rest::{key::AptlyKey, AptlyRest};
use aptly_rest_mock::AptlyRestMock;
use debian_packaging::package_version::PackageVersion;
use sync2aptly::{
    AptlyContent, LazyVersion, OriginContentBuilder, OriginDeb, OriginLocation, PoolPackagesCache,
    SyncAction, SyncOptions,
};

// A real-world rollback: the epoch got dropped, so the new version is lower
const NEWER: &str = "1:1.0-1";
const OLDER: &str = "1.0-2";
const NEWER_HASH: &str = "1111111111111111";
const OLDER_HASH: &str = "2222222222222222";

fn version(v: &str) -> PackageVersion {
    PackageVersion::parse(v).unwrap()
}

fn key(arch: &str, package: &str, v: &str, hash: &str) -> AptlyKey {
    AptlyKey::new(arch.into(), package.into(), version(v), hash.into())
}

fn deb(arch: &str, package: &str, v: &str, hash: &str) -> OriginDeb {
    OriginDeb {
        package: package.into(),
        version: LazyVersion::with_value(version(v)),
        architecture: arch.into(),
        location: OriginLocation::Path(format!("/origin/{package}_{v}_{arch}.deb").into()),
        from_source: "policykit-1".into(),
        aptly_hash: hash.into(),
    }
}

async fn run_sync(
    origin: Vec<OriginDeb>,
    aptly_keys: Vec<AptlyKey>,
    options: SyncOptions,
) -> Vec<SyncAction> {
    let mock = AptlyRestMock::start().await;
    let aptly = AptlyRest::new(mock.url());

    let mut builder = OriginContentBuilder::new();
    for d in origin {
        builder.add_deb(d);
    }

    let mut aptly_content = AptlyContent::new_empty("test".to_owned());
    for k in aptly_keys {
        aptly_content.add_key(k);
    }

    let mut actions = sync2aptly::sync(
        builder.build(),
        aptly.clone(),
        aptly_content,
        PoolPackagesCache::new(aptly),
        &options,
    )
    .await
    .unwrap()
    .actions()
    .to_vec();
    actions.sort();
    actions
}

fn added_keys(actions: &[SyncAction]) -> Vec<&AptlyKey> {
    actions
        .iter()
        .filter_map(|a| match a {
            SyncAction::AddDeb { key, .. } => Some(key),
            _ => None,
        })
        .collect()
}

fn removed_keys(actions: &[SyncAction]) -> Vec<&AptlyKey> {
    actions
        .iter()
        .filter_map(|a| match a {
            SyncAction::RemoveAptly(key) => Some(key),
            _ => None,
        })
        .collect()
}

const ALLOW: SyncOptions = SyncOptions {
    allow_downgrades: true,
};
const DENY: SyncOptions = SyncOptions {
    allow_downgrades: false,
};

#[tokio::test]
async fn arch_specific_downgrade_allowed() {
    let actions = run_sync(
        vec![deb("amd64", "libpolkit-agent-1-0", OLDER, OLDER_HASH)],
        vec![key("amd64", "libpolkit-agent-1-0", NEWER, NEWER_HASH)],
        ALLOW,
    )
    .await;

    assert_eq!(
        added_keys(&actions),
        [&key("amd64", "libpolkit-agent-1-0", OLDER, OLDER_HASH)]
    );
    assert_eq!(
        removed_keys(&actions),
        [&key("amd64", "libpolkit-agent-1-0", NEWER, NEWER_HASH)]
    );
}

#[tokio::test]
async fn arch_specific_downgrade_denied() {
    let actions = run_sync(
        vec![deb("amd64", "libpolkit-agent-1-0", OLDER, OLDER_HASH)],
        vec![key("amd64", "libpolkit-agent-1-0", NEWER, NEWER_HASH)],
        DENY,
    )
    .await;

    assert!(actions.is_empty(), "{actions:?}");
}

#[tokio::test]
async fn arch_specific_downgrade_to_package_in_aptly() {
    let actions = run_sync(
        vec![deb("amd64", "libpolkit-agent-1-0", OLDER, OLDER_HASH)],
        vec![
            key("amd64", "libpolkit-agent-1-0", OLDER, OLDER_HASH),
            key("amd64", "libpolkit-agent-1-0", NEWER, NEWER_HASH),
        ],
        ALLOW,
    )
    .await;

    assert!(added_keys(&actions).is_empty(), "{actions:?}");
    assert_eq!(
        removed_keys(&actions),
        [&key("amd64", "libpolkit-agent-1-0", NEWER, NEWER_HASH)]
    );
}

#[tokio::test]
async fn arch_specific_unchanged() {
    let actions = run_sync(
        vec![deb("amd64", "libpolkit-agent-1-0", OLDER, OLDER_HASH)],
        vec![key("amd64", "libpolkit-agent-1-0", OLDER, OLDER_HASH)],
        ALLOW,
    )
    .await;

    assert!(actions.is_empty(), "{actions:?}");
}

#[tokio::test]
async fn arch_indep_downgrade_allowed() {
    let actions = run_sync(
        vec![deb("all", "policykit-1-doc", OLDER, OLDER_HASH)],
        vec![key("all", "policykit-1-doc", NEWER, NEWER_HASH)],
        ALLOW,
    )
    .await;

    assert_eq!(
        added_keys(&actions),
        [&key("all", "policykit-1-doc", OLDER, OLDER_HASH)]
    );
    assert_eq!(
        removed_keys(&actions),
        [&key("all", "policykit-1-doc", NEWER, NEWER_HASH)]
    );
}

#[tokio::test]
async fn arch_indep_downgrade_denied() {
    let actions = run_sync(
        vec![deb("all", "policykit-1-doc", OLDER, OLDER_HASH)],
        vec![key("all", "policykit-1-doc", NEWER, NEWER_HASH)],
        DENY,
    )
    .await;

    assert!(actions.is_empty(), "{actions:?}");
}

#[tokio::test]
async fn arch_indep_downgrade_to_package_in_aptly() {
    let actions = run_sync(
        vec![deb("all", "policykit-1-doc", OLDER, OLDER_HASH)],
        vec![
            key("all", "policykit-1-doc", OLDER, OLDER_HASH),
            key("all", "policykit-1-doc", NEWER, NEWER_HASH),
        ],
        ALLOW,
    )
    .await;

    assert!(added_keys(&actions).is_empty(), "{actions:?}");
    assert_eq!(
        removed_keys(&actions),
        [&key("all", "policykit-1-doc", NEWER, NEWER_HASH)]
    );
}
