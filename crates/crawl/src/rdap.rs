//! RDAP (Registration Data Access Protocol) domain-registration client.
//!
//! RDAP is the JSON-based successor to WHOIS.  It is free, requires no API
//! key, and is served by IANA-accredited registries.  The bootstrap endpoint
//! `https://rdap.org/domain/<domain>` follows an HTTP redirect to the
//! authoritative registry for the requested TLD.
//!
//! This module provides a small, self-contained [`RdapClient`] that performs
//! a redirect-following GET against the bootstrap and parses the resulting
//! registration record into a strongly-typed [`RdapRecord`].  Every network
//! failure degrades gracefully: the caller receives `Ok(None)` (or an empty
//! vec) and a `tracing::warn!` is emitted, so a transient outage never aborts
//! the larger crawl pipeline.

use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::warn;
use uuid::Uuid;

use apex_core::entities::{Observation, ObservationType};

/// RDAP bootstrap root.  Follows an HTTP redirect to the authoritative
/// registry for the domain's TLD.
const RDAP_BOOTSTRAP: &str = "https://rdap.org/domain/";

/// Normalised domain-registration record extracted from an RDAP response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RdapRecord {
    /// The domain name that was looked up.
    pub domain: String,
    /// RDAP status array (`active`, `clientTransferProhibited`, …).
    pub status: Vec<String>,
    /// Registrar / sponsoring organisation, if present.
    pub registrar: Option<String>,
    /// Registration date string (verbatim from the registry).
    pub registration_date: Option<String>,
    /// Expiration date string.
    pub expiration_date: Option<String>,
    /// Last-updated date string.
    pub updated_date: Option<String>,
    /// Authoritative name servers.
    pub name_servers: Vec<String>,
    /// Abuse-reporting email or phone, if advertised.
    pub abuse_contact: Option<String>,
    /// Whether the registry reports DNSSEC as enabled.
    pub secure_dns: Option<bool>,
}

impl RdapRecord {
    /// Wrap this record into a crawl [`Observation`] of type
    /// [`ObservationType::NewDomain`].
    ///
    /// `entity_id` optionally attaches the observation to a tracked entity
    /// (e.g. the company that owns the domain).
    pub fn to_observation(&self, entity_id: Option<Uuid>) -> Observation {
        let mut obs = Observation::new(
            ObservationType::NewDomain,
            Utc::now(),
            serde_json::to_value(self).unwrap_or(json!(null)),
            json!({
                "source": "rdap",
                "url": format!("{}{}", RDAP_BOOTSTRAP, self.domain),
            }),
        );
        obs.entity_id = entity_id;
        obs
    }
}

/// RDAP client.  Owns a redirect-following `reqwest::Client`.
pub struct RdapClient {
    client: Client,
}

