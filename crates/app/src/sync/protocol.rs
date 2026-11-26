//! The y-sync protocol's messages as y-websocket sends them, one or more to a WebSocket
//! message, and what an update would change.

use bytes::Bytes;
use yrs::encoding::read::{self, Cursor, Read as _};
use yrs::sync::Message;
use yrs::sync::protocol::{MSG_AUTH, MSG_QUERY_AWARENESS, MSG_SYNC, PERMISSION_DENIED};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{ReadTxn, Update};

/// The y-sync messages of one WebSocket message, in order. y-websocket sends one at a time,
/// but a message may hold several, one after another. Each must be whole: a message cut
/// short fails, rather than being left out.
pub(crate) fn decode(data: &[u8]) -> Result<Vec<Message>, read::Error> {
    let mut cursor = Cursor::new(data);
    let mut messages = Vec::with_capacity(1);
    loop {
        let start = cursor.next;
        skip_message(&mut cursor)?;
        messages.push(Message::decode_v1(&data[start..cursor.next])?);
        if !cursor.has_content() {
            return Ok(messages);
        }
    }
}

/// Moves past one message: its tag, then what that kind of message holds.
fn skip_message(cursor: &mut Cursor<'_>) -> Result<(), read::Error> {
    match cursor.read_var::<u8>()? {
        MSG_SYNC => {
            cursor.read_var::<u8>()?;
            cursor.read_buf()?;
        }
        MSG_AUTH => {
            if cursor.read_var::<u8>()? == PERMISSION_DENIED {
                cursor.read_buf()?;
            }
        }
        MSG_QUERY_AWARENESS => {}
        // Awareness, and kinds this server does not know, hold one buffer.
        _ => {
            cursor.read_buf()?;
        }
    }
    Ok(())
}

/// One message, encoded on its own: y-websocket reads one message from each WebSocket
/// message it gets.
pub(crate) fn encode(message: &Message) -> Bytes {
    Bytes::from(message.encode_v1())
}

/// The type of the message that tells a connection following a page's comments that a
/// thread changed, beside y-protocols' own: its payload is a JSON text,
/// `{"type":"comments","thread_id":"..."}`, and the client reads the comments again.
pub const MESSAGE_COMMENTS: u8 = 100;

/// The message that a thread of a page's comments changed: made, replied to, edited,
/// resolved, reopened or deleted.
pub fn comments_changed(thread_id: uuid::Uuid) -> Bytes {
    let json = serde_json::json!({"type": "comments", "thread_id": thread_id.to_string()});
    encode(&Message::Custom(
        MESSAGE_COMMENTS,
        json.to_string().into_bytes(),
    ))
}

/// What a message is, for the metrics of message sizes.
pub(crate) fn kind(message: &Message) -> &'static str {
    use yrs::sync::SyncMessage;

    match message {
        Message::Sync(SyncMessage::SyncStep1(_)) => "sync_step1",
        Message::Sync(SyncMessage::SyncStep2(_)) => "sync_step2",
        Message::Sync(SyncMessage::Update(_)) => "update",
        Message::Awareness(_) => "awareness",
        Message::AwarenessQuery => "awareness_query",
        Message::Auth(_) => "auth",
        Message::Custom(..) => "other",
    }
}

/// Whether applying `update` would change the document `txn` reads: it holds something the
/// document lacks, or deletes something the document has not deleted. A client that only
/// views a page still answers the server's sync step 1 with what it holds, which is
/// nothing new; anything more is a change it may not make.
pub(crate) fn changes(update: &Update, txn: &impl ReadTxn) -> bool {
    let known = txn.state_vector();
    for (client, ranges) in update.insertions(true).iter() {
        let have = known.get(client);
        if ranges.iter().any(|range| range.end > have) {
            return true;
        }
    }
    let deletes = update.delete_set();
    if deletes.is_empty() {
        return false;
    }
    let deleted = txn.snapshot().delete_set;
    !deletes.diff(&deleted).is_empty()
}

#[cfg(test)]
mod tests {
    use yrs::sync::{Awareness, SyncMessage};
    use yrs::{Doc, GetString, StateVector, Text, Transact};

    use super::*;

    fn text_doc(client: u64, text: &str) -> Doc {
        let doc = Doc::with_client_id(client);
        let field = doc.get_or_insert_text("t");
        field.push(&mut doc.transact_mut(), text);
        doc
    }

    fn update_of(doc: &Doc, since: &StateVector) -> Update {
        let bytes = doc.transact().encode_state_as_update_v1(since);
        Update::decode_v1(&bytes).expect("an update")
    }

    #[test]
    fn several_messages_decode_in_order_and_garbage_fails() {
        let doc = text_doc(1, "hello");
        let step1 = Message::Sync(SyncMessage::SyncStep1(doc.transact().state_vector()));
        let query = Message::AwarenessQuery;
        let mut both = step1.encode_v1();
        both.extend(query.encode_v1());

        assert_eq!(decode(&both).expect("decodes"), vec![step1.clone(), query]);
        assert_eq!(decode(&encode(&step1)).expect("decodes"), vec![step1]);
        assert!(decode(&[]).is_err());
        assert!(decode(&[0, 9]).is_err());
        let mut truncated = both.clone();
        truncated.truncate(both.len() - 1);
        truncated.push(0);
        assert!(decode(&truncated).is_err());
        let awareness = Message::Awareness(Awareness::new(doc).update().expect("an update"));
        assert_eq!(kind(&awareness), "awareness");
    }

    #[test]
    fn what_a_document_holds_already_changes_nothing() {
        let server = text_doc(1, "hello");
        let viewer = Doc::with_client_id(2);
        viewer
            .transact_mut()
            .apply_update(update_of(&server, &StateVector::default()))
            .expect("applies");
        let field = server.get_or_insert_text("t");
        field.remove_range(&mut server.transact_mut(), 0, 1);
        viewer
            .transact_mut()
            .apply_update(update_of(&server, &StateVector::default()))
            .expect("applies");
        assert_eq!(
            viewer
                .get_or_insert_text("t")
                .get_string(&viewer.transact()),
            "ello"
        );

        // A viewer's answer to sync step 1 holds its whole delete set, all known.
        let answer = update_of(&viewer, &server.transact().state_vector());
        assert!(!changes(&answer, &server.transact()));
        assert!(!changes(&Update::new(), &server.transact()));
        let everything = update_of(&viewer, &StateVector::default());
        assert!(!changes(&everything, &server.transact()));
    }

    #[test]
    fn insertions_and_deletions_the_document_lacks_are_changes() {
        let server = text_doc(1, "hello");
        let editor = Doc::with_client_id(3);
        editor
            .transact_mut()
            .apply_update(update_of(&server, &StateVector::default()))
            .expect("applies");
        let before = editor.transact().state_vector();
        let field = editor.get_or_insert_text("t");
        field.push(&mut editor.transact_mut(), "!");
        assert!(changes(&update_of(&editor, &before), &server.transact()));

        let deleter = Doc::with_client_id(4);
        deleter
            .transact_mut()
            .apply_update(update_of(&server, &StateVector::default()))
            .expect("applies");
        let before = deleter.transact().state_vector();
        let field = deleter.get_or_insert_text("t");
        field.remove_range(&mut deleter.transact_mut(), 0, 2);
        let deletion = update_of(&deleter, &before);
        assert!(deletion.insertions(true).is_empty());
        assert!(changes(&deletion, &server.transact()));
    }
}
