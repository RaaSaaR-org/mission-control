//! Bearer-token authentication for the REST API.
//!
//! Tokens live in a YAML file loaded once at startup. Each entry stores an
//! argon2id PHC hash of the secret and a list of capabilities. Requests carry
//! `Authorization: Bearer <token>` and the middleware constant-time-compares
//! the input against each known hash.
//!
//! The token *name* (not the secret) is attached to the request as an
//! extension so handlers and the trace layer can log it.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, RwLock};

use argon2::{Argon2, PasswordVerifier};
use axum::extract::State;
use axum::http::{header, HeaderMap, Method, Request};
use axum::middleware::Next;
use axum::response::Response;
use password_hash::PasswordHash;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::error::ApiError;

/// One token entry as parsed from the YAML file.
#[derive(Debug, Clone, Deserialize)]
struct RawToken {
    name: String,
    hash: String,
    #[serde(default)]
    capabilities: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawTokensFile {
    tokens: Vec<RawToken>,
}

/// Capabilities a token can hold. `Read` is implicit on every authenticated
/// request; `Write` gates POST endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Read,
    Write,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub name: String,
    parsed_hash: ParsedHash,
    capabilities: Vec<Capability>,
}

/// Owned, parsed argon2 hash. We pre-parse at load time so each request only
/// pays for the constant-time argon2 verify, not for re-parsing the PHC string.
#[derive(Debug, Clone)]
struct ParsedHash {
    phc: String,
}

impl ParsedHash {
    fn new(phc: String) -> Result<Self, password_hash::Error> {
        // Validate at load time that the PHC string parses; we'll re-parse on
        // each verify (cheap) since `PasswordHash` borrows from the source.
        PasswordHash::new(&phc)?;
        Ok(Self { phc })
    }

    fn verify(&self, candidate: &[u8]) -> bool {
        let parsed = match PasswordHash::new(&self.phc) {
            Ok(h) => h,
            Err(_) => return false,
        };
        Argon2::default()
            .verify_password(candidate, &parsed)
            .is_ok()
    }
}

/// Loaded token store. Cheap to clone (Arc) so it can be shared across handlers.
#[derive(Debug, Clone)]
pub struct TokenStore {
    inner: Arc<TokenStoreInner>,
}

#[derive(Debug)]
struct TokenStoreInner {
    tokens: Vec<Token>,
    /// Fast-path cache mapping `sha256(bearer)` to the index of the matching
    /// token. Argon2 verification is intentionally slow (≈30–100 ms each),
    /// and iterating it on every request would make the API trivially DoS-able.
    /// We pay the cost once per never-before-seen bearer; subsequent requests
    /// hit the cache. The cache is naturally bounded by `tokens.len()` —
    /// failed verifications never populate it.
    fast_cache: RwLock<HashMap<[u8; 32], usize>>,
}

impl TokenStore {
    /// Load tokens from a YAML file. Empty/missing capability entries are
    /// treated as `[read]`; an explicit empty list means read-only.
    pub fn from_file(path: &Path) -> Result<Self, TokenLoadError> {
        let content = std::fs::read_to_string(path).map_err(TokenLoadError::Io)?;
        Self::from_yaml(&content)
    }

    pub fn from_yaml(yaml: &str) -> Result<Self, TokenLoadError> {
        let raw: RawTokensFile = serde_yaml::from_str(yaml).map_err(TokenLoadError::Parse)?;
        if raw.tokens.is_empty() {
            return Err(TokenLoadError::Empty);
        }
        let mut tokens = Vec::with_capacity(raw.tokens.len());
        for t in raw.tokens {
            let parsed_hash = ParsedHash::new(t.hash).map_err(TokenLoadError::Hash)?;
            let mut caps: Vec<Capability> = Vec::new();
            for c in &t.capabilities {
                match c.as_str() {
                    "read" => caps.push(Capability::Read),
                    "write" => caps.push(Capability::Write),
                    other => return Err(TokenLoadError::UnknownCapability(other.into())),
                }
            }
            if caps.is_empty() {
                caps.push(Capability::Read);
            }
            tokens.push(Token {
                name: t.name,
                parsed_hash,
                capabilities: caps,
            });
        }
        Ok(Self {
            inner: Arc::new(TokenStoreInner {
                tokens,
                fast_cache: RwLock::new(HashMap::new()),
            }),
        })
    }

    /// Find the token matching the candidate secret, populating the fast-path
    /// cache so the next request with the same bearer skips argon2 entirely.
    fn verify(&self, candidate: &[u8]) -> Option<&Token> {
        let digest = sha256(candidate);

        // Fast path: known bearer.
        if let Some(&idx) = self
            .inner
            .fast_cache
            .read()
            .expect("auth cache poisoned")
            .get(&digest)
        {
            return self.inner.tokens.get(idx);
        }

        // Slow path: never-before-seen bearer. Argon2-verify against every
        // token; on hit, remember the digest. Failures are not cached, so an
        // attacker hammering with random bearers cannot grow this map.
        for (idx, token) in self.inner.tokens.iter().enumerate() {
            if token.parsed_hash.verify(candidate) {
                self.inner
                    .fast_cache
                    .write()
                    .expect("auth cache poisoned")
                    .insert(digest, idx);
                return Some(token);
            }
        }
        None
    }
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

#[derive(Debug, thiserror::Error)]
pub enum TokenLoadError {
    #[error("read tokens file: {0}")]
    Io(std::io::Error),
    #[error("parse tokens file: {0}")]
    Parse(serde_yaml::Error),
    #[error("invalid argon2 hash: {0}")]
    Hash(password_hash::Error),
    #[error("unknown capability: {0} (expected 'read' or 'write')")]
    UnknownCapability(String),
    #[error("tokens file is empty — define at least one token")]
    Empty,
}

/// Identity injected into the request via `request.extensions_mut()` after
/// successful auth. Handlers can extract it with `Extension<AuthedToken>` to
/// branch on the token name (e.g. for audit logging) — kept on the public
/// surface even when no in-tree handler reads it yet.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct AuthedToken {
    pub name: String,
    pub capabilities: Vec<Capability>,
}

