//! IFDB metadata lookup. Network only — never called except from an explicit
//! `f`/`r` keypress (see the fetch worker).
//!
//! Endpoint verified live 2026-07-16:
//!   GET https://ifdb.org/viewgame?ifiction&ifid=<IFID>   → iFiction XML
//! The cover is a SECOND request to the <coverart><url> in that response
//! (`https://ifdb.org/coverart?id=<tuid>&version=<n>`). There is no
//! `viewgame?coverart` endpoint — do not construct cover URLs.
//!
//! Not-found shape, confirmed live 2026-07-16 with a junk IFID
//! (`curl 'https://ifdb.org/viewgame?ifiction&ifid=ZCODE-1-000000-0000'`):
//! a genuine **HTTP 404**, body
//! `<viewgame xmlns="http://ifdb.org/api/xmlns"><errorCode>notFound</errorCode>
//! <errorMessage>No game was found matching the requested IFID.</errorMessage>
//! </viewgame>` — not the `<ifindex>`-with-no-`<story>` shape one might guess
//! at (same result for a UUID-shaped unknown IFID and a garbage IFID string;
//! an absent `ifid` param instead gets HTTP 400/`badRequest`). ureq surfaces
//! that 404 as `Err(ureq::Error::StatusCode(404))`, which is mapped to
//! `Ok(FetchOutcome::NotFound)` below.
//!
//! The parser also treats a well-formed `<ifindex>` with no `<story>` as
//! `IFictionError::NotIFiction` (see `ifiction.rs`); that specific parse
//! error is *also* mapped to `Ok(FetchOutcome::NotFound)` here, in case IFDB
//! ever answers that way for some other query shape. Both are "no record",
//! never a transport error.

use std::time::Duration;

use crate::ifiction::{self, IFiction, IFictionError};

const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_XML: u64 = 1024 * 1024; // 1 MiB
const MAX_COVER: u64 = 8 * 1024 * 1024; // 8 MiB

pub enum FetchOutcome {
    Found(Box<IFiction>),
    NotFound,
}

#[derive(Debug)]
pub enum FetchError {
    Transport(String),
}

pub trait MetadataSource: Send + Sync {
    fn fetch(&self, ifid: &str) -> Result<FetchOutcome, FetchError>;
    /// Fetch by IFDB page id (tuid), for a story the user pointed at an IFDB
    /// page by hand when the automatic IFID lookup found nothing (SQ-0371).
    fn fetch_by_id(&self, tuid: &str) -> Result<FetchOutcome, FetchError>;
    fn fetch_cover(&self, url: &str) -> Result<Vec<u8>, FetchError>;
}

pub struct IfdbClient {
    agent: ureq::Agent,
}

impl IfdbClient {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .user_agent(user_agent())
            .build();
        Self { agent: ureq::Agent::new_with_config(config) }
    }
}

impl Default for IfdbClient {
    fn default() -> Self {
        Self::new()
    }
}

impl IfdbClient {
    /// GET an IFDB `viewgame?ifiction…` URL and interpret the response.
    fn get_ifiction(&self, url: String) -> Result<FetchOutcome, FetchError> {
        match self.agent.get(url).call() {
            Ok(mut resp) => {
                let bytes = resp
                    .body_mut()
                    .with_config()
                    .limit(MAX_XML)
                    .read_to_vec()
                    .map_err(|e| FetchError::Transport(e.to_string()))?;
                match ifiction::parse(&bytes) {
                    Ok(f) => Ok(FetchOutcome::Found(Box::new(f))),
                    // A well-formed response with no <story> is IFDB's "no
                    // record" answer, not a failure to read the response.
                    Err(IFictionError::NotIFiction) => Ok(FetchOutcome::NotFound),
                    Err(e) => Err(FetchError::Transport(format!("{e:?}"))),
                }
            }
            // The naive Ok/Err split would treat "no record" as a transport
            // failure, which would leave no `not_found` block written and
            // make the library sweep re-request this story forever.
            Err(ureq::Error::StatusCode(404)) => Ok(FetchOutcome::NotFound),
            Err(e) => Err(FetchError::Transport(e.to_string())),
        }
    }

    /// One IFDB lookup for exactly this IFID string.
    fn fetch_one(&self, ifid: &str) -> Result<FetchOutcome, FetchError> {
        self.get_ifiction(ifiction_url(ifid))
    }
}

