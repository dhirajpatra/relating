//! Enterprise identity: token comparison, roles, and OIDC JWT verification.
//!
//! Dependency-free by design: HMAC-SHA256 is built on the workspace `sha2`
//! crate and base64url is decoded by hand, so JWT verification works without
//! pulling an OIDC client stack. HS256 is verified today; RS256 tokens are
//! rejected with a message naming the follow-up (`jsonwebtoken` support).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Constant-time string equality for secret comparison.
///
/// Walks the full length of the longer input and folds length differences
/// into the accumulator, so early-exit timing oracles learn nothing beyond
/// rough length. Adequate for bearer-token comparison; not a substitute for
/// a formally verified primitive.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    let mut diff = ab.len() ^ bb.len();
    let max = ab.len().max(bb.len());
    for i in 0..max {
        diff |= (ab.get(i).copied().unwrap_or(0) ^ bb.get(i).copied().unwrap_or(0)) as usize;
    }
    diff == 0
}

/// sha256 hex digest, used for session-token hashes and the audit hash chain.
/// Session tokens are random 256-bit values; the digest lets the database
/// verify possession without storing the token itself.
pub fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    digest.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Granular roles, weakest first. Serialized `snake_case` over JSON and the DB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Viewer,
    Editor,
    Admin,
}

impl Role {
    pub fn parse(s: &str) -> Option<Role> {
        match s.trim().to_ascii_lowercase().as_str() {
            "viewer" | "read" | "reader" => Some(Role::Viewer),
            "editor" | "write" | "writer" => Some(Role::Editor),
            "admin" | "owner" => Some(Role::Admin),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Editor => "editor",
            Role::Admin => "admin",
        }
    }

    pub fn rank(self) -> u8 {
        match self {
            Role::Viewer => 0,
            Role::Editor => 1,
            Role::Admin => 2,
        }
    }

    pub fn at_least(self, minimum: Role) -> bool {
        self.rank() >= minimum.rank()
    }

    pub fn max(a: Option<Role>, b: Option<Role>) -> Option<Role> {
        match (a, b) {
            (Some(x), Some(y)) if x.rank() >= y.rank() => Some(x),
            (Some(_), Some(y)) => Some(y),
            (Some(x), None) => Some(x),
            (None, other) => other,
        }
    }
}

/// Who is making a request, after authentication.
#[derive(Debug, Clone)]
pub enum Actor {
    /// Open mode (no token and no OIDC configured): loopback only.
    Anonymous,
    /// The static `RELATING_API_TOKEN`: break-glass administrator.
    ApiToken,
    /// An OIDC-authenticated user.
    User { id: String, email: String },
}

impl Actor {
    /// Stable display string for audit rows and `approved_by` stamps.
    /// Never contains secret material.
    pub fn display(&self) -> String {
        match self {
            Actor::Anonymous => "anonymous".to_string(),
            Actor::ApiToken => "api-token".to_string(),
            Actor::User { email, .. } => format!("user:{}", email),
        }
    }

    pub fn is_identified_user(&self) -> bool {
        matches!(self, Actor::User { .. })
    }
}

/// Authorization context resolved per request: the actor plus the roles
/// that apply to this request's scope.
#[derive(Debug, Clone)]
pub struct ActorContext {
    pub actor: Actor,
    /// Strongest global (project-less) role, if any.
    pub global_role: Option<Role>,
    /// Strongest role for the project in the request path, if any.
    pub project_role: Option<Role>,
    pub global_admin: bool,
}

impl ActorContext {
    pub fn anonymous() -> Self {
        Self {
            actor: Actor::Anonymous,
            global_role: None,
            project_role: None,
            global_admin: false,
        }
    }

    pub fn api_token() -> Self {
        Self {
            actor: Actor::ApiToken,
            global_role: Some(Role::Admin),
            project_role: Some(Role::Admin),
            global_admin: true,
        }
    }

    /// Effective role: the stronger of the global and project grants.
    pub fn effective_role(&self) -> Option<Role> {
        Role::max(self.global_role, self.project_role)
    }
}