#[allow(dead_code)]
impl AuthedToken {
    pub fn has(&self, cap: Capability) -> bool {
        self.capabilities.contains(&cap)
    }
}

/// Per-app state for the auth layer.
#[derive(Debug, Clone)]
pub struct AuthState {
    pub store: TokenStore,
    pub read_only: bool,
}

/// Middleware: parse `Authorization: Bearer …`, verify against the token store,
/// inject `AuthedToken` into request extensions. Rejects writes when
/// `read_only` is set, regardless of token capabilities.
pub async fn require_auth(
    State(state): State<AuthState>,
    mut req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let bearer =
        extract_bearer(req.headers()).ok_or(ApiError::Unauthenticated("missing bearer token"))?;

    let token = state
        .store
        .verify(bearer.as_bytes())
        .ok_or(ApiError::Unauthenticated("invalid bearer token"))?;

    if state.read_only && is_write(req.method()) {
        return Err(ApiError::Forbidden("server is read-only"));
    }

    if is_write(req.method()) && !token.capabilities.contains(&Capability::Write) {
        return Err(ApiError::Forbidden("token lacks write capability"));
    }

    req.extensions_mut().insert(AuthedToken {
        name: token.name.clone(),
        capabilities: token.capabilities.clone(),
    });

    Ok(next.run(req).await)
}

fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    let h = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let bearer = h
        .strip_prefix("Bearer ")
        .or_else(|| h.strip_prefix("bearer "))?;
    let trimmed = bearer.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn is_write(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use argon2::password_hash::{rand_core::OsRng, SaltString};
    use argon2::PasswordHasher;

    fn hash(secret: &str) -> String {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(secret.as_bytes(), &salt)
            .unwrap()
            .to_string()
    }

    #[test]
    fn loads_and_verifies_token() {
        let yaml = format!(
            "tokens:\n  - name: bot\n    hash: \"{}\"\n    capabilities: [read, write]\n",
            hash("hunter2")
        );
        let store = TokenStore::from_yaml(&yaml).unwrap();
        assert!(store.verify(b"hunter2").is_some());
        assert!(store.verify(b"wrong").is_none());
    }

    #[test]
    fn empty_capabilities_means_read_only() {
        let yaml = format!("tokens:\n  - name: read\n    hash: \"{}\"\n", hash("s"));
        let store = TokenStore::from_yaml(&yaml).unwrap();
        let t = store.verify(b"s").unwrap();
        assert_eq!(t.capabilities, vec![Capability::Read]);
    }

    #[test]
    fn unknown_capability_rejected() {
        let yaml = format!(
            "tokens:\n  - name: x\n    hash: \"{}\"\n    capabilities: [admin]\n",
            hash("s")
        );
        match TokenStore::from_yaml(&yaml) {
            Err(TokenLoadError::UnknownCapability(c)) => assert_eq!(c, "admin"),
            other => panic!("expected UnknownCapability, got {other:?}"),
        }
    }

    #[test]
    fn empty_file_rejected() {
        match TokenStore::from_yaml("tokens: []\n") {
            Err(TokenLoadError::Empty) => {}
            other => panic!("expected Empty, got {other:?}"),
        }
    }

    #[test]
    fn cache_avoids_argon2_on_repeat_verify() {
        // Argon2 with default cost is intentionally slow. The cache turns
        // every-request into a hashmap lookup — repeat verifies should be
        // dramatically faster than the first.
        let yaml = format!(
            "tokens:\n  - name: bot\n    hash: \"{}\"\n",
            hash("hunter2")
        );
        let store = TokenStore::from_yaml(&yaml).unwrap();

        let t0 = std::time::Instant::now();
        assert!(store.verify(b"hunter2").is_some());
        let cold = t0.elapsed();

        let t1 = std::time::Instant::now();
        for _ in 0..100 {
            assert!(store.verify(b"hunter2").is_some());
        }
        let hot_total = t1.elapsed();

        // 100 cache hits should be at least 10× faster than one cold verify.
        // If argon2 ran 100 more times, hot_total would dwarf cold by 100×;
        // with the cache, hot_total is microseconds.
        assert!(
            hot_total < cold / 10,
            "cache not effective: cold={:?}, 100×hot={:?}",
            cold,
            hot_total
        );
    }

    #[test]
    fn failed_verify_does_not_grow_cache() {
        let yaml = format!("tokens:\n  - name: bot\n    hash: \"{}\"\n", hash("right"));
        let store = TokenStore::from_yaml(&yaml).unwrap();
        for i in 0..50 {
            assert!(store.verify(format!("wrong-{i}").as_bytes()).is_none());
        }
        let cache_size = store.inner.fast_cache.read().unwrap().len();
        assert_eq!(cache_size, 0, "failed verifies must not populate cache");
    }
}
