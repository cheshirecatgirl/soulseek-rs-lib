//! `UserInfoRequest` (peer code 15) and its reply (code 16): what a peer says
//! about itself when another user opens its profile.

use crate::message::{Message, MessageHandler};
use crate::peer::PeerMessage;
use std::sync::mpsc::Sender;

pub struct UserInfoRequest;
impl MessageHandler<PeerMessage> for UserInfoRequest {
    fn get_code(&self) -> u32 {
        15
    }
    fn handle(&self, _message: &mut Message, sender: Sender<PeerMessage>) {
        let _ = sender.send(PeerMessage::UserInfoRequested);
    }
}

/// Build a `UserInfoRequest` (peer code 15, no body): ask a peer what it says
/// about itself.
#[must_use]
pub fn build_user_info_request() -> Message {
    Message::new().write_int32(15).clone()
}

/// What a peer says about itself, from its `UserInfoResponse` (code 16).
/// The largest profile picture worth holding in memory.
///
/// Four megabytes is far past any reasonable avatar and far short of what a
/// peer could send if it wanted to cost us something.
pub const MAX_PICTURE: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PeerInfo {
    pub description: String,
    /// Whether a profile picture came with it, and how many bytes it was.
    ///
    /// Kept even when the bytes are not, so a caller can tell "no picture"
    /// from "a picture too big to hold".
    pub picture_bytes: Option<usize>,
    /// The picture itself, when it is small enough to keep.
    ///
    /// A profile picture is the one part of a peer's answer that a person
    /// actually looks at, so it is worth carrying — but a peer chooses how
    /// big it is, and nothing obliges them to be reasonable. Anything past
    /// [`MAX_PICTURE`] is counted and dropped rather than held: the size still
    /// arrives, so a caller can say a picture was refused rather than say
    /// there was none.
    pub picture: Option<Vec<u8>>,
    /// Uploads the peer says it has done in total.
    pub total_uploads: u32,
    /// How many transfers are waiting in its queue.
    pub queue_size: u32,
    /// Whether it has a free upload slot right now.
    pub slots_free: bool,
    /// Whether it would let us queue anything at all. Absent from older
    /// clients, which stop the message one field early.
    pub upload_allowed: Option<u32>,
}

/// `UserInfoResponse` (peer code 16): a peer's answer about itself.
pub struct UserInfoResponseHandler;
impl MessageHandler<PeerMessage> for UserInfoResponseHandler {
    fn get_code(&self) -> u32 {
        16
    }
    fn handle(&self, message: &mut Message, sender: Sender<PeerMessage>) {
        let description = message.read_string();
        let has_picture = message.read_bool();
        let mut picture = None;
        let picture_bytes = has_picture.then(|| {
            let length = message.read_int32() as usize;
            let available =
                message.get_size().saturating_sub(message.get_pointer());
            let length = length.min(available);
            let at = message.get_pointer();
            // `get_slice` takes absolute bounds, not an offset and a count.
            if length <= MAX_PICTURE {
                picture = Some(message.get_slice(at, at + length));
            }
            message.set_pointer(at + length);
            length
        });
        let total_uploads = message.read_int32();
        let queue_size = message.read_int32();
        let slots_free = message.read_bool();
        // Museek+ writes `slotsavail` as an integer, leaving three bytes of
        // rubbish where the next field starts; Nicotine+ guards the read the
        // same way, by only taking a field there is room for.
        let upload_allowed =
            (message.get_size().saturating_sub(message.get_pointer()) >= 4)
                .then(|| message.read_int32());

        let _ = sender.send(PeerMessage::UserInfoReceived(PeerInfo {
            description,
            picture_bytes,
            picture,
            total_uploads,
            queue_size,
            slots_free,
            upload_allowed,
        }));
    }
}