/// Pure RBAC policy: method + path against the resolved context.
/// Returns `Ok` when allowed, `Err(reason)` for a 403 with that message.
///
/// - Anonymous passes only in open mode (locked mode rejects earlier).
/// - Static API tokens are administrators.
/// - OIDC users need a viewer grant to read, an editor grant to write,
///   and global admin for grant management. Drift checks are reads even
///   though they POST. Project creation needs a global editor grant; the
///   creator is auto-granted admin on the new project.
pub fn authorize(ctx: &ActorContext, method: &str, path: &str) -> Result<(), String> {
    match &ctx.actor {
        Actor::Anonymous | Actor::ApiToken => return Ok(()),
        Actor::User { .. } => {}
    }
    if path.contains("/grants") {
        return if ctx.global_admin {
            Ok(())
        } else {
            Err("Grant management requires a global admin role".to_string())
        };
    }
    // Identity lookup works for any authenticated user, including grantless
    // ones who need to learn their own address to request access.
    if path == "/api/v1/auth/me" {
        return Ok(());
    }
    let is_write = !matches!(method, "GET" | "HEAD" | "OPTIONS");
    // Drift checks and connection probes POST but never write.
    let read_only_post = method == "POST" && (path.ends_with("/drift") || path.ends_with("/test"));
    let effective = ctx.effective_role();
    if !is_write || read_only_post {
        return match effective {
            Some(_) => Ok(()),
            None => Err("This project needs a viewer grant for your account".to_string()),
        };
    }
    match effective {
        Some(role) if role.at_least(Role::Editor) => Ok(()),
        Some(_) => Err("This action needs an editor grant for your account".to_string()),
        None => Err("This project needs an editor grant for your account".to_string()),
    }
}

/// OIDC relying-party configuration, snapshotted from the environment at boot:
/// `RELATING_OIDC_ISSUER` + `RELATING_OIDC_AUDIENCE` enable SSO;
/// `RELATING_OIDC_CLIENT_SECRET` verifies HS256 id tokens;
/// `RELATING_OIDC_AUTH_URL` overrides `{issuer}/authorize` for login.
#[derive(Debug, Clone)]
pub struct OidcConfig {
    pub issuer: String,
    pub audience: String,
    pub client_secret: Option<String>,
    pub auth_url: String,
}

impl OidcConfig {
    pub fn from_env() -> Option<OidcConfig> {
        let issuer = std::env::var("RELATING_OIDC_ISSUER")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())?;
        let audience = std::env::var("RELATING_OIDC_AUDIENCE")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())?;
        let client_secret = std::env::var("RELATING_OIDC_CLIENT_SECRET")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let auth_url = std::env::var("RELATING_OIDC_AUTH_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| format!("{}/authorize", issuer.trim_end_matches('/')));
        Some(OidcConfig {
            issuer,
            audience,
            client_secret,
            auth_url,
        })
    }

    /// Authorization URL for the frontend to redirect the browser to.
    /// The frontend completes the provider flow and posts the id token back
    /// to `/auth/callback` with the round-tripped state.
    pub fn login_url(&self, redirect_uri: &str, state: &str, nonce: &str) -> String {
        format!(
            "{}?response_type=id_token&client_id={}&redirect_uri={}&scope={}&state={}&nonce={}",
            self.auth_url,
            url_encode(&self.audience),
            url_encode(redirect_uri),
            url_encode("openid email profile"),
            url_encode(state),
            url_encode(nonce),
        )
    }
}

fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

/// Verified OIDC id-token claims (the subset the server enforces).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtClaims {
    pub iss: String,
    pub sub: String,
    pub aud: String,
    pub exp: i64,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

/// Seconds of clock-skew leeway accepted on `exp`.
pub const JWT_LEEWAY_SECS: i64 = 30;

