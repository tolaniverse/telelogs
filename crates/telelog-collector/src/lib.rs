//! telelog-collector reads logs next to Docker and Kubernetes and serves them to Telelog Cloud
//! over one outbound connection, so nothing on the machine listens for connections.
//!
//! It enrolls once with a token from the web console ([`enroll`]), then [`run`]s: attached to the
//! gateway, answering the app calls it forwards, and reconnecting when the connection drops.

mod calls;
pub mod enrollment;
mod session;
mod storage;

pub use enrollment::{Enrollment, enroll};
pub use session::{Collector, Timing, refused, run};

use telelog_proto::cloud::v1::{CollectorFrame, collector_frame};

/// This collector's version, sent when enrolling and attaching.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) fn frame(frame: collector_frame::Frame) -> CollectorFrame {
    CollectorFrame { frame: Some(frame) }
}

pub(crate) fn host_name() -> String {
    gethostname::gethostname().to_string_lossy().into_owned()
}
