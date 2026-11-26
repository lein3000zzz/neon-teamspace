//! The [`BlobStore`] port, where attachments' bytes are kept, and its adapter over the
//! `object_store` crate: [`ObjectBlobs`], on a directory of local disk or in a bucket of any
//! S3-compatible store.
//!
//! A blob is written as a stream, a part at a time, and never held whole in memory: a blob
//! that ends within its first part goes up in one request, and a longer one as a multipart
//! upload, a few parts at once. Reading streams it too, the whole blob or a range of it.
//! Deleting a blob that is not there is no error, so a deletion tried twice does no harm.

use std::fmt;
use std::ops::Range;
use std::path::Path as LocalPath;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream::{BoxStream, StreamExt as _, TryStreamExt as _};
use object_store::aws::AmazonS3Builder;
use object_store::client::{ClientOptions, HttpClient, HttpConnector};
use object_store::local::LocalFileSystem;
use object_store::path::Path;
use object_store::{
    GetOptions, GetRange, MultipartUpload, ObjectStore, ObjectStoreExt as _, PutPayload,
    PutPayloadMut,
};

/// How much of a blob is sent at once: what one part of a multipart upload holds, and what
/// a blob shorter than that is sent whole as. S3 takes parts of 5 MiB and more.
pub const PART_BYTES: usize = 8 * 1024 * 1024;

/// How many parts of one blob go up at once.
const PARTS_AT_ONCE: usize = 2;

/// Why the store did not do what it was asked.
#[derive(Debug, thiserror::Error)]
pub enum BlobError {
    /// No blob is kept under the key.
    #[error("no such blob")]
    NotFound,
    /// The store did not answer, or answered with an error: it says which, never the
    /// blob's bytes.
    #[error("the blob store failed: {0}")]
    Store(String),
}

impl From<object_store::Error> for BlobError {
    fn from(error: object_store::Error) -> Self {
        match error {
            object_store::Error::NotFound { .. } => Self::NotFound,
            other => Self::Store(other.to_string()),
        }
    }
}

/// A blob, or a range of it, as it is read.
pub struct BlobRead {
    /// The whole blob's size in bytes.
    pub size: u64,
    /// The bytes the stream holds, of the blob's.
    pub range: Range<u64>,
    pub bytes: BoxStream<'static, Result<Bytes, BlobError>>,
}

impl fmt::Debug for BlobRead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlobRead")
            .field("size", &self.size)
            .field("range", &self.range)
            .finish_non_exhaustive()
    }
}

/// A blob being written. Nothing is kept unless it is finished; one dropped unfinished may
/// leave parts in the store, which aborting removes.
#[async_trait]
pub trait BlobWriter: Send {
    /// Adds bytes at the end. A part goes up once one is full.
    async fn write(&mut self, bytes: Bytes) -> Result<(), BlobError>;

    /// Sends what is left and keeps the blob.
    async fn finish(self: Box<Self>) -> Result<(), BlobError>;

    /// Gives up the blob, and what went up of it.
    async fn abort(self: Box<Self>) -> Result<(), BlobError>;
}

/// Where attachments' bytes are kept, each blob under a key the caller makes.
#[async_trait]
pub trait BlobStore: Send + Sync {
    /// Starts writing a blob under `key`, which takes the place of any blob there.
    async fn create(&self, key: &str) -> Result<Box<dyn BlobWriter>, BlobError>;

    /// Reads a blob, or `range` of it, which the caller keeps within the blob: what is past
    /// its end is left out.
    async fn read(&self, key: &str, range: Option<Range<u64>>) -> Result<BlobRead, BlobError>;

    /// Deletes a blob. One that is not there is no error.
    async fn delete(&self, key: &str) -> Result<(), BlobError>;
}

/// Where an S3-compatible store is, and how to sign in to it.
#[derive(Clone, PartialEq, Eq)]
pub struct S3Location {
    /// Its URL, such as `http://seaweedfs:8333`; empty for Amazon S3 itself.
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    /// Buckets in the URL's path, as most stores but Amazon's want, rather than in its host.
    pub path_style: bool,
}

impl fmt::Debug for S3Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3Location")
            .field("endpoint", &self.endpoint)
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("path_style", &self.path_style)
            .finish_non_exhaustive()
    }
}

/// The HTTP client S3 requests go through, which the server provides.
pub type Connector = Arc<dyn HttpConnector>;

/// A [`BlobStore`] over an [`ObjectStore`]: local disk, or an S3 bucket.
#[derive(Debug, Clone)]
pub struct ObjectBlobs {
    store: Arc<dyn ObjectStore>,
}

impl ObjectBlobs {
    pub fn new(store: Arc<dyn ObjectStore>) -> Self {
        Self { store }
    }

    /// Blobs as files under `dir`, which is made if it is missing.
    pub fn local(dir: &LocalPath) -> Result<Self, BlobError> {
        std::fs::create_dir_all(dir)
            .map_err(|error| BlobError::Store(format!("cannot make {}: {error}", dir.display())))?;
        let store = LocalFileSystem::new_with_prefix(dir)?.with_automatic_cleanup(true);
        Ok(Self::new(Arc::new(store)))
    }