/// Verify an HS256 JWT id token against the OIDC configuration.
/// `now_unix` is injected so tests control time without sleeping.
pub fn verify_jwt_hs256(
    token: &str,
    secret: &str,
    expected_iss: &str,
    expected_aud: &str,
    now_unix: i64,
) -> Result<JwtClaims, String> {
    let mut parts = token.split('.');
    let (header_b64, payload_b64, sig_b64) = match (parts.next(), parts.next(), parts.next()) {
        (Some(h), Some(p), Some(s)) if parts.next().is_none() => (h, p, s),
        _ => return Err("Malformed JWT: expected three dot-separated parts".to_string()),
    };
    let header_json: serde_json::Value = serde_json::from_slice(
        &base64url_decode(header_b64).map_err(|e| format!("Malformed JWT header: {}", e))?,
    )
    .map_err(|_| "Malformed JWT header JSON".to_string())?;
    match header_json.get("alg").and_then(|v| v.as_str()) {
        Some("HS256") => {}
        Some("RS256") | Some("ES256") | Some("RS384") | Some("ES384") => {
            return Err(
                "Asymmetric JWT signatures (RS256/ES256) need jsonwebtoken support; configure an HS256 client or wait for the RS256 follow-up"
                    .to_string(),
            );
        }
        Some(other) => return Err(format!("Unsupported JWT alg '{}'", other)),
        None => return Err("JWT header is missing alg".to_string()),
    }
    let signing_input = format!("{}.{}", header_b64, payload_b64);
    let expected_sig = hmac_sha256(secret.as_bytes(), signing_input.as_bytes());
    let presented_sig =
        base64url_decode(sig_b64).map_err(|e| format!("Malformed JWT signature: {}", e))?;
    if presented_sig.len() != expected_sig.len()
        || !presented_sig
            .iter()
            .zip(expected_sig.iter())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    {
        return Err("JWT signature mismatch".to_string());
    }
    let payload: serde_json::Value = serde_json::from_slice(
        &base64url_decode(payload_b64).map_err(|e| format!("Malformed JWT payload: {}", e))?,
    )
    .map_err(|_| "Malformed JWT payload JSON".to_string())?;
    let iss = payload
        .get("iss")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if iss != expected_iss {
        return Err(format!(
            "JWT issuer '{}' does not match '{}'",
            iss, expected_iss
        ));
    }
    let aud_ok = match payload.get("aud") {
        Some(serde_json::Value::String(aud)) => aud == expected_aud,
        Some(serde_json::Value::Array(auds)) => {
            auds.iter().any(|v| v.as_str() == Some(expected_aud))
        }
        _ => false,
    };
    if !aud_ok {
        return Err("JWT audience does not match this server".to_string());
    }
    let exp = payload.get("exp").and_then(|v| v.as_i64()).unwrap_or(0);
    if exp + JWT_LEEWAY_SECS <= now_unix {
        return Err("JWT is expired".to_string());
    }
    let sub = payload
        .get("sub")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if sub.is_empty() {
        return Err("JWT is missing sub".to_string());
    }
    Ok(JwtClaims {
        iss: iss.to_string(),
        sub,
        aud: expected_aud.to_string(),
        exp,
        email: payload
            .get("email")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        name: payload
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    })
}

/// True when the bearer value has JWT shape (three dot-separated parts).
pub fn is_jwt_shape(token: &str) -> bool {
    let mut parts = token.split('.');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(_), Some(_), Some(_), None)
    )
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut key_block = [0u8; 64];
    if key.len() > 64 {
        key_block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(message);
    let inner_digest = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_digest);
    outer.finalize().into()
}

/// RFC 4648 base64url without padding (JWT alphabet); padded input accepted.
pub fn base64url_decode(input: &str) -> Result<Vec<u8>, String> {
    fn sextet(c: u8) -> Result<u8, String> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'-' => Ok(62),
            b'_' => Ok(63),
            b'=' => Err("padding".to_string()),
            _ => Err(format!("invalid base64url character '{}'", c as char)),
        }
    }
    let stripped: String = input.chars().filter(|c| *c != '=').collect();
    let mut out = Vec::with_capacity(stripped.len() * 3 / 4);
    let bytes = stripped.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let mut group = [0u8; 4];
        let mut count = 0;
        for k in 0..4 {
            if i + k >= bytes.len() {
                break;
            }
            group[k] = sextet(bytes[i + k]).map_err(|e| {
                if e == "padding" {
                    "misplaced base64url padding".to_string()
                } else {
                    e
                }
            })?;
            count += 1;
        }
        if count < 2 {
            return Err("truncated base64url input".to_string());
        }
        out.push((group[0] << 2) | (group[1] >> 4));
        if count > 2 {
            out.push(((group[1] & 0x0f) << 4) | (group[2] >> 2));
        }
        if count > 3 {
            out.push(((group[2] & 0x03) << 6) | group[3]);
        }
        i += 4;
    }
    Ok(out)
}

