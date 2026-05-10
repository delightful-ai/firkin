//! reference — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::error::{Error, Result};
#[allow(unused_imports)]
use std::fmt;
#[allow(unused_imports)]
use std::str::FromStr;
/// Parsed OCI image reference with Docker-compatible short-name expansion.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Reference {
    pub(crate) registry: String,
    pub(crate) namespace: String,
    name: String,
    pub(crate) tag: Option<String>,
    pub(crate) digest: Option<String>,
}
impl Reference {
    /// Parse an image reference.
    ///
    /// Short forms follow Docker rules:
    /// `busybox` becomes `docker.io/library/busybox:latest`,
    /// `foo/bar` becomes `docker.io/foo/bar:latest`, and a first path segment
    /// containing `.` or `:` is treated as an explicit registry.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidReference`] when the input is empty, contains
    /// whitespace, has empty path/selector components, or contains multiple
    /// digest separators.
    pub fn parse(input: impl AsRef<str>) -> Result<Self> {
        let input = input.as_ref();
        Self::parse_inner(input)
    }
    fn parse_inner(input: &str) -> Result<Self> {
        if input.is_empty() {
            return invalid(input, "reference is empty");
        }
        if input.chars().any(char::is_whitespace) {
            return invalid(input, "reference contains whitespace");
        }
        let (name_part, digest) = split_once_optional(input, '@', "digest separator")?;
        if name_part.is_empty() {
            return invalid(input, "name is empty");
        }
        let digest = match digest {
            Some("") => return invalid(input, "digest is empty"),
            Some(value) => Some(value.to_owned()),
            None => None,
        };
        let (path, tag) = split_tag(name_part);
        if path.is_empty() {
            return invalid(input, "path is empty");
        }
        if path.split('/').any(str::is_empty) {
            return invalid(input, "path contains an empty segment");
        }
        let tag = match tag {
            Some("") => return invalid(input, "tag is empty"),
            Some(value) => Some(value.to_owned()),
            None if digest.is_none() => Some("latest".to_owned()),
            None => None,
        };
        let parts = path.split('/').collect::<Vec<_>>();
        let (registry, namespace_parts) = match parts.as_slice() {
            [single] => ("docker.io", vec!["library", *single]),
            [first, rest @ ..] if first.contains('.') || first.contains(':') => {
                (*first, rest.to_vec())
            }
            _ => ("docker.io", parts),
        };
        if namespace_parts.is_empty() {
            return invalid(input, "repository path is empty");
        }
        let name = namespace_parts
            .last()
            .expect("namespace_parts checked non-empty")
            .to_string();
        let namespace = namespace_parts.join("/");
        Ok(Self {
            registry: registry.to_owned(),
            namespace,
            name,
            tag,
            digest,
        })
    }
    /// Registry host, for example `docker.io`.
    #[must_use]
    pub fn registry(&self) -> &str {
        &self.registry
    }
    /// Repository path without registry, for example `library/busybox`.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }
    /// Final repository path segment.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Tag selector, if present.
    #[must_use]
    pub fn tag(&self) -> Option<&str> {
        self.tag.as_deref()
    }
    /// Digest selector, if present.
    #[must_use]
    pub fn digest(&self) -> Option<&str> {
        self.digest.as_deref()
    }
    /// Return a copy with a tag selector, replacing any digest selector.
    #[must_use]
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self.digest = None;
        self
    }
    /// Return a copy with a digest selector, replacing any tag selector.
    #[must_use]
    pub fn with_digest(mut self, digest: impl Into<String>) -> Self {
        self.tag = None;
        self.digest = Some(digest.into());
        self
    }
    /// True when the reference names immutable content by digest.
    #[must_use]
    pub const fn pinned(&self) -> bool {
        self.digest.is_some()
    }
    /// Canonical string form.
    #[must_use]
    pub fn canonical(&self) -> String {
        let mut out = format!("{}/{}", self.registry, self.namespace);
        if let Some(tag) = &self.tag {
            out.push(':');
            out.push_str(tag);
        }
        if let Some(digest) = &self.digest {
            out.push('@');
            out.push_str(digest);
        }
        out
    }
}
impl fmt::Display for Reference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.canonical())
    }
}
impl FromStr for Reference {
    type Err = Error;
    fn from_str(input: &str) -> Result<Self> {
        Self::parse(input)
    }
}
fn split_once_optional<'a>(
    input: &'a str,
    needle: char,
    name: &'static str,
) -> Result<(&'a str, Option<&'a str>)> {
    let mut split = input.split(needle);
    let before = split.next().expect("split always yields at least one item");
    let after = split.next();
    if split.next().is_some() {
        return invalid(input, name);
    }
    Ok((before, after))
}
fn split_tag(input: &str) -> (&str, Option<&str>) {
    let slash = input.rfind('/');
    let colon = input.rfind(':');
    match colon {
        Some(colon) if slash.is_none_or(|slash| colon > slash) => {
            (&input[..colon], Some(&input[colon + 1..]))
        }
        _ => (input, None),
    }
}
fn invalid<T>(input: &str, reason: &'static str) -> Result<T> {
    Err(Error::InvalidReference {
        input: input.to_owned(),
        reason,
    })
}
