//! `ChangePassword` (server code 142): the server's confirmation that our
//! password is now the one it repeats back. Until this arrives the change
//! has only been asked for.
//!
//! Also here: messages the protocol documentation lists as deprecated or
//! obsolete that a server or an old client may still send. They carry
//! nothing we act on, and a handler that reads them quietly is better than
//! a warning in the log for every one.

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

/// Server codes read and set aside, all deprecated or obsolete.
///
/// `ParentIP` (73), `ParentInactivityTimeout` (86), `SearchInactivityTimeout`
/// (87), `MinParentsInCache` (88), `DistribPingInterval` (90),
/// `UserPrivileged` (122), `NotifyPrivileges` (124) and `AckNotifyPrivileges`
/// (125).
pub const SET_ASIDE: [u32; 8] = [73, 86, 87, 88, 90, 122, 124, 125];

pub struct SetAsideHandler(pub u32);

impl MessageHandler<ServerMessage> for SetAsideHandler {
    fn get_code(&self) -> u32 {
        self.0
    }

    fn handle(&self, _message: &mut Message, _sender: Sender<ServerMessage>) {}
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