impl MetadataSource for IfdbClient {
    fn fetch(&self, ifid: &str) -> Result<FetchOutcome, FetchError> {
        let outcome = self.fetch_one(ifid)?;
        // IFDB does an exact IFID match, and it registers most Infocom games
        // under the bare `ZCODE-<release>-<serial>` form — without the trailing
        // header checksum our `compute_ifid` always appends. So a checksummed
        // lookup 404s even when IFDB has the game under the shorter IFID
        // (verified 2026-07-16: `ZCODE-52-871125` resolves, `…-FFFF` does not).
        // On a miss, retry once with the checksum stripped; the bare form's
        // answer — found or not — is final.
        if matches!(outcome, FetchOutcome::NotFound) {
            if let Some(bare) = strip_zcode_checksum(ifid) {
                return self.fetch_one(&bare);
            }
        }
        Ok(outcome)
    }

    fn fetch_by_id(&self, tuid: &str) -> Result<FetchOutcome, FetchError> {
        self.get_ifiction(ifiction_id_url(tuid))
    }

    fn fetch_cover(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        let mut resp =
            self.agent.get(url).call().map_err(|e| FetchError::Transport(e.to_string()))?;
        resp.body_mut()
            .with_config()
            .limit(MAX_COVER)
            .read_to_vec()
            .map_err(|e| FetchError::Transport(e.to_string()))
    }
}

fn user_agent() -> String {
    format!("lanthorn/{} (+https://github.com/sharkusk/lanthorn)", env!("CARGO_PKG_VERSION"))
}

fn ifiction_url(ifid: &str) -> String {
    format!("https://ifdb.org/viewgame?ifiction&ifid={}", percent_encode(ifid))
}

/// iFiction by IFDB page id (tuid). Verified 2026-07-16: `?ifiction&id=<tuid>`
/// returns the game's iFiction XML.
fn ifiction_id_url(tuid: &str) -> String {
    format!("https://ifdb.org/viewgame?ifiction&id={}", percent_encode(tuid))
}

/// Pull an IFDB page id (tuid) out of what a user pasted: a full page URL
/// (`https://ifdb.org/viewgame?id=<tuid>`, optionally with `&version=…`), a bare
/// `?id=<tuid>` query, or the tuid on its own. `None` if nothing tuid-shaped is
/// found (SQ-0371). The `?id=`/`&id=` boundary avoids matching the `ifid=` of an
/// iFiction URL.
pub fn extract_tuid(input: &str) -> Option<String> {
    let s = input.trim();
    if let Some(start) = ["?id=", "&id="].iter().find_map(|m| s.find(m).map(|i| i + m.len())) {
        let tuid: String = s[start..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        return (!tuid.is_empty()).then_some(tuid);
    }
    // A bare tuid: all-alphanumeric, no scheme or query punctuation.
    (!s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric())).then(|| s.to_string())
}

/// A `ZCODE-<release>-<serial>-<checksum>` IFID with its trailing 4-hex-digit
/// header checksum removed, giving the bare `ZCODE-<release>-<serial>` form
/// IFDB commonly indexes. `None` for anything else — a bare ZCODE IFID (the
/// serial is 6 chars, never 4 hex), a Glulx/UUID IFID, or any non-ZCODE string
/// — so only a genuinely-checksummed lookup ever triggers the retry.
fn strip_zcode_checksum(ifid: &str) -> Option<String> {
    let rest = ifid.strip_prefix("ZCODE-")?;
    let (prefix, last) = ifid.rsplit_once('-')?;
    // Guard against `ZCODE-<checksum>` with nothing between: require the prefix
    // to still carry the release-serial body after the leading `ZCODE-`.
    if !rest.contains('-') {
        return None;
    }
    (last.len() == 4 && last.bytes().all(|b| b.is_ascii_hexdigit())).then(|| prefix.to_string())
}

/// A human IFDB search page for `query` (a title). Offered when an automatic
/// fetch found nothing, so the user can look the story up by hand (SQ-0371).
/// Verified 2026-07-16: `ifdb.org/search?searchfor=…` returns 200.
pub fn search_url(query: &str) -> String {
    format!("https://ifdb.org/search?searchfor={}", percent_encode(query))
}

