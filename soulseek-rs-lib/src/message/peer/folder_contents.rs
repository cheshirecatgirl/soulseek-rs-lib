use crate::message::peer::SharedDirectory;
use crate::message::peer::shared_file_list::{
    decompress_body, read_directories, write_directories,
};
use crate::message::{Message, MessageHandler};
use crate::peer::PeerMessage;
use crate::utils::zlib::compress;
use std::sync::mpsc::Sender;

pub struct FolderContentsRequest;
impl MessageHandler<PeerMessage> for FolderContentsRequest {
    fn get_code(&self) -> u32 {
        36
    }
    fn handle(&self, message: &mut Message, sender: Sender<PeerMessage>) {
        let token = message.read_int32();
        let folder = message.read_string();
        if folder.is_empty() {
            return;
        }
        let _ =
            sender.send(PeerMessage::FolderContentsRequested { token, folder });
    }
}

pub struct FolderContentsResponse;
impl MessageHandler<PeerMessage> for FolderContentsResponse {
    fn get_code(&self) -> u32 {
        37
    }
    fn handle(&self, message: &mut Message, sender: Sender<PeerMessage>) {
        let Some((token, folder, directories)) = parse_folder_contents(message)
        else {
            return;
        };
        let _ = sender.send(PeerMessage::FolderContentsReceived {
            token,
            folder,
            directories,
        });
    }
}

#[must_use]
pub fn build_folder_contents_request(token: u32, folder: &str) -> Message {
    Message::new()
        .write_int32(36)
        .write_int32(token)
        .write_string(folder)
        .clone()
}

#[must_use]
pub fn build_folder_contents(
    token: u32,
    folder: &str,
    dirs: &[SharedDirectory],
) -> Message {
    let mut payload = Message::new();
    payload.write_int32(token).write_string(folder);
    write_directories(&mut payload, dirs);
    let compressed = compress(&payload.get_data());
    Message::new()
        .write_int32(37)
        .write_raw_bytes(compressed)
        .clone()
}

/// Parse a `FolderContentsResponse` payload, or `None` when it is malformed.
#[must_use]
pub fn parse_folder_contents(
    message: &mut Message,
) -> Option<(u32, String, Vec<SharedDirectory>)> {
    let mut body = decompress_body(message)?;
    let token = body.read_int32();
    let folder = body.read_string();
    let dirs = read_directories(&mut body);
    Some((token, folder, dirs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::peer::SharedFileEntry;

    #[test]
    fn a_folder_listing_round_trips() {
        let dirs = vec![SharedDirectory {
            name: "music\\album".to_string(),
            files: vec![
                SharedFileEntry {
                    name: "one.flac".to_string(),
                    size: 40,
                    attributes: vec![(0, 992), (1, 214)],
                },
                SharedFileEntry::bare("two.flac".to_string(), 50),
            ],
        }];
        let built = build_folder_contents(7, "music\\album", &dirs);
        let mut message = Message::new_with_data(built.get_data());
        message.set_pointer(4);
        let (token, folder, back) =
            parse_folder_contents(&mut message).unwrap();
        assert_eq!(token, 7);
        assert_eq!(folder, "music\\album");
        assert_eq!(back, dirs);
    }

    #[test]
    fn a_hostile_count_does_not_hang() {
        let mut payload = Message::new();
        payload
            .write_int32(1)
            .write_string("x")
            .write_int32(u32::MAX);
        let framed = compress(&payload.get_data());
        let mut message = Message::new_with_data(framed);
        message.set_pointer(0);
        let (_, _, dirs) = parse_folder_contents(&mut message).unwrap();
        assert!(dirs.is_empty());
    }
}
