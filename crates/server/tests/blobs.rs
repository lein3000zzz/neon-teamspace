//! The blob store's adapters against real stores: a directory of local disk, always, and an
//! S3-compatible server, such as SeaweedFS, when `NEON_TEST_S3_ENDPOINT` names one, which
//! `make test-s3` starts in a container. Each keeps the same promises: a blob comes back as
//! it was written, whole or a range of it, written in one request or in parts, and is gone
//! once deleted or given up on.

use std::sync::Arc;

use bytes::Bytes;
use futures_util::StreamExt as _;
use neon_app::attachments::{BlobError, BlobStore, ObjectBlobs, PART_BYTES, S3Location};
use neon_server::storage::S3Connector;

/// Bytes that differ from one place to the next, so a part out of order would show.
fn pattern(len: usize) -> Vec<u8> {
    (0..len)
        .map(|n| u8::try_from((n * 7 + n / 251) % 256).expect("a byte"))
        .collect()
}

async fn read_all(
    store: &dyn BlobStore,
    key: &str,
    range: Option<std::ops::Range<u64>>,
) -> Vec<u8> {
    let read = store.read(key, range).await.expect("read");
    let mut bytes = Vec::new();
    let mut stream = read.bytes;
    while let Some(chunk) = stream.next().await {
        bytes.extend_from_slice(&chunk.expect("a chunk"));
    }
    bytes
}

/// Writes `content` in chunks of `chunk` bytes.
async fn write(store: &dyn BlobStore, key: &str, content: &[u8], chunk: usize) {
    let mut writer = store.create(key).await.expect("created");
    for part in content.chunks(chunk.max(1)) {
        writer
            .write(Bytes::copy_from_slice(part))
            .await
            .expect("written");
    }
    writer.finish().await.expect("finished");
}

/// What every adapter must do, under keys that start with `prefix`.
async fn keeps_its_promises(store: &dyn BlobStore, prefix: &str) {
    // A small blob goes up in one request.
    let small = pattern(5000);
    let key = format!("{prefix}/small");
    write(store, &key, &small, 700).await;
    assert_eq!(read_all(store, &key, None).await, small);
    let read = store.read(&key, Some(10..20)).await.expect("a range");
    assert_eq!((read.size, read.range.clone()), (5000, 10..20));
    assert_eq!(read_all(store, &key, Some(10..20)).await, &small[10..20]);
    assert_eq!(
        read_all(store, &key, Some(4990..5000)).await,
        &small[4990..]
    );

    // A large one goes up in parts, a few at once, in order.
    let large = pattern(2 * PART_BYTES + 12345);
    let key_large = format!("{prefix}/large");
    write(store, &key_large, &large, 1024 * 1024 + 17).await;
    assert_eq!(read_all(store, &key_large, None).await, large);
    let middle = PART_BYTES as u64 - 5..PART_BYTES as u64 + 5;
    assert_eq!(
        read_all(store, &key_large, Some(middle.clone())).await,
        &large[PART_BYTES - 5..PART_BYTES + 5]
    );

    // An empty blob is a blob.
    let key_empty = format!("{prefix}/empty");
    write(store, &key_empty, &[], 1).await;
    assert!(read_all(store, &key_empty, None).await.is_empty());

    // Deleting is for good, and twice is no harm.
    for key in [&key, &key_large, &key_empty] {
        store.delete(key).await.expect("deleted");
        assert!(matches!(
            store.read(key, None).await,
            Err(BlobError::NotFound)
        ));
        store.delete(key).await.expect("deleted again");
    }

    // A blob given up on is not there, whether a part of it went up or not.
    for (len, name) in [
        (100, "abandoned-small"),
        (PART_BYTES + 1, "abandoned-large"),
    ] {
        let key = format!("{prefix}/{name}");
        let mut writer = store.create(&key).await.expect("created");
        writer
            .write(Bytes::from(pattern(len)))
            .await
            .expect("written");
        writer.abort().await.expect("aborted");
        assert!(matches!(
            store.read(&key, None).await,
            Err(BlobError::NotFound)
        ));
    }
}

#[tokio::test]
async fn local_disk_keeps_blobs_as_files_in_its_directory() {
    let dir = tempfile::tempdir().expect("a directory");
    let root = dir.path().join("attachments");
    let store = ObjectBlobs::local(&root).expect("a store");
    keeps_its_promises(&store, "0193a5b20000700080000000000000a1").await;

    write(&store, "workspace/file", b"bytes", 2).await;
    assert_eq!(
        std::fs::read(root.join("workspace").join("file")).expect("a file on disk"),
        b"bytes"
    );
    store.delete("workspace/file").await.expect("deleted");
    assert!(!root.join("workspace").join("file").exists());
}

/// The S3-compatible server the environment names, if any: its endpoint, bucket, region and
/// keys.
fn s3() -> Option<S3Location> {
    let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    Some(S3Location {
        endpoint: var("NEON_TEST_S3_ENDPOINT")?,
        bucket: var("NEON_TEST_S3_BUCKET").unwrap_or_else(|| "neon-test".to_owned()),
        region: var("NEON_TEST_S3_REGION").unwrap_or_else(|| "us-east-1".to_owned()),
        access_key_id: var("NEON_TEST_S3_ACCESS_KEY_ID")?,
        secret_access_key: var("NEON_TEST_S3_SECRET_ACCESS_KEY")?,
        path_style: true,
    })
}

#[tokio::test]
async fn an_s3_compatible_store_keeps_blobs_in_its_bucket() {
    let Some(location) = s3() else {
        eprintln!(
            "NEON_TEST_S3_ENDPOINT is unset: skipped the S3 adapter's test, which make test-s3 runs"
        );
        return;
    };
    let store = ObjectBlobs::s3(&location, Arc::new(S3Connector)).expect("a store");
    let prefix = format!("test/{}", uuid::Uuid::now_v7().simple());
    keeps_its_promises(&store, &prefix).await;

    // Wrong keys are the store's refusal, never a blob.
    let wrong = S3Location {
        secret_access_key: "not-the-secret".to_owned(),
        ..location
    };
    let refused = ObjectBlobs::s3(&wrong, Arc::new(S3Connector)).expect("a store");
    assert!(matches!(
        refused.read(&format!("{prefix}/none"), None).await,
        Err(BlobError::Store(_))
    ));
}
