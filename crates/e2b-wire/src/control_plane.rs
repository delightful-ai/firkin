//! control plane — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use serde::Serialize;
#[allow(unused_imports)]
use serde::de::DeserializeOwned;
#[allow(unused_imports)]
use std::collections::BTreeMap;
#[allow(unused_imports)]
use std::str::FromStr;
/// HTTP-like method for the transport-neutral E2B control-plane dispatcher.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub enum ControlPlaneMethod {
    /// `GET`.
    Get,
    /// `POST`.
    Post,
    /// `PATCH`.
    Patch,
    /// `PUT`.
    Put,
    /// `DELETE`.
    Delete,
}
impl FromStr for ControlPlaneMethod {
    type Err = MethodNotAllowed;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "GET" => Self::Get,
            "POST" => Self::Post,
            "PATCH" => Self::Patch,
            "PUT" => Self::Put,
            "DELETE" => Self::Delete,
            _ => return Err(MethodNotAllowed),
        })
    }
}
impl ControlPlaneMethod {
    pub const fn mutates_state(self) -> bool {
        !matches!(self, Self::Get)
    }
}
/// Returned when a string does not match one of the supported control-plane methods.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub struct MethodNotAllowed;
impl std::fmt::Display for MethodNotAllowed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("method not allowed")
    }
}
impl std::error::Error for MethodNotAllowed {}
/// Transport-neutral E2B control-plane request.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub struct ControlPlaneRequest {
    /// HTTP-like method.
    pub method: ControlPlaneMethod,
    /// Absolute path, optionally including a query string.
    pub path: String,
    /// Raw JSON request body.
    pub body: Option<Vec<u8>>,
    /// Origin base URL for responses that need callback URLs.
    pub origin: Option<String>,
}
impl ControlPlaneRequest {
    /// Construct a request without a body.
    #[must_use]
    pub fn new(method: ControlPlaneMethod, path: impl Into<String>) -> Self {
        Self {
            method,
            path: path.into(),
            body: None,
            origin: None,
        }
    }
    /// Attach a JSON-encoded body.
    ///
    /// # Errors
    ///
    /// Returns JSON serialization errors for invalid request bodies.
    pub fn with_json<T>(mut self, body: &T) -> Result<Self, serde_json::Error>
    where
        T: Serialize,
    {
        self.body = Some(serde_json::to_vec(body)?);
        Ok(self)
    }
    /// Attach an origin base URL.
    #[must_use]
    pub fn with_origin(mut self, origin: impl Into<String>) -> Self {
        self.origin = Some(origin.into());
        self
    }
}
/// Transport-neutral E2B control-plane response.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub struct ControlPlaneResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response headers.
    pub headers: BTreeMap<String, String>,
    /// Raw JSON response body.
    pub body: Vec<u8>,
}
impl ControlPlaneResponse {
    /// Construct an empty response.
    #[must_use]
    pub fn empty(status: u16) -> Self {
        Self {
            status,
            headers: BTreeMap::new(),
            body: Vec::new(),
        }
    }
    /// Construct a JSON response.
    ///
    /// # Errors
    ///
    /// Returns JSON serialization errors for invalid response bodies.
    pub fn json<T>(status: u16, body: &T) -> Result<Self, serde_json::Error>
    where
        T: Serialize,
    {
        Ok(Self {
            status,
            headers: BTreeMap::from([("content-type".to_owned(), "application/json".to_owned())]),
            body: serde_json::to_vec(body)?,
        })
    }
    /// Decode the response body as JSON.
    ///
    /// # Errors
    ///
    /// Returns JSON deserialization errors when the body does not match `T`.
    pub fn decode_json<T>(&self) -> Result<T, serde_json::Error>
    where
        T: DeserializeOwned,
    {
        serde_json::from_slice(&self.body)
    }
}
