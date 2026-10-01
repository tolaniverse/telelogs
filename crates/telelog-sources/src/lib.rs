//! Log sources. Shared by the server and, for local mode, the desktop app.

pub mod docker;

pub use docker::{DockerSource, LiveStart};
