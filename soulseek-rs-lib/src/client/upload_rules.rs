//! Whether a peer's request to queue one of our files is taken, and if not,
//! the words its client is sent back.
//!
//! The words are the protocol's own. Clients match on them, so they are
//! spelled exactly as the protocol document lists them, full stops included.

use super::ClientContext;
use crate::types::UploadStatus;
use std::path::PathBuf;

/// The file is not shared, or not shared with this person.
pub const FILE_NOT_SHARED: &str = "File not shared.";
/// The person has as many files waiting as we allow one person.
pub const TOO_MANY_FILES: &str = "Too many files";
/// The person has as much data waiting as we allow one person.
pub const TOO_MANY_MEGABYTES: &str = "Too many megabytes";
/// We are closing, so nothing waiting will be served.
pub const PENDING_SHUTDOWN: &str = "Pending shutdown.";

/// How much one person may have waiting in our queue at once. `None` is no
/// limit. Friends are not held to either.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueueLimits {
    pub files: Option<u32>,
    pub megabytes: Option<u64>,
}

/// What becomes of a request to queue a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Take it: the file is here, and this person may have it.
    Queue { real_path: PathBuf, size: u64 },
    /// They already have it waiting or on its way. Clients ask again while
    /// they wait, and asking again must not cost them their place.
    AlreadyWaiting,
    /// Refuse it, saying this.
    Deny(&'static str),
}

impl ClientContext {
    /// Decide a request from `requester` to queue `filename`.
    ///
    /// In this order, as Nicotine+ decides it: a client that is closing
    /// takes nothing; a file that is not shared with this person does not
    /// exist as far as they are concerned; a repeat of something already
    /// waiting changes nothing; and only then are the per-person limits
    /// counted, because a refusal for a file we would never have served
    /// anyway should say so.
    #[must_use]
    pub fn judge_queue_request(
        &self,
        requester: &str,
        filename: &str,
    ) -> Verdict {
        if self.shutting_down {
            return Verdict::Deny(PENDING_SHUTDOWN);
        }
        let friend = self.is_friend(requester);
        let Some(file) = self.shares.get_for(filename, friend) else {
            return Verdict::Deny(FILE_NOT_SHARED);
        };
        if self.has_waiting_or_moving(requester, filename) {
            return Verdict::AlreadyWaiting;
        }
        if !friend {
            let (files, bytes) = self.queued_by(requester);
            if self
                .queue_limits
                .files
                .is_some_and(|most| files + 1 > u64::from(most))
            {
                return Verdict::Deny(TOO_MANY_FILES);
            }
            if self
                .queue_limits
                .megabytes
                .is_some_and(|most| bytes + file.size > most * 1024 * 1024)
            {
                return Verdict::Deny(TOO_MANY_MEGABYTES);
            }
        }
        Verdict::Queue {
            real_path: file.real_path.clone(),
            size: file.size,
        }
    }

    /// Whether `requester` already has `filename` queued, offered, or
    /// streaming.
    fn has_waiting_or_moving(&self, requester: &str, filename: &str) -> bool {
        self.upload_queue.iter().any(|job| {
            job.downloader == requester && job.virtual_path == filename
        }) || self.uploads.values().any(|job| {
            job.downloader == requester && job.virtual_path == filename
        }) || self.active_uploads.values().any(|upload| {
            upload.username == requester
                && upload.filename == filename
                && upload.status == UploadStatus::InProgress
        })
    }

    /// Files and bytes `requester` has waiting in the queue.
    fn queued_by(&self, requester: &str) -> (u64, u64) {
        self.upload_queue
            .iter()
            .filter(|job| job.downloader == requester)
            .fold((0, 0), |(files, bytes), job| (files + 1, bytes + job.size))
    }

    /// Replace the set of people counted as friends: they may have what is
    /// shared with friends only, and are not held to the queue limits.
    pub fn set_friends(&mut self, users: Vec<String>) {
        self.friends = users.into_iter().collect();
    }

    #[must_use]
    pub fn is_friend(&self, username: &str) -> bool {
        self.friends.contains(username)
    }

    pub const fn set_queue_limits(&mut self, limits: QueueLimits) {
        self.queue_limits = limits;
    }

