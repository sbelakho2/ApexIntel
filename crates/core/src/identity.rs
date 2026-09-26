//! Canonical identity newtypes.
//!
//! User-owned data is keyed by [`UserId`] — the canonical `app_users.id`
//! carried by signed sessions and API keys. [`Username`] is the login name and
//! display label only; it may change without touching ownership. The two are
//! deliberately distinct types so an identity can never be passed where a
//! display name is expected, or vice versa.

use std::fmt;
use std::ops::Deref;

use serde::{Deserialize, Serialize};

/// Canonical application user id (`app_users.id`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UserId(String);

impl UserId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Deref for UserId {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for UserId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<&str> for UserId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl From<String> for UserId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&String> for UserId {
    fn from(value: &String) -> Self {
        Self(value.clone())
    }
}

impl From<UserId> for String {
    fn from(value: UserId) -> Self {
        value.0
    }
}

impl PartialEq<str> for UserId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for UserId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<String> for UserId {
    fn eq(&self, other: &String) -> bool {
        &self.0 == other
    }
}

/// Login/display name. Never an ownership key.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Username(String);

impl Username {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for Username {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Deref for Username {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for Username {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Username {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl From<String> for Username {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&String> for Username {
    fn from(value: &String) -> Self {
        Self(value.clone())
    }
}

impl From<Username> for String {
    fn from(value: Username) -> Self {
        value.0
    }
}

impl PartialEq<str> for Username {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for Username {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<String> for Username {
    fn eq(&self, other: &String) -> bool {
        &self.0 == other
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn user_id_is_transparent_over_its_string() {
        let id = UserId::from("usr-1");
        assert_eq!(id.as_str(), "usr-1");
        assert_eq!(id, "usr-1");
        assert_eq!(&*id, "usr-1");
        assert_eq!(id.to_string(), "usr-1");
        assert_eq!(UserId::from(id.as_str()), id);
        assert_eq!(String::from(id.clone()), "usr-1");
    }

    #[test]
    fn identity_types_serialize_as_plain_strings() {
        let id = UserId::from("usr-1");
        let name = Username::from("alice");
        assert_eq!(
            serde_json::to_value(&id).unwrap(),
            serde_json::json!("usr-1")
        );
        assert_eq!(
            serde_json::to_value(&name).unwrap(),
            serde_json::json!("alice")
        );

        let back: UserId = serde_json::from_value(serde_json::json!("usr-1")).unwrap();
        assert_eq!(back, id);
        let back: Username = serde_json::from_value(serde_json::json!("alice")).unwrap();
        assert_eq!(back, name);
    }

    #[test]
    fn a_user_id_and_a_username_cannot_be_swapped() {
        fn takes_user_id(_: &UserId) {}
        fn takes_username(_: &Username) {}

        let id = UserId::from("usr-1");
        let name = Username::from("alice");
        takes_user_id(&id);
        takes_username(&name);
    }
}
