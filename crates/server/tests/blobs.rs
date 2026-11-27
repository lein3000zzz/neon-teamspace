//! The blob store's adapters against real stores: a directory of local disk, always, and an
//! S3-compatible server, such as SeaweedFS, when `NEON_TEST_S3_ENDPOINT` names one, which
//! `make test-s3` starts in a container. Each keeps the same promises: a blob comes back as
//! it was written, whole or a range of it, written in one request or in parts, and is gone
//! once deleted or given up on, and so are the parts of one whose writer was dropped
//! unfinished, as when its request is cut off.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt as _;
use neon_app::attachments::{BlobError, BlobStore, ObjectBlobs, PART_BYTES, S3Location};
use neon_server::storage::S3Connector;

/// Long enough for what is waited on to happen, on the slowest runner.
const PATIENCE: Duration = Duration::from_secs(20);

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

/// Starts writing a blob of more than a part, so that a part of it is in the store, and
/// drops its writer unfinished.
async fn drop_mid_way(store: &dyn BlobStore, key: &str) {
    let mut writer = store.create(key).await.expect("created");
    writer
        .write(Bytes::from(pattern(PART_BYTES + 10)))
        .await
        .expect("written");
    drop(writer);
    assert!(matches!(
        store.read(key, None).await,
        Err(BlobError::NotFound)
    ));
}

/// Every file under `dir`, however deep.
fn files_under(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).expect("listed") {
        let path = entry.expect("an entry").path();
        if path.is_dir() {
            found.extend(files_under(&path));
        } else {
            found.push(path);
        }
    }
    found
}

#[tokio::test]
async fn local_disk_keeps_blobs_as_files_in_its_directory() {
    let dir = tempfile::tempdir().expect("a directory");
    let root = dir.path().join("attachments");
    let store = ObjectBlobs::local(&root).expect("a store");
    keeps_its_promises(&store, "0193a5b20000700080000000000000a1").await;
    tokio::time::timeout(PATIENCE, async {
        while !files_under(&root).is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("nothing is left of what was given up on");

    drop_mid_way(&store, "workspace/dropped").await;
    tokio::time::timeout(PATIENCE, async {
        while !files_under(&root).is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("nothing is left of what was dropped");

    // What a server that stopped while writing a blob leaves of it goes with it.
    std::fs::create_dir_all(root.join("workspace")).expect("a folder");
    for left in ["cut#1", "cut#12"] {
        std::fs::write(root.join("workspace").join(left), b"half").expect("written");
    }
    std::fs::write(root.join("workspace").join("cut#other"), b"kept").expect("written");
    store.delete("workspace/cut").await.expect("deleted");
    assert_eq!(
        files_under(&root),
        [root.join("workspace").join("cut#other")],
        "only the parts of that blob"
    );
    std::fs::remove_file(root.join("workspace").join("cut#other")).expect("removed");

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
    assert_eq!(
        open_uploads(&location, &prefix).await,
        "",
        "aborted uploads are gone"
    );

    // Dropped mid-way, its upload is aborted, its parts with it.
    let dropped = format!("{prefix}/dropped");
    drop_mid_way(&store, &dropped).await;
    tokio::time::timeout(PATIENCE, async {
        while !open_uploads(&location, &dropped).await.is_empty() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the dropped upload is aborted");

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

/// The keys of the multipart uploads under way under `prefix` in the bucket, one a line, as
/// S3's ListMultipartUploads names them.
async fn open_uploads(location: &S3Location, prefix: &str) -> String {
    use object_store::aws::{AwsAuthorizer, AwsCredential};
    use object_store::client::{ClientOptions, HttpConnector as _, HttpRequestBody};

    let client = S3Connector
        .connect(&ClientOptions::default())
        .expect("a client");
    let encoded: String = prefix
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect();
    let mut request = axum::http::Request::builder()
        .method("GET")
        .uri(format!(
            "{}/{}?prefix={encoded}&uploads=",
            location.endpoint, location.bucket
        ))
        .body(HttpRequestBody::empty())
        .expect("a request");
    let credential = AwsCredential {
        key_id: location.access_key_id.clone(),
        secret_key: location.secret_access_key.clone(),
        token: None,
    };
    AwsAuthorizer::new(&credential, "s3", &location.region)
        .try_authorize(&mut request, None)
        .expect("signed");
    let answer = client.execute(request).await.expect("an answer");
    assert!(answer.status().is_success(), "{}", answer.status());
    let body = answer.into_body().bytes().await.expect("a body");
    let text = String::from_utf8(body.to_vec()).expect("XML");
    text.split("<Key>")
        .skip(1)
        .filter_map(|rest| rest.split("</Key>").next())
        .collect::<Vec<_>>()
        .join("\n")
}