/// RFC 4648 base64url without padding (JWT alphabet).
pub fn base64url_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = chunk.get(1).copied().unwrap_or(0) as usize;
        let b2 = chunk.get(2).copied().unwrap_or(0) as usize;
        out.push(ALPHABET[b0 >> 2] as char);
        out.push(ALPHABET[((b0 & 0x03) << 4) | (b1 >> 4)] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((b1 & 0x0f) << 2) | (b2 >> 6)] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[b2 & 0x3f] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_time_eq_vectors() {
        assert!(constant_time_eq("", ""));
        assert!(constant_time_eq("token-abc", "token-abc"));
        assert!(!constant_time_eq("token-abc", "token-abd"));
        assert!(!constant_time_eq("short", "longer"));
        assert!(!constant_time_eq("longer", "short"));
        assert!(!constant_time_eq("", "nonempty"));
    }

    #[test]
    fn test_hmac_sha256_rfc4231_case_2() {
        // RFC 4231 §4.2 test case 2 (HMAC-SHA-256, key "Jefe").
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        let hex: String = mac.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(
            hex,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn test_base64url_round_trip_and_vectors() {
        assert_eq!(base64url_encode(b"f"), "Zg");
        assert_eq!(base64url_encode(b"fo"), "Zm8");
        assert_eq!(base64url_encode(b"foo"), "Zm9v");
        assert_eq!(base64url_encode(b">>>"), "Pj4-");
        assert_eq!(base64url_decode("Zg").unwrap(), b"f");
        assert_eq!(base64url_decode("Zm8=").unwrap(), b"fo");
        assert_eq!(base64url_decode("Pj4-").unwrap(), b">>>");
        assert!(base64url_decode("***").is_err());
        let bytes: Vec<u8> = (0..=255u8).collect();
        assert_eq!(base64url_decode(&base64url_encode(&bytes)).unwrap(), bytes);
    }

    fn mint_hs256(secret: &str, payload: serde_json::Value) -> String {
        let header = base64url_encode(br#"{"alg":"HS256","typ":"JWT"}"#);
        let body = base64url_encode(payload.to_string().as_bytes());
        let mac = hmac_sha256(secret.as_bytes(), format!("{}.{}", header, body).as_bytes());
        format!("{}.{}.{}", header, body, base64url_encode(&mac))
    }

    #[test]
    fn test_verify_jwt_hs256_round_trip_and_rejections() {
        let now = 1_800_000_000i64;
        let payload = serde_json::json!({
            "iss": "https://sso.example",
            "sub": "user-1",
            "aud": "relating",
            "exp": now + 300,
            "email": "ada@example.com",
        });
        let token = mint_hs256("secret", payload);
        let claims =
            verify_jwt_hs256(&token, "secret", "https://sso.example", "relating", now).unwrap();
        assert_eq!(claims.sub, "user-1");
        assert_eq!(claims.email.as_deref(), Some("ada@example.com"));

        assert!(verify_jwt_hs256(&token, "wrong", "https://sso.example", "relating", now).is_err());
        assert!(
            verify_jwt_hs256(&token, "secret", "https://evil.example", "relating", now).is_err()
        );
        assert!(verify_jwt_hs256(&token, "secret", "https://sso.example", "other", now).is_err());
        assert!(verify_jwt_hs256(
            &token,
            "secret",
            "https://sso.example",
            "relating",
            now + 600
        )
        .is_err());
        assert!(verify_jwt_hs256(
            "not-a-jwt",
            "secret",
            "https://sso.example",
            "relating",
            now
        )
        .is_err());
        // Tampered payload invalidates the signature.
        let mut tampered = token.clone();
        tampered.push('A');
        assert!(
            verify_jwt_hs256(&tampered, "secret", "https://sso.example", "relating", now).is_err()
        );
        // Array audiences accept a matching member.
        let array_aud = mint_hs256(
            "secret",
            serde_json::json!({"iss": "https://sso.example", "sub": "u", "aud": ["x", "relating"], "exp": now + 60}),
        );
        assert!(
            verify_jwt_hs256(&array_aud, "secret", "https://sso.example", "relating", now).is_ok()
        );
    }

    #[test]
    fn test_verify_jwt_rejects_asymmetric_algs_with_follow_up() {
        let header = base64url_encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let body = base64url_encode(br#"{"iss":"i","sub":"s","aud":"a","exp":9999999999}"#);
        let token = format!("{}.{}.{}", header, body, base64url_encode(b"sig"));
        let err = verify_jwt_hs256(&token, "secret", "i", "a", 1).unwrap_err();
        assert!(err.contains("jsonwebtoken"), "{}", err);
    }

    #[test]
    fn test_role_parse_rank_and_max() {
        assert_eq!(Role::parse("ADMIN"), Some(Role::Admin));
        assert_eq!(Role::parse("write"), Some(Role::Editor));
        assert_eq!(Role::parse("reader"), Some(Role::Viewer));
        assert_eq!(Role::parse("superuser"), None);
        assert!(Role::Admin.at_least(Role::Editor));
        assert!(!Role::Viewer.at_least(Role::Editor));
        assert_eq!(
            Role::max(Some(Role::Viewer), Some(Role::Editor)),
            Some(Role::Editor)
        );
        assert_eq!(Role::max(None, None), None);
    }

    #[test]
    fn test_authorize_matrix() {
        let viewer = ActorContext {
            actor: Actor::User {
                id: "u".to_string(),
                email: "v@example.com".to_string(),
            },
            global_role: None,
            project_role: Some(Role::Viewer),
            global_admin: false,
        };
        assert!(authorize(&viewer, "GET", "/api/v1/projects/p/sources").is_ok());
        assert!(authorize(&viewer, "POST", "/api/v1/projects/p/sources").is_err());
        assert!(authorize(&viewer, "POST", "/api/v1/projects/p/migrations/m/drift").is_ok());
        assert!(authorize(&viewer, "POST", "/api/v1/projects/p/connectors/c/test").is_ok());
        assert!(authorize(&viewer, "GET", "/api/v1/projects/p/grants").is_err());

        let editor = ActorContext {
            project_role: Some(Role::Editor),
            ..viewer.clone()
        };
        assert!(authorize(&editor, "POST", "/api/v1/projects/p/migrations/plan").is_ok());
        assert!(authorize(&editor, "POST", "/api/v1/projects/p/grants").is_err());

        let admin = ActorContext {
            global_admin: true,
            global_role: Some(Role::Admin),
            project_role: None,
            ..viewer.clone()
        };
        assert!(authorize(&admin, "POST", "/api/v1/projects/p/grants").is_ok());
        assert!(authorize(&admin, "POST", "/api/v1/projects").is_ok());

        let nog = ActorContext {
            global_role: None,
            project_role: None,
            global_admin: false,
            ..viewer.clone()
        };
        assert!(authorize(&nog, "GET", "/api/v1/projects/p/sources").is_err());
        assert!(authorize(&nog, "GET", "/api/v1/auth/me").is_ok());
        assert!(authorize(&ActorContext::api_token(), "DELETE", "/api/v1/projects/p").is_ok());
        assert!(authorize(&ActorContext::anonymous(), "GET", "/api/v1/projects").is_ok());
    }

    #[test]
    fn test_oidc_login_url_encodes_params() {
        let cfg = OidcConfig {
            issuer: "https://sso.example".to_string(),
            audience: "relating".to_string(),
            client_secret: None,
            auth_url: "https://sso.example/authorize".to_string(),
        };
        let url = cfg.login_url("https://app.example/cb?x=1", "s t", "n/n");
        assert!(url.contains("client_id=relating"), "{}", url);
        assert!(
            url.contains("redirect_uri=https%3A%2F%2Fapp.example%2Fcb%3Fx%3D1"),
            "{}",
            url
        );
        assert!(url.contains("state=s%20t"), "{}", url);
    }
}