/// Build a `UserInfoResponse` (peer code 16): who we are, and how busy.
///
/// The picture is a length and the bytes, present only when the flag before
/// it says so — the same layout [`UserInfoResponseHandler`] reads.
#[must_use]
pub fn build_user_info(
    description: &str,
    picture: Option<&[u8]>,
    upload_slots: u32,
    queue_size: u32,
    slots_free: bool,
) -> Message {
    let mut message = Message::new();
    message.write_int32(16).write_string(description);
    match picture {
        Some(bytes) => {
            let length = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
            message
                .write_bool(true)
                .write_int32(length)
                .write_raw_bytes(bytes.to_vec());
        }
        None => {
            message.write_bool(false);
        }
    }
    message
        .write_int32(upload_slots)
        .write_int32(queue_size)
        .write_bool(slots_free)
        // Who may send us files unasked: no one, since nothing here accepts
        // a file we did not queue.
        .write_int32(0)
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    fn parse(build: impl FnOnce(&mut Message)) -> PeerInfo {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(build);
        UserInfoResponseHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(PeerMessage::UserInfoReceived(info)) => info,
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn a_peers_answer_carries_its_slots_and_queue() {
        let info = parse(|m| {
            m.write_string("hello, I share flacs");
            m.write_bool(false);
            m.write_int32(42); // total uploads
            m.write_int32(3); // queue size
            m.write_bool(true); // a slot is free
            m.write_int32(1); // uploads allowed
        });

        assert_eq!(info.description, "hello, I share flacs");
        assert_eq!(info.picture_bytes, None);
        assert_eq!(info.total_uploads, 42);
        assert_eq!(info.queue_size, 3);
        assert!(info.slots_free);
        assert_eq!(info.upload_allowed, Some(1));
    }

    #[test]
    fn a_picture_is_kept_and_the_fields_after_it_still_line_up() {
        let info = parse(|m| {
            m.write_string("with a picture");
            m.write_bool(true);
            m.write_int32(4);
            m.write_raw_bytes(vec![1, 2, 3, 4]);
            m.write_int32(7);
            m.write_int32(0);
            m.write_bool(false);
            m.write_int32(0);
        });

        assert_eq!(info.picture_bytes, Some(4));
        assert_eq!(info.picture.as_deref(), Some(&[1u8, 2, 3, 4][..]));
        assert_eq!(info.total_uploads, 7, "the fields after it still line up");
    }

    #[test]
    fn a_picture_past_the_cap_is_counted_and_dropped() {
        // The size still arrives, so a caller can say a picture was refused
        // rather than say there was none.
        let huge = vec![7u8; MAX_PICTURE + 1];
        let info = parse(|m| {
            m.write_string("enormous");
            m.write_bool(true);
            m.write_int32(u32::try_from(huge.len()).unwrap());
            m.write_raw_bytes(huge.clone());
            m.write_int32(3);
            m.write_int32(0);
            m.write_bool(false);
        });

        assert_eq!(info.picture_bytes, Some(MAX_PICTURE + 1));
        assert_eq!(info.picture, None, "too big to hold");
        assert_eq!(info.total_uploads, 3, "and it was still stepped over");
    }

    /// What we send, read back by what reads a peer's: the two have to agree
    /// on where the picture ends, or every field after it is wrong.
    fn ours(description: &str, picture: Option<&[u8]>) -> PeerInfo {
        let built = build_user_info(description, picture, 4, 2, true);
        // The code is ours to write and the handler's caller's to strip.
        let body = built.get_data()[4..].to_vec();
        parse(|m| {
            m.write_raw_bytes(body);
        })
    }

    #[test]
    fn our_own_answer_carries_the_profile_we_set() {
        let info = ours("flac, mostly", Some(&[0x89, b'P', b'N', b'G']));
        assert_eq!(info.description, "flac, mostly");
        assert_eq!(
            info.picture.as_deref(),
            Some(&[0x89, b'P', b'N', b'G'][..])
        );
        assert_eq!(
            info.total_uploads, 4,
            "the fields after the picture still line up"
        );
        assert_eq!(info.queue_size, 2);
        assert!(info.slots_free);
        assert_eq!(info.upload_allowed, Some(0), "nobody may push files to us");
    }

    #[test]
    fn our_own_answer_without_a_picture_says_so() {
        let info = ours("", None);
        assert_eq!(info.description, "");
        assert_eq!(info.picture_bytes, None);
        assert_eq!(info.total_uploads, 4);
    }

    #[test]
    fn a_message_that_stops_early_still_parses() {
        // Older clients omit the trailing field, and Museek+ leaves rubbish
        // where it would be; neither may cost us the fields before it.
        let info = parse(|m| {
            m.write_string("older client");
            m.write_bool(false);
            m.write_int32(5);
            m.write_int32(1);
            m.write_bool(true);
        });

        assert_eq!(info.total_uploads, 5);
        assert_eq!(info.queue_size, 1);
        assert!(info.slots_free);
        assert_eq!(info.upload_allowed, None);
    }

    #[test]
    fn a_picture_longer_than_the_message_does_not_run_off_the_end() {
        let info = parse(|m| {
            m.write_string("hostile");
            m.write_bool(true);
            m.write_int32(u32::MAX);
            m.write_raw_bytes(vec![9, 9]);
        });

        assert_eq!(info.picture_bytes, Some(2));
        assert_eq!(
            info.picture.as_deref(),
            Some(&[9u8, 9][..]),
            "what is really there is kept; the length it claimed is not"
        );
        assert_eq!(info.total_uploads, 0, "nothing left to read reads as zero");
    }
}