/// Hand-rolled: the one call site (an IFID reaching us from story bytes,
/// not guaranteed URL-safe) doesn't justify a `percent-encoding` dependency.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'~' | b'-' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(all(test, feature = "t-picker"))]
mod tests {
    use super::*;

    #[test]
    fn ifiction_url_is_the_verified_endpoint() {
        assert_eq!(
            ifiction_url("ZCODE-52-871125"),
            "https://ifdb.org/viewgame?ifiction&ifid=ZCODE-52-871125"
        );
    }

    /// An IFID reaches us from story bytes and is not guaranteed URL-safe.
    #[test]
    fn ifid_is_percent_encoded_into_the_query() {
        assert!(
            ifiction_url("A B&c=d").ends_with("ifid=A%20B%26c%3Dd"),
            "got {}",
            ifiction_url("A B&c=d")
        );
    }

    #[test]
    fn user_agent_identifies_lanthorn_and_its_repo() {
        let ua = user_agent();
        assert!(ua.starts_with("lanthorn/"));
        assert!(ua.contains("github.com/sharkusk/lanthorn"));
        assert!(ua.contains(env!("CARGO_PKG_VERSION")));
    }

    /// The cover URL is IFDB's, taken from the response — never constructed by
    /// us. `viewgame?coverart` does not exist; the real form is
    /// `ifdb.org/coverart?id=<tuid>&version=<n>` and only the response knows it.
    #[test]
    fn cover_url_comes_from_the_response_not_from_us() {
        let f = crate::ifiction::parse(include_bytes!("../tests/fixtures/ifdb-zork1.xml")).unwrap();
        assert_eq!(
            f.ifdb.unwrap().cover_url.as_deref(),
            Some("https://ifdb.org/coverart?id=0dbnusxunq7fw5ro&version=45")
        );
    }

    #[test]
    fn extract_tuid_handles_urls_and_bare_ids() {
        let t = "0dbnusxunq7fw5ro";
        assert_eq!(extract_tuid(&format!("https://ifdb.org/viewgame?id={t}")).as_deref(), Some(t));
        assert_eq!(extract_tuid(&format!("https://ifdb.org/coverart?id={t}&version=45")).as_deref(), Some(t));
        assert_eq!(extract_tuid(&format!("  {t}  ")).as_deref(), Some(t), "bare id, trimmed");
        // Must not mistake the `ifid=` of an iFiction URL for an `id=`.
        assert_eq!(extract_tuid("https://ifdb.org/viewgame?ifiction&ifid=ZCODE-5-1"), None);
        // Nothing tuid-shaped.
        assert_eq!(extract_tuid("not a url"), None);
        assert_eq!(extract_tuid(""), None);
    }

    #[test]
    fn ifiction_id_url_targets_the_page_id_endpoint() {
        assert_eq!(
            ifiction_id_url("0dbnusxunq7fw5ro"),
            "https://ifdb.org/viewgame?ifiction&id=0dbnusxunq7fw5ro"
        );
    }

    #[test]
    fn search_url_encodes_the_title_query() {
        assert_eq!(search_url("Zork I"), "https://ifdb.org/search?searchfor=Zork%20I");
        assert!(search_url("A&B").ends_with("searchfor=A%26B"));
    }

    #[test]
    fn strip_zcode_checksum_recovers_the_bare_ifid() {
        // Our compute_ifid always appends a 4-hex checksum; IFDB commonly
        // indexes Infocom games under the bare form, so this is the fallback key.
        assert_eq!(strip_zcode_checksum("ZCODE-52-871125-FFFF").as_deref(), Some("ZCODE-52-871125"));
        assert_eq!(strip_zcode_checksum("ZCODE-88-840726-A129").as_deref(), Some("ZCODE-88-840726"));
        // A bare ZCODE IFID has a 6-char serial as its last segment, not a
        // 4-hex checksum → nothing to strip, no pointless retry.
        assert_eq!(strip_zcode_checksum("ZCODE-52-871125"), None);
        // Non-ZCODE IFIDs (Glulx hash, Inform UUID) are never touched.
        assert_eq!(strip_zcode_checksum("GLULX-0123456789ABCDEF"), None);
        assert_eq!(strip_zcode_checksum("AC0DAF65-F40F-4A41-A4E4-50414F836E14"), None);
        // A serial ending in a dash-run must not be mistaken for a checksum.
        assert_eq!(strip_zcode_checksum("ZCODE-5-------"), None);
    }
}
