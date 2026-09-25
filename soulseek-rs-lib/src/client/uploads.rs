use super::{
    ActiveUpload, Arc, Client, ClientContext, DownloadStatus, PeerMessage,
    RwLock, RwLockExt, ServerMessage, collect_failed_tokens, error,
    next_upload_token, thread,
};
use crate::message::server::MessageFactory;
use crate::types::UploadStatus;
use std::sync::atomic::{AtomicBool, AtomicU64};

impl Client {
    /// Whether the server listed `username` as privileged. Privileged peers are
    /// served before everyone else.
    #[must_use]
    pub fn is_privileged(&self, username: &str) -> bool {
        self.context
            .read_safe()
            .is_ok_and(|ctx| ctx.is_privileged(username))
    }

    /// Ask the server how much of our own privilege time is left (code 92). The
    /// answer arrives asynchronously; read it with
    /// [`Client::own_privilege_seconds`].
    ///
    /// # Errors
    /// [`crate::error::SoulseekRs::NotConnected`] without a server connection.
    pub fn check_privileges(&self) -> crate::error::Result<()> {
        let Some(handle) = &self.server_handle else {
            return Err(crate::error::SoulseekRs::NotConnected);
        };
        let _ = handle.send(super::ServerMessage::CheckPrivileges);
        Ok(())
    }

    /// Seconds of our own privileges left, or `None` until the server has
    /// answered a [`Client::check_privileges`].
    #[must_use]
    pub fn own_privilege_seconds(&self) -> Option<u32> {
        self.context
            .read_safe()
            .ok()
            .and_then(|ctx| ctx.own_privileges)
    }

    /// Queued-upload states recorded since the last call, for a caller that
    /// samples [`Client::uploads`] and would otherwise miss a peer that queued
    /// and was served between two samples.
    #[must_use]
    pub fn take_upload_events(&self) -> Vec<crate::types::UploadInfo> {
        self.context
            .write_safe()
            .map(|mut ctx| ctx.take_upload_events())
            .unwrap_or_default()
    }

    /// How many uploads run at once. Lowering it does not interrupt transfers
    /// already in flight; the queue simply refills more slowly.
    /// Set what a peer is told when it asks who we are.
    ///
    /// Every other client shows this as a person's profile: the text they
    /// wrote and the picture they chose. Answering with nothing, which is
    /// all this library could do, made everyone using it a blank page.
    ///
    /// # Errors
    ///
    /// A picture larger than [`MAX_PICTURE`](crate::message::peer::MAX_PICTURE)
    /// is refused rather than sent: it is the size past which this library
    /// drops a peer's own picture unread, and a client reading ours is owed
    /// the same courtesy.
    pub fn set_profile(
        &self,
        text: &str,
        picture: Option<Vec<u8>>,
    ) -> crate::error::Result<()> {
        if picture.as_ref().is_some_and(|bytes| {
            bytes.len() > crate::message::peer::MAX_PICTURE
        }) {
            return Err(crate::error::SoulseekRs::InvalidMessage(format!(
                "a profile picture can be at most {} bytes",
                crate::message::peer::MAX_PICTURE
            )));
        }
        if let Ok(mut ctx) = self.context.write_safe() {
            text.clone_into(&mut ctx.profile_text);
            ctx.profile_picture = picture;
        }
        Ok(())
    }

    pub fn set_upload_slots(&self, slots: usize) {
        if let Ok(mut ctx) = self.context.write_safe() {
            ctx.upload_slots = slots.max(1);
        }
    }

    /// Replace the set of people whose upload requests are not served.
    ///
    /// Not refused — ignored: a refusal is still a conversation. Anything of
    /// theirs already in the queue goes with it, or ignoring somebody would
    /// only apply to requests they had not made yet.
    pub fn set_ignored(&self, users: Vec<String>) {
        if let Ok(mut ctx) = self.context.write_safe() {
            ctx.set_ignored(users);
        }
    }

    /// Replace the set of people counted as friends. They are sent what is
    /// shared with friends only, and are not held to the queue limits.
    pub fn set_friends(&self, users: Vec<String>) {
        if let Ok(mut ctx) = self.context.write_safe() {
            ctx.set_friends(users);
        }
    }

    /// How much one person may have waiting in our queue. A request past
    /// either limit is refused with the protocol's "Too many files" or "Too
    /// many megabytes", which their client shows them.
    pub fn set_queue_limits(
        &self,
        limits: crate::client::upload_rules::QueueLimits,
    ) {
        if let Ok(mut ctx) = self.context.write_safe() {
            ctx.set_queue_limits(limits);
        }
    }

    /// Say we are closing: everyone waiting in the queue is told "Pending
    /// shutdown." and every later request is refused the same way. Call it
    /// before [`Client::disconnect`], while the peers can still be reached,
    /// so their clients stop waiting on a queue that is gone.
    pub fn announce_shutdown(&self) {
        let waiting = match self.context.write_safe() {
            Ok(mut ctx) => ctx.begin_shutdown(),
            Err(_) => return,
        };
        for (downloader, filename) in waiting {
            Self::deny_upload(
                &self.context,
                &downloader,
                &filename,
                crate::client::upload_rules::PENDING_SHUTDOWN,
            );
        }
    }

