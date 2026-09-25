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
        let _ = sender.send(PeerMessage::UploadFailed {
            filename,
            reason: Some(plain(&reason)),
        });
    }
}

/// What a denial reason means, said to the person whose download it was.
///
/// The reasons are fixed strings that clients agree on, some with a trailing
/// full stop and some without, so both spellings are matched. Anything else
/// is a client's own wording and is passed on as it came.
#[must_use]
pub fn plain(reason: &str) -> String {
    let said = match reason.trim().trim_end_matches('.') {
        "Banned" => "The user does not share with you",
        "Cancelled" => "The user cancelled the upload",
        "File not shared" => "The user no longer shares this file",
        "File read error" => "The user could not read this file",
        "Pending shutdown" => "The user is closing their client",
        "Too many files" => "You have queued as many files as this user allows",
        "Too many megabytes" => {
            "You have queued as much data as this user allows"
        }
        "Disallowed extension" => "The user does not share this kind of file",
        "" => "The user declined the download",
        _ => return format!("The user declined: {}", reason.trim()),
    };
    said.to_string()
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
                assert_eq!(
                    reason.as_deref(),
                    Some("The user no longer shares this file")
                );
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
                assert_eq!(reason, Some(plain("Too many files")));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn known_reasons_read_plainly_with_or_without_a_full_stop() {
        assert_eq!(plain("Pending shutdown."), plain("Pending shutdown"));
        assert_eq!(
            plain("Too many megabytes"),
            "You have queued as much data as this user allows"
        );
        assert_eq!(plain("Banned"), "The user does not share with you");
    }

    #[test]
    fn an_unknown_reason_is_passed_on_as_it_came() {
        assert_eq!(plain(" Away "), "The user declined: Away");
        assert_eq!(plain(""), "The user declined the download");
    }
}