    /// Stop taking requests, and empty the queue, returning each
    /// `(downloader, filename)` that was waiting so they can be told.
    pub fn begin_shutdown(&mut self) -> Vec<(String, String)> {
        self.shutting_down = true;
        self.upload_queue
            .drain(..)
            .map(|job| (job.downloader, job.virtual_path))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shares::Shares;
    use std::sync::Arc;

    /// A context sharing `open.mp3` (100 bytes) with everyone and
    /// `mine.mp3` with friends only.
    fn sharing() -> (ClientContext, String, String) {
        let base = std::env::temp_dir().join(format!(
            "slsk-rules-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let open = base.join("open");
        let mine = base.join("mine");
        std::fs::create_dir_all(&open).unwrap();
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::write(open.join("open.mp3"), [0u8; 100]).unwrap();
        std::fs::write(mine.join("mine.mp3"), [0u8; 100]).unwrap();
        let mut ctx = ClientContext::new();
        ctx.shares =
            Arc::new(Shares::scan_marked(&[(open, false), (mine, true)]));
        (
            ctx,
            "open\\open.mp3".to_string(),
            "mine\\mine.mp3".to_string(),
        )
    }

    #[test]
    fn a_file_we_do_not_share_is_refused_as_not_shared() {
        let (ctx, _, _) = sharing();
        assert_eq!(
            ctx.judge_queue_request("bob", "open\\nothing.mp3"),
            Verdict::Deny(FILE_NOT_SHARED)
        );
    }

    #[test]
    fn a_friends_only_file_is_not_shared_with_anyone_else() {
        let (mut ctx, _, mine) = sharing();
        assert_eq!(
            ctx.judge_queue_request("bob", &mine),
            Verdict::Deny(FILE_NOT_SHARED)
        );
        ctx.set_friends(vec!["bob".to_string()]);
        assert!(matches!(
            ctx.judge_queue_request("bob", &mine),
            Verdict::Queue { size: 100, .. }
        ));
    }

    #[test]
    fn asking_again_keeps_the_place() {
        let (mut ctx, open, _) = sharing();
        ctx.enqueue_upload("bob", &open, PathBuf::new(), 100);
        assert_eq!(
            ctx.judge_queue_request("bob", &open),
            Verdict::AlreadyWaiting
        );
        // Someone else asking for the same file is a new request.
        assert!(matches!(
            ctx.judge_queue_request("ann", &open),
            Verdict::Queue { .. }
        ));
    }

    #[test]
    fn limits_count_what_the_person_has_waiting() {
        let (mut ctx, open, _) = sharing();
        ctx.set_queue_limits(QueueLimits {
            files: Some(1),
            megabytes: None,
        });
        ctx.enqueue_upload("bob", "open\\other.mp3", PathBuf::new(), 100);
        assert_eq!(
            ctx.judge_queue_request("bob", &open),
            Verdict::Deny(TOO_MANY_FILES)
        );
        // Someone else's waiting files are not bob's.
        assert!(matches!(
            ctx.judge_queue_request("ann", &open),
            Verdict::Queue { .. }
        ));
        // And a friend is not held to it.
        ctx.set_friends(vec!["bob".to_string()]);
        assert!(matches!(
            ctx.judge_queue_request("bob", &open),
            Verdict::Queue { .. }
        ));
    }

    #[test]
    fn the_size_limit_counts_the_file_asked_for() {
        let (mut ctx, open, _) = sharing();
        ctx.set_queue_limits(QueueLimits {
            files: None,
            megabytes: Some(1),
        });
        ctx.enqueue_upload(
            "bob",
            "open\\big.flac",
            PathBuf::new(),
            1024 * 1024,
        );
        assert_eq!(
            ctx.judge_queue_request("bob", &open),
            Verdict::Deny(TOO_MANY_MEGABYTES)
        );
    }

    #[test]
    fn closing_refuses_new_requests_and_hands_back_the_queue() {
        let (mut ctx, open, _) = sharing();
        ctx.enqueue_upload("bob", &open, PathBuf::new(), 100);
        let waiting = ctx.begin_shutdown();
        assert_eq!(waiting, vec![("bob".to_string(), open.clone())]);
        assert_eq!(
            ctx.judge_queue_request("ann", &open),
            Verdict::Deny(PENDING_SHUTDOWN)
        );
    }
}