    /// Blobs in an S3 bucket, its requests made through `connector`.
    pub fn s3(location: &S3Location, connector: Connector) -> Result<Self, BlobError> {
        let mut builder = AmazonS3Builder::new()
            .with_bucket_name(&location.bucket)
            .with_region(&location.region)
            .with_access_key_id(&location.access_key_id)
            .with_secret_access_key(&location.secret_access_key)
            .with_virtual_hosted_style_request(!location.path_style)
            .with_http_connector(Shared(connector));
        if !location.endpoint.is_empty() {
            builder = builder
                .with_endpoint(&location.endpoint)
                .with_allow_http(location.endpoint.starts_with("http://"));
        }
        Ok(Self::new(Arc::new(builder.build()?)))
    }
}

/// A shared connector, as the builder takes one by value.
#[derive(Debug)]
struct Shared(Connector);

impl HttpConnector for Shared {
    fn connect(&self, options: &ClientOptions) -> object_store::Result<HttpClient> {
        self.0.connect(options)
    }
}

fn path(key: &str) -> Result<Path, BlobError> {
    Path::parse(key).map_err(|error| BlobError::Store(format!("an invalid key: {error}")))
}

#[async_trait]
impl BlobStore for ObjectBlobs {
    async fn create(&self, key: &str) -> Result<Box<dyn BlobWriter>, BlobError> {
        Ok(Box::new(ObjectWriter {
            store: Arc::clone(&self.store),
            path: path(key)?,
            buffer: PutPayloadMut::new(),
            upload: None,
        }))
    }

    async fn read(&self, key: &str, range: Option<Range<u64>>) -> Result<BlobRead, BlobError> {
        let options = GetOptions {
            range: range.map(GetRange::Bounded),
            ..GetOptions::default()
        };
        let result = self.store.get_opts(&path(key)?, options).await?;
        Ok(BlobRead {
            size: result.meta.size,
            range: result.range.clone(),
            bytes: result.into_stream().map_err(BlobError::from).boxed(),
        })
    }

    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        match self.store.delete(&path(key)?).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

/// A blob being written to an [`ObjectStore`]: its first part kept until it fills, then a
/// multipart upload.
struct ObjectWriter {
    store: Arc<dyn ObjectStore>,
    path: Path,
    buffer: PutPayloadMut,
    upload: Option<Upload>,
}

/// A multipart upload under way, with its parts in flight.
struct Upload {
    upload: Box<dyn MultipartUpload>,
    parts: tokio::task::JoinSet<object_store::Result<()>>,
}

impl Upload {
    /// Waits until fewer than `most` parts are in flight, failing with the first that
    /// failed.
    async fn settle(&mut self, most: usize) -> Result<(), BlobError> {
        while self.parts.len() > most {
            if let Some(done) = self.parts.join_next().await {
                done.map_err(|error| BlobError::Store(format!("a part stopped short: {error}")))??;
            }
        }
        Ok(())
    }

    async fn send(&mut self, part: PutPayload) -> Result<(), BlobError> {
        self.settle(PARTS_AT_ONCE - 1).await?;
        self.parts.spawn(self.upload.put_part(part));
        Ok(())
    }
}

impl ObjectWriter {
    /// Sends the full buffer as a part, starting the multipart upload with the first.
    async fn flush(&mut self) -> Result<(), BlobError> {
        let part: PutPayload = std::mem::take(&mut self.buffer).into();
        let upload = match &mut self.upload {
            Some(upload) => upload,
            None => self.upload.insert(Upload {
                upload: self.store.put_multipart(&self.path).await?,
                parts: tokio::task::JoinSet::new(),
            }),
        };
        upload.send(part).await
    }
}

#[async_trait]
impl BlobWriter for ObjectWriter {
    async fn write(&mut self, mut bytes: Bytes) -> Result<(), BlobError> {
        while !bytes.is_empty() {
            let room = PART_BYTES - self.buffer.content_length();
            if bytes.len() < room {
                self.buffer.push(bytes);
                return Ok(());
            }
            self.buffer.push(bytes.split_to(room));
            self.flush().await?;
        }
        Ok(())
    }

    async fn finish(mut self: Box<Self>) -> Result<(), BlobError> {
        if self.upload.is_none() {
            let whole: PutPayload = std::mem::take(&mut self.buffer).into();
            self.store.put(&self.path, whole).await?;
            return Ok(());
        }
        if self.buffer.content_length() > 0 {
            self.flush().await?;
        }
        let Some(mut upload) = self.upload.take() else {
            return Ok(());
        };
        let done = async {
            upload.settle(0).await?;
            upload.upload.complete().await?;
            Ok::<_, BlobError>(())
        }
        .await;
        if let Err(error) = done {
            upload.parts.shutdown().await;
            // The upload failed already; what aborting says adds nothing to that.
            let _ = upload.upload.abort().await;
            return Err(error);
        }
        Ok(())
    }

    async fn abort(mut self: Box<Self>) -> Result<(), BlobError> {
        match self.upload.take() {
            Some(mut upload) => {
                upload.parts.shutdown().await;
                upload.upload.abort().await?;
                Ok(())
            }
            None => Ok(()),
        }
    }
}