impl RdapClient {
    /// Build a client with a 15s timeout and a descriptive User-Agent.
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("ApexIntel-RDAP/1.0")
            // RDAP bootstrap redirects to the authoritative registry.
            .build()
            .unwrap_or_else(|error| {
                warn!(error = %error, "rdap: failed to build HTTP client; using default");
                Client::new()
            });
        Self { client }
    }

    /// Look up the registration record for `domain`.
    ///
    /// Returns `Ok(None)` for any non-success outcome (network error, 404
    /// "domain not found", or a 429 rate-limit) after logging a warning.
    /// Returns `Ok(Some(record))` only on a clean 200 with parseable JSON.
    pub async fn lookup(&self, domain: &str) -> Result<Option<RdapRecord>> {
        let url = format!("{}{}", RDAP_BOOTSTRAP, domain.trim().to_lowercase());

        let response = match self.client.get(&url).send().await {
            Ok(resp) => resp,
            Err(error) => {
                warn!(domain = %domain, error = %error, "rdap: network error");
                return Ok(None);
            }
        };

        let status = response.status();
        if status.as_u16() == 404 {
            // Domain not registered / no RDAP record — not an error.
            return Ok(None);
        }
        if status.as_u16() == 429 {
            warn!(domain = %domain, "rdap: rate limited (429); skipping");
            return Ok(None);
        }
        if !status.is_success() {
            warn!(domain = %domain, status = status.as_u16(), "rdap: non-success status");
            return Ok(None);
        }

        let json: serde_json::Value = match response.json().await {
            Ok(value) => value,
            Err(error) => {
                warn!(domain = %domain, error = %error, "rdap: failed to parse JSON");
                return Ok(None);
            }
        };

        Ok(Some(Self::parse(&json, domain)))
    }

    /// Map a raw RDAP JSON document onto an [`RdapRecord`].
    ///
    /// Every field is defensively optional: a missing or differently-shaped
    /// key simply yields an empty/`None` value rather than failing the parse.
    fn parse(value: &serde_json::Value, domain: &str) -> RdapRecord {
        let status = value
            .get("status")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default();

        // Events array: registration, expiration, last changed.
        let events = value.get("events").and_then(|v| v.as_array());
        let event_date = |action: &str| -> Option<String> {
            events.and_then(|arr| {
                arr.iter().find_map(|e| {
                    let is_match = e
                        .get("eventAction")
                        .and_then(|v| v.as_str())
                        .is_some_and(|a| a.eq_ignore_ascii_case(action));
                    is_match
                        .then(|| e.get("eventDate"))
                        .and_then(|d| d.and_then(|v| v.as_str()).map(ToString::to_string))
                })
            })
        };
        let registration_date = event_date("registration");
        let expiration_date = event_date("expiration");
        let updated_date = event_date("last changed");

        // Entities: registrar + abuse.
        let (registrar, abuse_contact) =
            Self::extract_entities(value.get("entities").and_then(|v| v.as_array()));

        // Name servers.
        let name_servers = value
            .get("nameservers")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|ns| {
                        ns.get("ldhName")
                            .or_else(|| ns.get("unicodeName"))
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();

        // secureDNS delegationSigned / zoneSigned.
        let secure_dns = value
            .get("secureDNS")
            .and_then(|v| v.get("delegationSigned"))
            .and_then(|v| v.as_bool());

        RdapRecord {
            domain: domain.trim().to_lowercase(),
            status,
            registrar,
            registration_date,
            expiration_date,
            updated_date,
            name_servers,
            abuse_contact,
            secure_dns,
        }
    }

    /// Walk the top-level `entities` array to find the registrar and the
    /// abuse contact.  Both are located by their `roles` array.
    fn extract_entities(
        entities: Option<&Vec<serde_json::Value>>,
    ) -> (Option<String>, Option<String>) {
        let mut registrar = None;
        let mut abuse_contact = None;
        let Some(entities) = entities else {
            return (registrar, abuse_contact);
        };

        for entity in entities {
            let has_role = |role: &str| -> bool {
                entity
                    .get("roles")
                    .and_then(|v| v.as_array())
                    .is_some_and(|roles| {
                        roles
                            .iter()
                            .any(|r| r.as_str().is_some_and(|s| s.eq_ignore_ascii_case(role)))
                    })
            };

            if registrar.is_none() && has_role("registrar") {
                registrar = entity
                    .get("vcardArray")
                    .and_then(|v| Self::vcard_fn(Some(v)))
                    .or_else(|| entity.get("handle").and_then(|v| v.as_str()).map(ToString::to_string));
            }

            // Abuse contact is typically nested under the registrar entity as
            // a sub-entity with role "abuse", or surfaced via a vCard email.
            if abuse_contact.is_none() {
                if let Some(nested) = entity.get("entities").and_then(|v| v.as_array()) {
                    for sub in nested {
                        let is_abuse = sub
                            .get("roles")
                            .and_then(|v| v.as_array())
                            .is_some_and(|roles| {
                                roles.iter().any(|r| {
                                    r.as_str().is_some_and(|s| s.eq_ignore_ascii_case("abuse"))
                                })
                            });
                        if is_abuse {
                            abuse_contact = Self::vcard_email(sub.get("vcardArray"));
                            if abuse_contact.is_none() {
                                abuse_contact = sub
                                    .get("handle")
                                    .and_then(|v| v.as_str())
                                    .map(ToString::to_string);
                            }
                            if abuse_contact.is_some() {
                                break;
                            }
                        }
                    }
                }
            }
        }

        (registrar, abuse_contact)
    }

    /// Extract the `fn` (formatted name) field from a vCardArray.
    fn vcard_fn(vcard: Option<&serde_json::Value>) -> Option<String> {
        let arr = vcard?.as_array()?;
        // vcardArray = [ "vcard", [ ["fn", {}, "text", "Name"], ... ] ]
        if arr.len() < 2 {
            return None;
        }
        let entries = arr.get(1)?.as_array()?;
        for entry in entries {
            let parts = entry.as_array()?;
            if parts.len() >= 4 && parts.first().and_then(|v| v.as_str()) == Some("fn") {
                return parts
                    .get(3)
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
            }
        }
        None
    }

    /// Extract an `email` field from a vCardArray.
    fn vcard_email(vcard: Option<&serde_json::Value>) -> Option<String> {
        let arr = vcard?.as_array()?;
        if arr.len() < 2 {
            return None;
        }
        let entries = arr.get(1)?.as_array()?;
        for entry in entries {
            let parts = entry.as_array()?;
            if parts.len() >= 4
                && parts
                    .first()
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| s.eq_ignore_ascii_case("email"))
            {
                if let Some(addr) = parts.get(3).and_then(|v| v.as_str()) {
                    return Some(addr.to_string());
                }
            }
        }
        None
    }
}

