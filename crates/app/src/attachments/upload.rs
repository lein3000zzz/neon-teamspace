//! Writing a file to the blob store as its bytes come: its type from its first bytes, its
//! size against the most it may have, its SHA-256 and an image's width and height on the
//! way. A body that sends nothing for the service's idle limit is given up on, with the part
//! it filled. Nothing of it is kept until its row is added.

use std::time::Duration;

use bytes::{Bytes, BytesMut};
use futures_util::{Stream, StreamExt as _};
use neon_domain::attachments::{
    DIMENSION_BYTES, File, FileKind, SNIFF_BYTES, clean_name, dimensions, sniff,
};
use neon_store::files;
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use super::blobs::{BlobStore, BlobWriter};
use super::{AttachmentError, AttachmentService, UPLOAD_WINDOW};

/// What a file is written for, before its bytes say what it is.
pub(super) struct Template<'a> {
    pub(super) workspace_id: Uuid,
    /// Its page, or nil for a page an import has yet to make.
    pub(super) page_id: Uuid,
    pub(super) uploaded_by: Uuid,
    pub(super) kind: FileKind,
    /// The name its client gave it.
    pub(super) name: &'a str,
    pub(super) max: u64,
}

/// A file written to the blob store whose row is still to be added: until then, its blob
/// is left to the cleanup to delete.
#[derive(Debug, Clone)]
pub(crate) struct Prepared {
    pub(crate) file: File,
}

/// The key a file's blob is kept under: its workspace's folder, and its own id, made for it
/// alone.
fn key(workspace_id: Uuid, id: Uuid) -> String {
    format!("{}/{}", workspace_id.simple(), id.simple())
}

/// The body's next chunk, unless nothing comes for `idle`.
async fn next<S, E>(
    body: &mut S,
    idle: Duration,
) -> Result<Option<Result<Bytes, E>>, AttachmentError>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
{
    tokio::time::timeout(idle, body.next()).await.map_err(|_| {
        AttachmentError::Body(format!(
            "nothing of the file arrived for {} seconds",
            idle.as_secs()
        ))
    })
}

/// Writes `body` to the store as a new file; see the module docs.
pub(super) async fn write<S, E>(
    service: &AttachmentService,
    store: &dyn BlobStore,
    template: Template<'_>,
    mut body: S,
) -> Result<Prepared, AttachmentError>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin + Send,
    E: std::fmt::Display,
{
    let unreadable = |error: E| AttachmentError::Body(error.to_string());
    let idle = service.upload_idle();
    // The first bytes, enough to tell what the file is.
    let mut head = BytesMut::new();
    let mut ended = false;
    while head.len() < SNIFF_BYTES {
        match next(&mut body, idle).await? {
            Some(chunk) => head.extend_from_slice(&chunk.map_err(unreadable)?),
            None => {
                ended = true;
                break;
            }
        }
    }
    let media = sniff(&head, ended);
    if !template.kind.takes(media) {
        return Err(AttachmentError::UnsupportedType {
            kind: template.kind,
            found: media,
        });
    }
    let id = Uuid::now_v7();
    let key = key(template.workspace_id, id);
    let now = service.now();
    files::delete_blob_later(service.pool(), &key, now + UPLOAD_WINDOW).await?;
    let mut writer = store.create(&key).await?;
    let mut counted = Counted {
        max: template.max,
        size: 0,
        hash: Sha256::new(),
        image: media.has_dimensions().then(BytesMut::new),
    };
    let written = async {
        counted.write(writer.as_mut(), head.freeze()).await?;
        if !ended {
            while let Some(chunk) = next(&mut body, idle).await? {
                counted
                    .write(writer.as_mut(), chunk.map_err(unreadable)?)
                    .await?;
            }
        }
        Ok::<_, AttachmentError>(())
    }
    .await;
    if let Err(error) = written {
        match writer.abort().await {
            // Nothing of it is left: neither is there anything for the cleanup to delete.
            Ok(()) => files::blob_deleted(service.pool(), &key).await?,
            Err(aborted) => {
                tracing::warn!(error = %aborted, "an upload's parts are left to the cleanup");
            }
        }
        return Err(error);
    }
    writer.finish().await?;
    let dimensions = counted
        .image
        .as_ref()
        .and_then(|head| dimensions(media, head));
    Ok(Prepared {
        file: File {
            id,
            workspace_id: template.workspace_id,
            page_id: template.page_id,
            uploaded_by: Some(template.uploaded_by),
            kind: template.kind,
            key,
            name: clean_name(template.name, media),
            media,
            size: counted.size,
            sha256: counted.hash.finalize().into(),
            dimensions,
            created_at: now,
        },
    })
}

/// What is counted of a file as it is written.
struct Counted {
    max: u64,
    size: u64,
    hash: Sha256,
    /// An image's first bytes, up to what its width and height are read from.
    image: Option<BytesMut>,
}

impl Counted {
    async fn write(
        &mut self,
        writer: &mut dyn BlobWriter,
        chunk: Bytes,
    ) -> Result<(), AttachmentError> {
        self.size = self
            .size
            .saturating_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX));
        if self.size > self.max {
            return Err(AttachmentError::TooLarge { max: self.max });
        }
        self.hash.update(&chunk);
        if let Some(image) = &mut self.image {
            let room = DIMENSION_BYTES.saturating_sub(image.len());
            image.extend_from_slice(&chunk[..chunk.len().min(room)]);
        }
        writer.write(chunk).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_the_workspace_s_folder_and_the_file_s_id() {
        let workspace = Uuid::from_u128(0x0193_a5b2_0000_7000_8000_0000_0000_00a1);
        let file = Uuid::from_u128(0x0193_a5b2_0000_7000_8000_0000_0000_00f1);
        assert_eq!(
            key(workspace, file),
            "0193a5b20000700080000000000000a1/0193a5b20000700080000000000000f1"
        );
    }
}
