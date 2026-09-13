use crate::trace;
use std::sync::mpsc::Sender;

use crate::{
    actor::server_actor::ServerMessage, message::Message,
    message::handlers::MessageHandler,
};

/// The distributed message code for a search.
const SEARCH: u8 = 3;

/// `EmbeddedMessage` (server code 93): a message from the distributed network,
/// wrapped by the server and handed to us directly.
///
/// This is how a client that has told the server it has no parent still hears
/// what the network is looking for. Searches reach peers over the distributed
/// tree, and a client that never receives one answers nothing — which means
/// its shares cannot be found, however many it has. The server relays them
/// here instead of requiring us to be somebody's child.
///
/// The payload is a distributed message code and then that message's own body.
/// Only the search is acted on; the rest of the distributed protocol — branch
/// levels, roots, pings — is about a tree we are not part of.
pub struct EmbeddedMessageHandler;

impl MessageHandler<ServerMessage> for EmbeddedMessageHandler {
    fn get_code(&self) -> u32 {
        93
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let code = message.read_int8();
        if code != SEARCH {
            trace!("[server] embedded message {} ignored", code);
            return;
        }
        // The first field of a distributed search is unused by every client
        // that reads it, and is read only to get past it.
        let _unused = message.read_int32();
        let username = message.read_string();
        let token = message.read_int32();
        let query = message.read_string();
        if username.is_empty() || query.is_empty() {
            return;
        }
        trace!(
            "[server] distributed search from {}: {} ({})",
            username, query, token
        );
        let _ = sender.send(ServerMessage::FileSearchRequest {
            username,
            token,
            query,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    #[test]
    fn a_wrapped_search_arrives_as_a_search() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_int8(SEARCH)
                .write_int32(0)
                .write_string("velvet_hare")
                .write_int32(4242)
                .write_string("aphex twin");
        });

        EmbeddedMessageHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::FileSearchRequest {
                username,
                token,
                query,
            }) => {
                assert_eq!(username, "velvet_hare");
                assert_eq!(token, 4242);
                assert_eq!(query, "aphex twin");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn anything_else_in_the_wrapper_is_left_alone() {
        // Branch levels, roots and pings are about a tree this client is not
        // part of. Reading them as searches would mean answering nonsense.
        for code in [0u8, 4, 5] {
            let (tx, rx) = std::sync::mpsc::channel();
            let mut message = framed(|m| {
                m.write_int8(code).write_int32(7);
            });
            EmbeddedMessageHandler.handle(&mut message, tx);
            assert!(rx.try_recv().is_err(), "code {code} should be ignored");
        }
    }

    #[test]
    fn a_search_with_nothing_in_it_is_not_forwarded() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_int8(SEARCH)
                .write_int32(0)
                .write_string("someone")
                .write_int32(1)
                .write_string("");
        });

        EmbeddedMessageHandler.handle(&mut message, tx);
        assert!(rx.try_recv().is_err(), "an empty query matches everything");
    }
}