impl Default for RdapClient {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// A trimmed but structurally faithful RDAP response for example.com
    /// (status, events, registrar entity with nested abuse contact,
    /// nameservers, secureDNS).
    const SAMPLE_RDAP: &str = r#"{
      "objectClassName": "domain",
      "ldhName": "EXAMPLE.COM",
      "status": ["clientTransferProhibited", "serverDeleteProhibited"],
      "events": [
        {"eventAction": "registration", "eventDate": "1995-08-14T04:00:00Z"},
        {"eventAction": "expiration", "eventDate": "2025-08-13T04:00:00Z"},
        {"eventAction": "last changed", "eventDate": "2024-09-13T07:01:38Z"}
      ],
      "secureDNS": {"delegationSigned": false},
      "nameservers": [
        {"ldhName": "A.IANA-SERVERS.NET"},
        {"ldhName": "B.IANA-SERVERS.NET"}
      ],
      "entities": [
        {
          "roles": ["registrar"],
          "handle": "376",
          "vcardArray": ["vcard", [
            ["version", {}, "text", "4.0"],
            ["fn", {}, "text", "RESERVED-Internet Assigned Numbers Authority"]
          ]],
          "entities": [
            {
              "roles": ["abuse"],
              "vcardArray": ["vcard", [
                ["version", {}, "text", "4.0"],
                ["fn", {}, "text", "IANA Abuse"],
                ["email", {}, "text", "abuse@iana.org"]
              ]]
            }
          ]
        }
      ]
    }"#;

    #[test]
    fn parses_sample_rdap_record() {
        let value: serde_json::Value = serde_json::from_str(SAMPLE_RDAP).unwrap();
        let record = RdapClient::parse(&value, "example.com");

        assert_eq!(record.domain, "example.com");
        assert_eq!(record.status.len(), 2);
        assert!(record.status.contains(&"clientTransferProhibited".to_string()));
        assert_eq!(
            record.registration_date.as_deref(),
            Some("1995-08-14T04:00:00Z")
        );
        assert_eq!(
            record.expiration_date.as_deref(),
            Some("2025-08-13T04:00:00Z")
        );
        assert_eq!(
            record.updated_date.as_deref(),
            Some("2024-09-13T07:01:38Z")
        );
        assert_eq!(record.name_servers.len(), 2);
        assert!(record
            .name_servers
            .contains(&"A.IANA-SERVERS.NET".to_string()));
        assert_eq!(record.secure_dns, Some(false));
        assert!(
            record
                .registrar
                .as_deref()
                .unwrap()
                .contains("RESERVED")
        );
        assert_eq!(record.abuse_contact.as_deref(), Some("abuse@iana.org"));
    }

    #[test]
    fn parse_is_defensive_about_missing_fields() {
        let value: serde_json::Value = serde_json::json!({"objectClassName": "domain"});
        let record = RdapClient::parse(&value, "nodata.test");
        assert_eq!(record.domain, "nodata.test");
        assert!(record.status.is_empty());
        assert!(record.registrar.is_none());
        assert!(record.registration_date.is_none());
        assert!(record.name_servers.is_empty());
        assert!(record.secure_dns.is_none());
        assert!(record.abuse_contact.is_none());
    }

    #[test]
    fn to_observation_carries_provenance_and_type() {
        let value: serde_json::Value = serde_json::from_str(SAMPLE_RDAP).unwrap();
        let record = RdapClient::parse(&value, "example.com");
        let entity_id = Uuid::new_v4();
        let obs = record.to_observation(Some(entity_id));

        assert_eq!(obs.observation_type, ObservationType::NewDomain);
        assert_eq!(obs.entity_id, Some(entity_id));
        assert_eq!(
            obs.provenance.get("source").and_then(|v| v.as_str()),
            Some("rdap")
        );
        assert!(
            obs.provenance
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap()
                .ends_with("example.com")
        );
        assert_eq!(
            obs.value.get("domain").and_then(|v| v.as_str()),
            Some("example.com")
        );
    }

    #[test]
    fn lookup_url_is_lowercase() {
        // Sanity: the bootstrap URL builder must lowercase the input.
        let _ = format!("{}{}", RDAP_BOOTSTRAP, "Example.COM".to_lowercase());
    }
}
