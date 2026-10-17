//! `ChangePassword` (server code 142): the server repeats the new password
//! back once it has taken it. Until then the change was only asked for.

use std::sync::mpsc::Sender;

use crate::actor::server_actor::ServerMessage;
use crate::message::{Message, MessageHandler};

pub struct ChangePasswordHandler;

impl MessageHandler<ServerMessage> for ChangePasswordHandler {
    fn get_code(&self) -> u32 {
        142
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        if message.get_pointer() + 4 > message.get_size() {
            return;
        }
        let _ =
            sender.send(ServerMessage::PasswordChanged(message.read_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    #[test]
    fn the_confirmation_carries_the_new_password() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("hunter3");
        });
        ChangePasswordHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::PasswordChanged(password)) => {
                assert_eq!(password, "hunter3");
            }
            other => panic!("expected the confirmation, got {other:?}"),
        }
    }

    #[test]
    fn a_truncated_confirmation_is_not_a_confirmation() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|_| {});
        ChangePasswordHandler.handle(&mut message, tx);
        assert!(rx.try_recv().is_err());
    }
}
