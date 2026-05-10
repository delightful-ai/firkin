use std::fmt;
use std::str::FromStr;

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum InvalidId {
    #[error("{kind} is empty")]
    Empty { kind: &'static str },
    #[error("{kind} `{value}` is too long (max {max} chars)")]
    TooLong {
        kind: &'static str,
        value: String,
        max: usize,
    },
    #[error("{kind} `{value}` contains forbidden characters")]
    ForbiddenChars { kind: &'static str, value: String },
}

macro_rules! id_type {
    ($name:ident, $kind:literal) => {
        #[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, InvalidId> {
                let value = value.into();
                validate_id($kind, &value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = InvalidId;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::new(s)
            }
        }
    };
}

id_type!(RuntimeId, "runtime id");
id_type!(SandboxId, "sandbox id");
id_type!(TemplateId, "template id");
id_type!(TemplateBuildId, "template build id");
id_type!(SnapshotId, "snapshot id");
id_type!(ProcessId, "process id");
id_type!(ProcessTag, "process tag");
id_type!(PortName, "port name");
id_type!(WarmPoolKey, "warm-pool key");
id_type!(BackendName, "backend name");

fn validate_id(kind: &'static str, value: &str) -> Result<(), InvalidId> {
    const MAX_LEN: usize = 128;
    if value.is_empty() {
        return Err(InvalidId::Empty { kind });
    }
    if value.chars().count() > MAX_LEN {
        return Err(InvalidId::TooLong {
            kind,
            value: value.to_owned(),
            max: MAX_LEN,
        });
    }
    if !value.bytes().all(is_id_byte) {
        return Err(InvalidId::ForbiddenChars {
            kind,
            value: value.to_owned(),
        });
    }
    Ok(())
}

fn is_id_byte(byte: u8) -> bool {
    matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'.' | b'-')
}

#[cfg(test)]
mod tests {
    use super::{InvalidId, SandboxId};

    #[test]
    fn id_accepts_path_safe_value() {
        let id = SandboxId::new("sbx.firkin-1").expect("valid id");
        assert_eq!(id.as_str(), "sbx.firkin-1");
    }

    #[test]
    fn id_rejects_path_separator() {
        let error = SandboxId::new("bad/id").expect_err("separator rejected");
        assert!(matches!(error, InvalidId::ForbiddenChars { .. }));
    }
}
