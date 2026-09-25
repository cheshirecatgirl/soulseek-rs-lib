use crate::{
    actor::server_actor::ServerMessage, debug, info, message::Message,
};
use std::sync::mpsc::Sender;

use crate::message::MessageHandler;

pub struct LoginHandler;

impl MessageHandler<ServerMessage> for LoginHandler {
    fn get_code(&self) -> u32 {
        1
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let response = message.read_int8();

        if response != 1 {
            let reason = message.read_string();
            // Only a bad name comes with a detail, saying what is wrong
            // with it.
            let detail = (reason == "INVALIDUSERNAME"
                && message.get_pointer() < message.get_size())
            .then(|| message.read_string());
            let _ = sender.send(if reason.is_empty() {
                ServerMessage::LoginStatus(false)
            } else {
                ServerMessage::LoginRejected { reason, detail }
            });
            return;
        }

        info!("Login successful");
        let greeting = message.read_string();
        debug!("Server greeting: {:?}", greeting);

        let _ = sender.send(ServerMessage::LoginStatus(true));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    #[test]
    fn a_refusal_carries_its_reason() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_int8(0).write_string("INVALIDPASS");
        });
        LoginHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::LoginRejected { reason, detail }) => {
                assert_eq!(reason, "INVALIDPASS");
                assert_eq!(detail, None);
            }
            other => panic!("expected a rejection, got {other:?}"),
        }
    }

    #[test]
    fn a_bad_name_says_what_is_wrong_with_it() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_int8(0)
                .write_string("INVALIDUSERNAME")
                .write_string("Nick too long.");
        });
        LoginHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::LoginRejected { reason, detail }) => {
                assert_eq!(reason, "INVALIDUSERNAME");
                assert_eq!(detail.as_deref(), Some("Nick too long."));
            }
            other => panic!("expected a rejection, got {other:?}"),
        }
    }
}
