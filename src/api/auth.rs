//! Operator tokens (M4): a stand-in for real authentication until the
//! console has one. `OPERATOR_TOKENS` lists `actor_id:ROLE[+ROLE]:sha256hex`
//! entries separated by commas; the hex is the SHA-256 of the bearer token,
//! so plain tokens are never stored. A request authenticates with
//! `Authorization: Bearer <token>`.
//!
//! With no valid entry configured nothing authenticates (fail closed).
//! `SYSTEM_SERVICE` cannot be given to an operator.

use axum::http::{header, HeaderMap};
use sha2::{Digest, Sha256};

use crate::models::{ActorContext, ActorRole};

#[derive(Clone, Default)]
pub struct OperatorTokens {
    entries: Vec<([u8; 32], ActorContext)>,
}

impl OperatorTokens {
    /// Parse a token list. Invalid entries are skipped and reported (without
    /// the hash) so a typo cannot silently grant access.
    pub fn parse(spec: &str) -> (Self, Vec<String>) {
        let mut entries = Vec::new();
        let mut problems = Vec::new();
        for (index, entry) in spec.split(',').map(str::trim).filter(|e| !e.is_empty()).enumerate() {
            match parse_entry(entry) {
                Ok(parsed) => entries.push(parsed),
                Err(problem) => problems.push(format!("OPERATOR_TOKENS entry {}: {problem}", index + 1)),
            }
        }
        (Self { entries }, problems)
    }

    pub fn from_env() -> (Self, Vec<String>) {
        Self::parse(&std::env::var("OPERATOR_TOKENS").unwrap_or_default())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The operator behind the request's bearer token, if any. Every entry is
    /// compared in constant time, with no early exit.
    pub fn authenticate(&self, headers: &HeaderMap) -> Option<ActorContext> {
        let token = headers
            .get(header::AUTHORIZATION)?
            .to_str()
            .ok()?
            .strip_prefix("Bearer ")?
            .trim();
        if token.is_empty() {
            return None;
        }
        let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let mut found = None;
        for (hash, actor) in &self.entries {
            if constant_time_eq(hash, &digest) && found.is_none() {
                found = Some(actor.clone());
            }
        }
        found
    }
}

pub fn token_hash_hex(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn constant_time_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn parse_entry(entry: &str) -> Result<([u8; 32], ActorContext), String> {
    let parts: Vec<&str> = entry.split(':').collect();
    let [actor_id, roles, hash] = parts.as_slice() else {
        return Err("expected actor_id:ROLE[+ROLE]:sha256hex".into());
    };
    if actor_id.is_empty() || actor_id.len() > 64 {
        return Err("actor_id must be 1..=64 characters".into());
    }
    let mut parsed_roles = Vec::new();
    for role in roles.split('+') {
        let role = match role {
            "OBSERVER" => ActorRole::Observer,
            "DISPATCHER" => ActorRole::Dispatcher,
            "HAZARD_VERIFIER" => ActorRole::HazardVerifier,
            "RESPONDER" => ActorRole::Responder,
            "EOC_ADMIN" => ActorRole::EocAdmin,
            other => return Err(format!("role '{other}' is not an operator role")),
        };
        parsed_roles.push(role);
    }
    if hash.len() != 64 {
        return Err("hash must be 64 hex characters (SHA-256)".into());
    }
    let mut bytes = [0u8; 32];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hash[i * 2..i * 2 + 2], 16).map_err(|_| "hash is not hex".to_string())?;
    }
    Ok((bytes, ActorContext::new(*actor_id, parsed_roles)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Permission;
    use axum::http::HeaderValue;

    fn bearer(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, HeaderValue::from_str(&format!("Bearer {token}")).unwrap());
        headers
    }

    #[test]
    fn valid_token_authenticates_with_its_roles() {
        let spec = format!("asha:HAZARD_VERIFIER:{},obs:OBSERVER:{}", token_hash_hex("s3cret"), token_hash_hex("look"));
        let (tokens, problems) = OperatorTokens::parse(&spec);
        assert!(problems.is_empty(), "{problems:?}");
        let actor = tokens.authenticate(&bearer("s3cret")).unwrap();
        assert_eq!(actor.actor_id, "asha");
        assert!(actor.can(Permission::VerifyHazard));
        assert!(!tokens.authenticate(&bearer("look")).unwrap().can(Permission::VerifyHazard));
    }

    #[test]
    fn unknown_missing_or_malformed_tokens_do_not_authenticate() {
        let (tokens, _) = OperatorTokens::parse(&format!("asha:HAZARD_VERIFIER:{}", token_hash_hex("s3cret")));
        assert!(tokens.authenticate(&bearer("wrong")).is_none());
        assert!(tokens.authenticate(&HeaderMap::new()).is_none());
        let mut basic = HeaderMap::new();
        basic.insert(header::AUTHORIZATION, HeaderValue::from_static("Basic s3cret"));
        assert!(tokens.authenticate(&basic).is_none());
    }

    #[test]
    fn empty_or_invalid_configuration_fails_closed() {
        let (tokens, problems) = OperatorTokens::parse("");
        assert!(tokens.is_empty() && problems.is_empty());
        let (tokens, problems) = OperatorTokens::parse(&format!(
            "svc:SYSTEM_SERVICE:{h},short:EOC_ADMIN:abc,nocolon",
            h = token_hash_hex("x")
        ));
        assert!(tokens.is_empty());
        assert_eq!(problems.len(), 3);
        assert!(problems.iter().all(|p| !p.contains(&token_hash_hex("x"))), "hashes never echoed");
    }
}
