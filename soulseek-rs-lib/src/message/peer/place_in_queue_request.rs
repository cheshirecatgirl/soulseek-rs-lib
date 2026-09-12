use crate::{
    message::{Message, MessageHandler},
    peer::PeerMessage,
};
use std::sync::mpsc::Sender;

/// `PlaceInQueueRequest` (peer code 51): a peer asking where the file they
/// queued sits in our upload queue.
///
/// Worth answering rather than dropping: a downloader that gets no position
/// cannot tell a long queue from a client that has forgotten about it.
pub struct PlaceInQueueRequest;

impl MessageHandler<PeerMessage> for PlaceInQueueRequest {
    fn get_code(&self) -> u32 {
        51
    }

    fn handle(&self, message: &mut Message, sender: Sender<PeerMessage>) {
        let filename = message.read_string();
        let _ = sender.send(PeerMessage::IncomingPlaceInQueueRequest(filename));
    }
}

/// Ask a peer where the file we queued sits in their upload queue.
///
/// Peers are not obliged to volunteer a place, and most only send one when
/// asked, so a downloader that never asks shows an empty position forever.
#[must_use]
pub fn build_place_in_queue_request(filename: &str) -> Message {
    Message::new()
        .write_int32(51)
        .write_string(filename)
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    #[test]
    fn the_asked_about_filename_is_forwarded() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("@@share\\track.mp3");
        });

        PlaceInQueueRequest.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(PeerMessage::IncomingPlaceInQueueRequest(filename)) => {
                assert_eq!(filename, "@@share\\track.mp3");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn a_request_names_the_file_under_code_51() {
        let expect: Vec<u8> = [
            51, 0, 0, 0, // code
            17, 0, 0, 0, 64, 64, 115, 104, 97, 114, 101, 92, 116, 114, 97, 99,
            107, 46, 109, 112, 51, // "@@share\\track.mp3"
        ]
        .to_vec();

        assert_eq!(
            expect,
            build_place_in_queue_request("@@share\\track.mp3").get_data()
        );
    }
}
