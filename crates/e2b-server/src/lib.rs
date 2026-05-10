#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! E2B-compatible control-plane wire types.
//!
//! This crate is the typed boundary between the local backend and E2B SDKs.
//! It intentionally models the wire contract; runtime adapters still own
//! execution, persistence, proxying, snapshots, and network-policy enforcement.
#[allow(unused_imports)]
use bytes::Bytes;
#[allow(unused_imports)]
use http_body_util::BodyExt;
#[allow(unused_imports)]
use http_body_util::Full;
#[allow(unused_imports)]
use hyper::header::CONTENT_TYPE;
#[allow(unused_imports)]
use hyper::header::HeaderValue;
#[allow(unused_imports)]
use hyper::{Response, StatusCode};
#[allow(unused_imports)]
use prost::Message;
#[allow(unused_imports)]
use std::collections::BTreeMap;
#[allow(unused_imports)]
use std::convert::Infallible;
#[allow(unused_imports)]
use std::fmt::Write as _;
#[allow(unused_imports)]
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
#[allow(unused_imports)]
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
pub(crate) type HttpBody = http_body_util::combinators::BoxBody<Bytes, BoxError>;
pub(crate) mod envd_process_selector_proto {
    use prost::Oneof;
    #[derive(Clone, PartialEq, Oneof)]
    pub enum Selector {
        #[prost(uint32, tag = "1")]
        Pid(u32),
        #[prost(string, tag = "2")]
        Tag(String),
    }
}
pub(crate) mod envd_process_input_proto {
    use prost::Oneof;
    #[derive(Clone, PartialEq, Oneof)]
    pub enum Input {
        #[prost(bytes, tag = "1")]
        Stdin(Vec<u8>),
        #[prost(bytes, tag = "2")]
        Pty(Vec<u8>),
    }
}
#[allow(missing_docs)]
#[allow(private_interfaces)]
pub mod envd_filesystem_watch_dir_response_proto {
    use super::{
        EnvdFilesystemEventProto, EnvdFilesystemKeepAliveProto, EnvdFilesystemStartEventProto,
    };
    use prost::Oneof;
    #[derive(Clone, PartialEq, Oneof)]
    pub enum Event {
        #[prost(message, tag = "1")]
        Start(EnvdFilesystemStartEventProto),
        #[prost(message, tag = "2")]
        Filesystem(EnvdFilesystemEventProto),
        #[prost(message, tag = "3")]
        Keepalive(EnvdFilesystemKeepAliveProto),
    }
}
#[derive(Clone, PartialEq, Message)]
struct EnvdFilesystemKeepAliveProto {}
#[allow(missing_docs)]
#[allow(private_interfaces)]
pub mod envd_process_event_proto {
    use super::{EnvdDataEventProto, EnvdEndEventProto, EnvdKeepAliveProto, EnvdStartEventProto};
    use prost::Oneof;
    #[derive(Clone, PartialEq, Oneof)]
    pub enum Event {
        #[prost(message, tag = "1")]
        Start(EnvdStartEventProto),
        #[prost(message, tag = "2")]
        Data(EnvdDataEventProto),
        #[prost(message, tag = "3")]
        End(EnvdEndEventProto),
        #[prost(message, tag = "4")]
        Keepalive(EnvdKeepAliveProto),
    }
}
#[allow(missing_docs)]
#[allow(private_interfaces)]
pub mod envd_data_event_proto {
    use prost::Oneof;
    #[derive(Clone, PartialEq, Oneof)]
    pub enum Output {
        #[prost(bytes, tag = "1")]
        Stdout(Vec<u8>),
        #[prost(bytes, tag = "2")]
        Stderr(Vec<u8>),
        #[prost(bytes, tag = "3")]
        Pty(Vec<u8>),
    }
}
pub mod backend;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use backend::*;
pub mod control_plane;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use control_plane::*;
pub mod domain_proxy;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use domain_proxy::*;
pub mod envd_http;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use envd_http::*;
pub mod lifecycle;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use lifecycle::*;
pub mod registry;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use registry::*;
pub mod routes;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use routes::*;
pub mod state;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use state::*;
#[derive(Clone, PartialEq, Message)]
struct EnvdKeepAliveProto {}
pub(crate) fn auth_error_to_http(message: &str) -> Response<HttpBody> {
    let body = serde_json::to_vec(&BTreeMap::from([("message", message)]))
        .expect("string error body serializes");
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header(CONTENT_TYPE, "application/json")
        .body(full_body(body))
        .expect("static auth error response is valid")
}
pub(crate) fn header_matches(value: Option<&HeaderValue>, expected: &str) -> bool {
    value
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == expected)
}
pub(crate) fn full_body(body: impl Into<Bytes>) -> HttpBody {
    Full::new(body.into())
        .map_err(|error: Infallible| match error {})
        .boxed()
}