    /// Drop everything `username` was queued or offered, then hand their slot to
    /// whoever is next.
    pub(crate) fn release_upload_slots(
        client_context: &Arc<RwLock<ClientContext>>,
        username: &str,
    ) {
        let freed = client_context
            .write_safe()
            .is_ok_and(|mut ctx| ctx.release_upload_slots(username));
        if freed {
            Self::pump_upload_queue(client_context);
        }
    }

    /// Fill every free upload slot from the queue and send the offers.
    ///
    /// Called whenever the queue or the slot count could have changed: a new
    /// request, a finished transfer, or a privileged list that re-ranked who is
    /// waiting.
    pub(crate) fn pump_upload_queue(
        client_context: &Arc<RwLock<ClientContext>>,
    ) {
        let (registry, offers) = match client_context.write_safe() {
            Ok(mut ctx) => ctx.pump_uploads(next_upload_token),
            Err(e) => {
                error!("[client] pump_upload_queue write: {}", e);
                return;
            }
        };
        let Some(registry) = registry else {
            return;
        };
        for offer in offers {
            let _ = registry.send_to_peer(
                &offer.requester_key,
                PeerMessage::ServeUpload {
                    token: offer.token,
                    filename: offer.virtual_path,
                    size: offer.size,
                },
            );
        }
    }

    /// Consume the upload job for `token` and stream the file to `host:port`
    /// on a background thread.
    pub(crate) fn spawn_serve(
        client_context: &Arc<RwLock<ClientContext>>,
        own_username: &str,
        token: u32,
        host: String,
        port: u32,
    ) {
        let Ok(mut ctx) = client_context.write_safe() else {
            return;
        };
        let Some(job) = ctx.uploads.remove(&token) else {
            return;
        };
        let bytes_sent = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        ctx.active_uploads.insert(
            token,
            ActiveUpload {
                username: job.downloader.clone(),
                filename: job.virtual_path.clone(),
                size: job.size,
                bytes_sent: bytes_sent.clone(),
                cancel: cancel.clone(),
                status: UploadStatus::InProgress,
                started: std::time::Instant::now(),
            },
        );
        drop(ctx);
        let own = own_username.to_string();
        let real_path = job.real_path;
        let context = client_context.clone();
        thread::spawn(move || {
            let result = crate::peer::upload_peer::serve_file(
                &host,
                port,
                &own,
                token,
                &real_path,
                &bytes_sent,
                &cancel,
            );
            let streamed = result.as_ref().ok().copied();
            let status = match &result {
                Ok(_) => UploadStatus::Completed,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                    UploadStatus::Cancelled
                }
                Err(e) => {
                    error!("[client] serve {}: {}", real_path.display(), e);
                    UploadStatus::Failed(e.to_string())
                }
            };
            let mut report = None;
            if let Ok(mut ctx) = context.write_safe()
                && let Some(upload) = ctx.active_uploads.get_mut(&token)
            {
                let secs = upload.started.elapsed().as_secs_f64();
                upload.status = status;
                if let Some(bytes) = streamed
                    && secs > 0.0
                {
                    let speed = (bytes as f64 / secs) as u32;
                    ctx.last_upload_speed = speed;
                    report = ctx.server_sender.clone().map(|s| (s, speed));
                }
            }
            // The server folds each finished upload's rate into the average
            // speed it shows other users for us.
            if let Some((server, speed)) = report {
                let _ = server.send(ServerMessage::SendMessage(
                    MessageFactory::build_send_upload_speed(speed),
                ));
            }
            // The slot this transfer held is free now, so whoever is next in
            // line gets it without waiting for another request to arrive.
            Self::pump_upload_queue(&context);
        });
    }

    pub(crate) fn process_failed_uploads(
        client_context: Arc<RwLock<ClientContext>>,
        username: &str,
        filename: Option<&str>,
        reason: Option<&str>,
    ) {
        let failed_tokens = match client_context.read_safe() {
            Ok(context) => {
                collect_failed_tokens(&context.downloads, username, filename)
            }
            Err(e) => {
                error!("[client] process_failed_uploads read: {}", e);
                return;
            }
        };

        if failed_tokens.is_empty() {
            return;
        }

        match client_context.write_safe() {
            Ok(mut context) => {
                for token in failed_tokens {
                    context.downloads.update_status(
                        token,
                        DownloadStatus::Failed(Some(
                            reason
                                .unwrap_or(
                                    "The upload failed on the other side",
                                )
                                .to_string(),
                        )),
                    );
                    context.downloads.remove(token);
                }
            }
            Err(e) => {
                error!("[client] process_failed_uploads write: {}", e);
            }
        }
    }
}
