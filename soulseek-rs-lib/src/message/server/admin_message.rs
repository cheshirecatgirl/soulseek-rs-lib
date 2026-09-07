use crate::actor::server_actor::ServerMessage;
use crate::message::{Message, MessageHandler};
use std::sync::mpsc::Sender;

pub struct AdminMessageHandler;

impl MessageHandler<ServerMessage> for AdminMessageHandler {
    fn get_code(&self) -> u32 {
        66
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let text = message.read_string();
        if text.is_empty() {
            return;
        }
        let _ = sender.send(ServerMessage::AdminMessage(text));
    }
}
