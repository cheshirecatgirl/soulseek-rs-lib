use crate::info;
use crate::{
    message::{Message, MessageHandler},
    peer::PeerMessage,
};
use std::sync::mpsc::Sender;

/// Build an `UploadDenied` (peer code 50): we will not send `filename`, and
/// `reason` is why, in the protocol's own words.
#[must_use]
pub fn build_upload_denied(filename: &str, reason: &str) -> Message {
    Message::new()
        .write_int32(50)
        .write_string(filename)
        .write_string(reason)
        .clone()
}

pub struct UploadDeniedHandler;
impl MessageHandler<PeerMessage> for UploadDeniedHandler {
    fn get_code(&self) -> u32 {
        50
    }
    fn handle(&self, message: &mut Message, sender: Sender<PeerMessage>) {
        let filename = message.read_string();
        let reason = message.read_string();
        info!("Upload denied for {}: {}", filename, reason);
        if reason == "Queued" {
            return;
        }
        // Passed on as the peer wrote it; the host decides how to word it.
        let reason = reason.trim();
        let _ = sender.send(PeerMessage::UploadFailed {
            filename,
            reason: (!reason.is_empty()).then(|| reason.to_string()),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    #[test]
    fn a_queued_reason_is_not_a_failure() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("@@share\\gone.mp3").write_string("Queued");
        });

        UploadDeniedHandler.handle(&mut message, tx);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn the_denied_filename_is_forwarded() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("@@share\\gone.mp3")
                .write_string("File not shared.");
        });

        UploadDeniedHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(PeerMessage::UploadFailed { filename, reason }) => {
                assert_eq!(filename, "@@share\\gone.mp3");
                assert_eq!(reason.as_deref(), Some("File not shared."));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn a_denial_we_build_reads_back_through_the_handler() {
        let built = build_upload_denied("@@share\\a.mp3", "Too many files");
        let mut message = Message::new_with_data(built.get_buffer());
        message.set_pointer(8);
        let (tx, rx) = std::sync::mpsc::channel();
        UploadDeniedHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(PeerMessage::UploadFailed { filename, reason }) => {
                assert_eq!(filename, "@@share\\a.mp3");
                assert_eq!(reason.as_deref(), Some("Too many files"));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn an_empty_reason_is_no_reason() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("@@share\\a.mp3").write_string("  ");
        });
        UploadDeniedHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(PeerMessage::UploadFailed { reason, .. }) => {
                assert_eq!(reason, None);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
