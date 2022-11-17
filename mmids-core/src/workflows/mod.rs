//! A workflow represents a single media pipeline.  Each workflow contains one or more steps that
//! can either receive video, transform video, or send video to other sources.  Media data
//! transitions from one step to the next in a linear fashion based on the order in which they
//! were defined.

pub mod definitions;
pub mod manager;
pub mod metadata;
mod runner;
pub mod steps;

pub use runner::{start_workflow, WorkflowRequest, WorkflowRequestOperation, WorkflowStatus};

use crate::codecs::{AudioCodec, VideoCodec};
use crate::{StreamId, VideoTimestamp};
use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::workflows::metadata::MediaPayloadMetadata;
pub use runner::{WorkflowState, WorkflowStepState};

/// Notification about media coming across a specific stream
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaNotification {
    /// The identifier for the stream that this notification pertains to
    pub stream_id: StreamId,

    /// The content of the notification message
    pub content: MediaNotificationContent,
}

/// The detailed information contained within a media notification
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaNotificationContent {
    /// Announces that this stream has now connected, and steps that receive this notification
    /// should prepare for media data to start coming through
    NewIncomingStream {
        /// The name for the stream that's being published
        stream_name: String,
    },

    /// Announces that this stream's source has disconnected and will no longer be sending any
    /// new notifications down.  Steps that receive this message can use this to clean up any
    /// information they are tracking about this stream, as no new media will arrive without
    /// a new `NewIncomingStream` announcement.
    StreamDisconnected,

    /// Video content
    Video {
        codec: VideoCodec,
        is_sequence_header: bool,
        is_keyframe: bool,
        data: Bytes,
        timestamp: VideoTimestamp,
    },

    /// Audio content
    Audio {
        codec: AudioCodec,
        is_sequence_header: bool,
        data: Bytes,
        timestamp: Duration,
    },

    /// New stream metadata
    Metadata { data: HashMap<String, String> },

    /// An individual payload as part of this media stream
    MediaPayload {
        /// High level description of the type of payload contained.
        codec: Arc<String>,

        /// How long since an unidentified epoch is this payload valid for. It cannot be assumed
        /// that this is necessarily the duration from stream begin, but can be used to determine
        /// when this payload should be decoded in comparison to payloads that came in before and
        /// after it.
        timestamp: Duration,

        /// Metadata that's only specific to this individual payload
        metadata: MediaPayloadMetadata,

        /// Actual payload bytes
        data: Bytes,

        /// Determines if this payload is a high priority packet that is required for decoding.
        /// This is meant for sequence headers (for h264 and aac as an example) where later packets
        /// cannot be decoded without it. These high priority packets are rarely re-sent, and
        /// therefore this flag lets us know to cache them when this is `true`.
        ///
        /// Flagging this as `true` will cause these packets to be  cached, potentially until a
        /// `StreamDisconnected` signal occurs, and therefore this must only be set for rare
        /// high priority packets (i.e. not for key frames in video).
        is_required_for_decoding: bool,
    },
}
