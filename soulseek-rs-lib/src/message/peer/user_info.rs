use crate::message::{Message, MessageHandler};
use crate::peer::PeerMessage;
use std::sync::mpsc::Sender;

/// What a peer says about itself: peer code 16, answering code 15.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PeerInfo {
    pub description: String,
    pub picture: Option<Vec<u8>>,
    pub upload_slots: u32,
    pub queue_length: u32,
    pub free_slots: bool,
}

pub struct UserInfoRequest;
impl MessageHandler<PeerMessage> for UserInfoRequest {
    fn get_code(&self) -> u32 {
        15
    }
    fn handle(&self, _message: &mut Message, sender: Sender<PeerMessage>) {
        let _ = sender.send(PeerMessage::UserInfoRequested);
    }
}

pub struct UserInfoReply;
impl MessageHandler<PeerMessage> for UserInfoReply {
    fn get_code(&self) -> u32 {
        16
    }
    fn handle(&self, message: &mut Message, sender: Sender<PeerMessage>) {
        let _ = sender
            .send(PeerMessage::UserInfoReceived(parse_user_info(message)));
    }
}

#[must_use]
pub fn build_user_info_request() -> Message {
    Message::new().write_int32(15).clone()
}

#[must_use]
pub fn build_user_info(info: &PeerInfo) -> Message {
    let mut message = Message::new();
    message.write_int32(16).write_string(&info.description);
    match &info.picture {
        Some(bytes) => {
            message
                .write_bool(true)
                .write_int32(bytes.len() as u32)
                .write_raw_bytes(bytes.clone());
        }
        None => {
            message.write_bool(false);
        }
    }
    message
        .write_int32(info.upload_slots)
        .write_int32(info.queue_length)
        .write_bool(info.free_slots)
        .clone()
}

/// Parse a `UserInfoReply` body. A truncated message yields whatever was
/// readable, since peers disagree about the trailing fields.
#[must_use]
pub fn parse_user_info(message: &mut Message) -> PeerInfo {
    let description = message.read_string();
    let mut picture = None;
    if message.read_bool() {
        let length = message.read_int32() as usize;
        let at = message.get_pointer();
        if length > 0 && at + length <= message.get_size() {
            picture = Some(message.get_slice(at, at + length));
            message.set_pointer(at + length);
        }
    }
    PeerInfo {
        description,
        picture,
        upload_slots: message.read_int32(),
        queue_length: message.read_int32(),
        free_slots: message.read_bool(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_info_round_trips_with_and_without_a_picture() {
        for picture in [None, Some(vec![1u8, 2, 3, 4])] {
            let info = PeerInfo {
                description: "nothing much".to_string(),
                picture,
                upload_slots: 3,
                queue_length: 11,
                free_slots: true,
            };
            let built = build_user_info(&info);
            let mut message = Message::new_with_data(built.get_data());
            message.set_pointer(4);
            assert_eq!(parse_user_info(&mut message), info);
        }
    }

    #[test]
    fn a_truncated_reply_does_not_panic() {
        let mut message = Message::new();
        message.write_string("only a description");
        let mut message = Message::new_with_data(message.get_data());
        message.set_pointer(0);
        let info = parse_user_info(&mut message);
        assert_eq!(info.description, "only a description");
        assert_eq!(info.upload_slots, 0);
    }
}
