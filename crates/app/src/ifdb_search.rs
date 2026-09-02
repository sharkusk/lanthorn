//! IFDB story search + download for the story picker (SQ-0413).
//!
//! Network only — every request is driven by an explicit user action (a typed
//! search, a chosen download). One request in flight at a time (the picker's
//! [`SearchWorker`] is a single serial thread); nothing here prefetches,
//! crawls, or bulk-enumerates. All requests carry lanthorn's descriptive
//! User-Agent, which is also required to get past IFDB's Cloudflare bot filter
//! (see the terms notes below).
//!
//! ─────────────────────────────────────────────────────────────────────────
//! ENDPOINTS — all verified live 2026-07-23 with curl, using ONLY the two
//! documented public IFDB APIs (search + viewgame). No HTML scraping.
//!
//! (1) Search (documented at https://ifdb.org/api/search):
//!      GET https://ifdb.org/search?xml&game&searchfor=<query>
//!    The `xml` flag selects the "legacy" XML variant of the same search
//!    engine the website uses (JSON is IFDB's recommended default, but this
//!    repo parses XML with roxmltree for iFiction already, so XML reuses that
//!    toolchain rather than adding a JSON dependency). `game` restricts the
//!    object type to game listings. Response shape (namespace
//!    `http://ifdb.org/api/xmlns`), verified live for `searchfor=Deep+Space+Drifter`:
//!      <searchReply xmlns="http://ifdb.org/api/xmlns"><games>
//!        <game>
//!          <tuid>k82q3libhff6ks8l</tuid>
//!          <title>Deep Space Drifter</title>
//!          <link>https://ifdb.org/viewgame?id=k82q3libhff6ks8l</link>
//!          <author>Michael J. Roberts and Steve McAdams</author>
//!          <hasCoverArt/>                     (empty element; absent if no art)
//!          <devsys>TADS 2</devsys>
//!          <published><machine>1990</machine><printable>1990</printable></published>
//!          <averageRating>2.428571428</averageRating>   (may be EMPTY: <averageRating></averageRating>)
//!          <numRatings>7</numRatings>
//!          <starRating>2.5</starRating>                  (rounded ½-star; may be EMPTY)
//!          <starSort>2.16…</starSort>
//!          <coverArtLink>…?id=…&amp;version=5</coverArtLink>  (absent if no art)
//!        </game> … up to ~100 games …
//!      </games></searchReply>
//!    Zero results (verified with a nonsense query): a well-formed reply with
//!    an EMPTY group — `<searchReply …><games></games></searchReply>` — NOT an
//!    error. Error shape (documented): `<searchReply><error>message</error></searchReply>`.
//!    Omitting the `game` object-type flag still defaults to a game search
//!    (verified: `?xml&searchfor=zork` returned `<games>`).
//!
//! (1b) "Popular on IFDB" seed list (SQ-0473) — shown when the search modal
//!    opens, before the user types anything. The documented search API has no
//!    `sort`/`sortby` parameter of its own; the *site's* regular search UI
//!    does (reverse-engineered from `https://ifdb.org/search?browse`'s HTML,
//!    which lists `sortby=ratu` as "Highest Rated First" among others), and it
//!    composes with the same `xml` endpoint — verified live 2026-07-23:
//!      GET https://ifdb.org/search?xml&game&searchfor=rating%3A4-%20%23ratings%3A10-&sortby=ratu
//!    i.e. `searchfor=rating:4- #ratings:10-` (both documented search
//!    operators: 4+ average stars, 10+ total ratings — see
//!    `https://ifdb.org/search?searchhelp`) plus `&sortby=ratu`. Same
//!    `<searchReply><games><game>…` shape as a normal search (confirmed live;
//!    no new parser needed). `sortby=ratu` was verified to order by `starSort`
//!    (the Evan Miller confidence score already in every reply, see above), NOT
//!    raw `averageRating` — e.g. a live reply had averageRating 4.638/4.683 in
//!    the opposite order their starSort (4.561/4.484) puts them. That makes it
//!    IFDB's own confidence-weighted "Top" ranking, not a naive rating sort —
//!    the closest documented-API approximation of "hot" IFDB offers, and the
//!    reason this module picks it over a plain `rating:` sort or a "newest"
//!    list. The `rating:4- #ratings:10-` floor keeps out one-rating flukes;
//!    `sortby=ratu` alone (with an empty `searchfor`) returns zero results, so
//!    the floor is load-bearing, not just cosmetic. One request, issued once
//!    per modal *instance* (not persisted across separate `/` opens — see
//!    [`SearchModal::open`]); no polling, no extra pages.
//!
//! (2) Download links (documented at https://ifdb.org/api/viewgame):
//!      GET https://ifdb.org/viewgame?ifiction&id=<tuid>
//!    This is the SAME iFiction XML endpoint `ifdb.rs` already uses for
//!    metadata; its `<story>` carries a `<downloads><links>` block (verified
//!    live for Deep Space Drifter and present in tests/fixtures/ifdb-zork1.xml):
//!      <downloads><links>
//!        <link>
//!          <url>https://ifarchive.org/if-archive/games/tads/deepspace.zip</url>
//!          <title>deepspace.zip</title>
//!          <desc>…</desc>
//!          <isGame/>                          (empty element; a playable version)
//!          <format>tads2</format>             (from IFDB's File Format list)
//!          <compression>zip</compression>     (absent for a raw story file)
//!          <compressedPrimary>deep.gam</compressedPrimary>
//!        </link> …
//!      </links></downloads>
//!    Direct, uncompressed story files (e.g. Zork's
//!    `https://eblong.com/infocom/gamefiles/zork1-invclues-r52-s871125.z5`,
//!    `<isGame/>`, `<format>zcode</format>`, NO `<compression>`) are what this
//!    module offers to download. See [`is_playable_download`].
//!
//! (3) File hosts — links point at ifarchive.org, eblong.com, etc. Verified
//!    `curl -I` on both an eblong.com `.z5` and an ifarchive.org `.z5`:
//!    HTTP 200, `content-type: application/x-zmachine`, no redirect, honest
//!    `content-length`. ifarchive.org has no `robots.txt` and serves via
//!    Cloudflare with `access-control-allow-origin: *`.
//!
//! ─────────────────────────────────────────────────────────────────────────
//! TERMS OF USE — verified live 2026-07-23; this feature complies by construction.
//!
//! IFDB API docs (https://ifdb.org/api/) explicitly invite this use:
//!   "IFDB offers a few Application Program Interfaces (APIs) that let third
//!    parties create Web-based or client-side software that accesses IFDB's
//!    data." … "viewgame: … This allows client programs, such as IF browsers
//!    or 'jukeboxes', to take advantage of IFDB's structured data."
//!   Caveats: "designed primarily for low-volume, user-driven applications,
//!    rather than big batches of automated requests. They're not really meant
//!    for constantly crawling the database, high-speed mirroring, etc." and
//!    "Beware HTTP Error 403 … Cloudflare … To circumvent it, you must set a
//!    custom user-agent header on your HTTP request."
//!   → We are user-driven and low-volume: one request per keystroke-completed
//!     search / per chosen download, one in flight at a time, no crawling or
//!     prefetch. We send a descriptive User-Agent (see [`user_agent`]).
//!
//! IFDB Code of Conduct — Game and Link Policy (https://ifdb.org/code-of-conduct):
//!   prohibits "Automated creation of game entries without moderator permission"
//!   and "Posting spam, malware, or misleading links". We are strictly
//!   READ-ONLY (search + fetch); we never create or edit listings.
//!
//! IFDB Copyright (https://ifdb.org/copyright): user-contributed metadata is
//!   licensed CC-BY 3.0 US. "IFDB doesn't store any games - just metadata about
//!   the games". Each game's own copyright is set by its author; download links
//!   point to the IF Archive / author sites, not IFDB. → We show a "Results from
//!   IFDB" attribution line in the search modal (honours CC-BY; cheap either way).
//!
//! Conclusion: the whole feature (search + metadata + downloading a chosen file
//! via the documented APIs) is expressly a supported third-party-client use.
//! No policy language requires HTML scraping or disallows what we do.
//!
//! SQ-0474 — auto-populating a download's sidecar + cover: the
//! `viewgame?ifiction` response fetched for (1)'s download options ALREADY
//! carries the game's full iFiction record (bibliographic data plus the
//! `<ifdb><coverart><url>`, same shape [`crate::ifiction::parse`] reads for
//! IFDB's other metadata path). [`IfdbSearchClient::download_options`] parses
//! it once and threads it through [`SearchEvent::Options`] so a subsequent
//! download can populate the picker's `info.json` sidecar and, at most once,
//! its cover — ZERO extra `viewgame` requests. The cover byte fetch (if any)
//! reuses [`crate::fetch_worker::maybe_fetch_cover`]'s existing one-request
//! logic via a [`crate::ifdb::MetadataSource`] the [`SearchWorker`] holds
//! alongside its [`SearchSource`], so a download never issues more than one
//! `coverart` GET, and never one at all when the record has no `<coverart>`.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::ifdb::MetadataSource;
use crate::ifiction::IFiction;

/// Whole-call cap for the XML endpoints (search, hot, viewgame). These replies
/// are kilobytes and a slow one is a broken one, so failing fast is right — but
/// see [`BODY_TIMEOUT`] for why a *download* must not be held to it.
const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_SEARCH_XML: u64 = 4 * 1024 * 1024; // 4 MiB — a ~100-game reply is well under this.

/// Cap on one downloaded story file.
///
/// 32 MiB: comfortably past the largest game anyone ships, still far short of
/// anything a mislabelled link could usefully deliver. It was 16 MiB under a
/// sentence about story files being small, which refused *Kerkerkruip* outright
/// — see [`crate::corpus`] for what that sentence was worth and why the figures
/// live there rather than here.
///
/// The assertion below is what keeps the two honest: raise the corpus floor and
/// the build fails until this is raised with it.
pub const MAX_DOWNLOAD: u64 = 32 * 1024 * 1024;
const _: () = assert!(
    MAX_DOWNLOAD > crate::corpus::LARGEST_GAME,
    "the download cap must admit the largest game we know of, or the feature refuses real games"
);

// ── Download timeouts ────────────────────────────────────────────────────────
//
// The size cap and the time budget are ONE fact, not two, which is why they sit
// together: a cap that admits a 22 MB Glulx game paired with a timeout that
// cannot finish one is a promise the code does not keep. That is precisely what
// 16 MiB and a 15-second `timeout_global` were doing — and `timeout_global` in
// ureq 3 is documented as "end-to-end, from DNS lookup to finishing reading the
// response body", so it is a WHOLE-TRANSFER cap and not the per-read timeout it
// might be mistaken for. 22 MB inside 15s needs 1.5 MB/s sustained.
//
// ureq 3 offers no idle timer at all — every knob it has caps a phase — so a
// download is bounded per phase instead: fail fast on a host that will not
// answer, then allow a real file the time a real link needs.

/// Cap on DNS + socket + TLS. A host that will not answer must not hang.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Cap on time-to-headers. A server that accepts the connection and then says
/// nothing fails here rather than inside the body budget.
pub const HEADERS_TIMEOUT: Duration = Duration::from_secs(20);
/// Cap on reading the body. Sized from the numbers above, not from taste:
/// [`MAX_DOWNLOAD`] at roughly 110 KB/s — a genuinely poor link — is about five
/// minutes, and a transfer slower than that has stalled rather than started.
pub const BODY_TIMEOUT: Duration = Duration::from_secs(300);

/// One game listing from an IFDB search.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub tuid: String,
    pub title: String,
    pub author: Option<String>,
    /// The game's IFDB listing page (for the "open in browser" fallback).
    pub link: Option<String>,
    /// Publication year/date, as IFDB's machine form (e.g. "1990").
    pub published: Option<String>,
    /// Rounded ½-star rating, if the game has ratings.
    pub star_rating: Option<f32>,
    pub num_ratings: Option<u32>,
    pub has_cover: bool,
}

/// A single downloadable file resolved from a game's viewgame record. Only
/// entries the app can actually open are produced (see [`is_playable_download`]).
#[derive(Debug, Clone, PartialEq)]
pub struct DownloadOption {
    /// A sanitized suggested filename (from the URL basename), always ending in
    /// an accepted story extension.
    pub filename: String,
    pub url: String,
    /// IFDB's declared format (e.g. "zcode", "glulx"), for display.
    pub format: Option<String>,
    /// The link's `<title>`, flattened to one line. Usually just the filename
    /// again, but occasionally the only thing that identifies the file
    /// ("Russian translation in Z8"). See [`Self::subtitle`].
    pub title: Option<String>,
    /// The link's `<desc>`, flattened to one line — "Release 16: latest version
    /// of the game.", "Competition version". This is what tells two downloads
    /// apart, and for some games it is the *only* thing that does: IFDB lists
    /// two different Photopia builds both named `photopia.z5`.
    pub desc: Option<String>,
}

impl DownloadOption {
    /// The one-line description shown under the filename in the chooser, or
    /// `None` when IFDB told us nothing the filename doesn't already say.
    ///
    /// `<title>` is dropped when it merely repeats the filename — which is the
    /// common case (27 of the 39 playable links across a 20-game sample), so
    /// showing it unconditionally would be noise on most rows. When it does
    /// differ it is kept *and* joined to the description, because the two carry
    /// different halves of the answer ("Russian translation in Z8" +
    /// "translated by Vyacheslav Dobranov").
    pub fn subtitle(&self) -> Option<String> {
        let title = self
            .title
            .as_deref()
            .filter(|t| !t.eq_ignore_ascii_case(self.filename.as_str()));
        match (title, self.desc.as_deref()) {
            (Some(t), Some(d)) => Some(format!("{t} — {d}")),
            (Some(t), None) => Some(t.to_string()),
            (None, Some(d)) => Some(d.to_string()),
            (None, None) => None,
        }
    }
}

/// A resolved game: its playable download options plus (SQ-0474) the
/// iFiction record parsed from the SAME `viewgame?ifiction` response, for
/// populating a chosen download's metadata sidecar + cover without a second
/// request.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResolvedGame {
    pub options: Vec<DownloadOption>,
    /// Boxed: `IFiction` is large enough that an unboxed `Option` here blows
    /// up `SearchEvent`'s size (clippy's `large_enum_variant`).
    pub record: Option<Box<IFiction>>,
}

#[derive(Debug)]
pub enum SearchError {
    /// Any transport/HTTP failure (offline, DNS, 5xx, timeout, Cloudflare 403).
    Transport(String),
    /// IFDB returned a well-formed `<error>` reply.
    Remote(String),
    /// The download exceeded [`MAX_DOWNLOAD`].
    TooLarge,
    /// The URL yielded no usable, accepted-extension filename.
    NoFilename,
    /// The downloaded bytes don't look like a story of the type the filename
    /// claims (SQ-0659) — e.g. a 200 HTML landing page served for a dead link.
    /// Nothing is written to disk.
    NotAStory,
    /// A local filesystem error writing the download.
    Io(String),
}

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SearchError::Transport(_) => write!(f, "IFDB unreachable"),
            SearchError::Remote(m) => write!(f, "IFDB: {m}"),
            SearchError::TooLarge => {
                write!(f, "file exceeds {} MiB cap", MAX_DOWNLOAD / (1024 * 1024))
            }
            SearchError::NoFilename => write!(f, "no downloadable story file"),
            SearchError::NotAStory => write!(f, "downloaded file is not a valid story file"),
            SearchError::Io(m) => write!(f, "could not save file: {m}"),
        }
    }
}

/// The network surface, abstracted so the picker's worker and its tests share
/// one interface (a `Fake` in tests never touches the network).
pub trait SearchSource: Send + Sync {
    fn search(&self, query: &str) -> Result<Vec<SearchHit>, SearchError>;
    /// The "Popular on IFDB" seed list shown when the search modal opens
    /// (SQ-0473), before the user has typed a query — see the module header's
    /// "(1b)" note for the exact, live-verified request.
    fn hot(&self) -> Result<Vec<SearchHit>, SearchError>;
    /// The playable download options for a game, by its IFDB tuid, plus
    /// (SQ-0474) its iFiction record parsed from the same response.
    fn download_options(&self, tuid: &str) -> Result<ResolvedGame, SearchError>;
    /// Download `url` into `dest_dir`, returning the written path.
    fn download(&self, url: &str, dest_dir: &Path) -> Result<PathBuf, SearchError>;
}

/// ureq-backed [`SearchSource`] hitting the live IFDB + file-host endpoints.
pub struct IfdbSearchClient {
    agent: ureq::Agent,
}

impl IfdbSearchClient {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .user_agent(user_agent())
            .build();
        Self { agent: ureq::Agent::new_with_config(config) }
    }
}

impl Default for IfdbSearchClient {
    fn default() -> Self {
        Self::new()
    }
}

impl IfdbSearchClient {
    /// GET `url` and read its body, capped at [`MAX_SEARCH_XML`]. Shared by
    /// every XML endpoint this client hits (search, hot, viewgame).
    fn fetch_xml(&self, url: String) -> Result<Vec<u8>, SearchError> {
        self.agent
            .get(url)
            .call()
            .map_err(|e| SearchError::Transport(e.to_string()))?
            .body_mut()
            .with_config()
            .limit(MAX_SEARCH_XML)
            .read_to_vec()
            .map_err(|e| SearchError::Transport(e.to_string()))
    }
}

impl SearchSource for IfdbSearchClient {
    fn search(&self, query: &str) -> Result<Vec<SearchHit>, SearchError> {
        parse_search(&self.fetch_xml(search_url(query))?)
    }

    fn hot(&self) -> Result<Vec<SearchHit>, SearchError> {
        parse_search(&self.fetch_xml(hot_url())?)
    }

    fn download_options(&self, tuid: &str) -> Result<ResolvedGame, SearchError> {
        let bytes = self.fetch_xml(viewgame_ifiction_url(tuid))?;
        let options = parse_download_options(&bytes);
        // Best-effort: a record that fails to parse (or isn't iFiction at
        // all) just means no auto-populated metadata later — the download
        // itself is unaffected.
        let record = crate::ifiction::parse(&bytes).ok().map(Box::new);
        Ok(ResolvedGame { options, record })
    }

    fn download(&self, url: &str, dest_dir: &Path) -> Result<PathBuf, SearchError> {
        // Per-request, because this agent's own `timeout_global` is the XML
        // endpoints' 15 seconds and a story file is not an XML reply. Cleared
        // rather than raised: `timeout_global` covers all the others, so leaving
        // it set would re-impose a whole-transfer cap over the phase budgets.
        let mut resp = self
            .agent
            .get(url)
            .config()
            .timeout_global(None)
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_recv_response(Some(HEADERS_TIMEOUT))
            .timeout_recv_body(Some(BODY_TIMEOUT))
            .build()
            .call()
            .map_err(|e| SearchError::Transport(e.to_string()))?;

        // Fast reject on an honest Content-Length before reading a byte.
        if let Some(len) = resp
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
        {
            if len > MAX_DOWNLOAD {
                return Err(SearchError::TooLarge);
            }
        }

        // Derive the filename from Content-Disposition if the host offers one,
        // else the URL basename. Never trusted blindly — see `download_filename`.
        let disposition = resp
            .headers()
            .get("content-disposition")
            .and_then(|v| v.to_str().ok());
        let name = download_filename(disposition, url).ok_or(SearchError::NoFilename)?;

        let bytes = read_capped(resp.body_mut().as_reader(), MAX_DOWNLOAD)?;

        // SQ-0659: never write bytes that aren't plausibly a story of the type
        // the filename claims — a 200 HTML landing page must not land on disk
        // as `game.z5` (the picker's rescan would then drop it silently).
        if !looks_like_story_file(&name, &bytes) {
            return Err(SearchError::NotAStory);
        }

        std::fs::create_dir_all(dest_dir).map_err(|e| SearchError::Io(e.to_string()))?;
        let dest = unique_dest(dest_dir, &name);
        std::fs::write(&dest, &bytes).map_err(|e| SearchError::Io(e.to_string()))?;
        Ok(dest)
    }
}

/// lanthorn's descriptive User-Agent — identifies the client and its repo, and
/// (per IFDB's docs) is required to clear Cloudflare's bot filter.
fn user_agent() -> String {
    format!("lanthorn/{} (+https://github.com/sharkusk/lanthorn)", env!("CARGO_PKG_VERSION"))
}

fn search_url(query: &str) -> String {
    format!("https://ifdb.org/search?xml&game&searchfor={}", percent_encode(query))
}

/// The "Popular on IFDB" seed query (SQ-0473) — see the module header's "(1b)"
/// note for how this was chosen and verified.
fn hot_url() -> String {
    format!(
        "https://ifdb.org/search?xml&game&searchfor={}&sortby=ratu",
        percent_encode("rating:4- #ratings:10-")
    )
}

fn viewgame_ifiction_url(tuid: &str) -> String {
    format!("https://ifdb.org/viewgame?ifiction&id={}", percent_encode(tuid))
}

/// Parse an IFDB `<searchReply>` game search into hits. A `<error>` reply
/// becomes [`SearchError::Remote`]; an empty `<games>` group is zero hits, not
/// an error. Reuses the same namespace-tolerant, hand-verified discipline as
/// `ifiction.rs`.
pub fn parse_search(xml: &[u8]) -> Result<Vec<SearchHit>, SearchError> {
    let text = std::str::from_utf8(xml)
        .map_err(|_| SearchError::Remote("reply was not valid UTF-8".into()))?;
    let doc = roxmltree::Document::parse(text)
        .map_err(|e| SearchError::Remote(format!("malformed reply: {e}")))?;

    // A top-level <error> is IFDB's failure shape.
    if let Some(err) = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "error")
        .and_then(|n| n.text())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Err(SearchError::Remote(err.to_string()));
    }

    let hits = doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "game")
        .filter_map(parse_game)
        .collect();
    Ok(hits)
}

/// One `<game>` element → a `SearchHit`. Requires at least a tuid and title;
/// anything less is skipped rather than surfaced as a broken row.
fn parse_game(game: roxmltree::Node) -> Option<SearchHit> {
    let tuid = child_text(game, "tuid")?;
    let title = child_text(game, "title")?;
    let published = game
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "published")
        .and_then(|p| child_text(p, "machine"));
    let has_cover = game
        .children()
        .any(|n| n.is_element() && matches!(n.tag_name().name(), "hasCoverArt" | "coverArtLink"));
    Some(SearchHit {
        tuid,
        title,
        author: child_text(game, "author"),
        link: child_text(game, "link"),
        published,
        star_rating: child_text(game, "starRating").and_then(|s| s.parse().ok()),
        num_ratings: child_text(game, "numRatings").and_then(|s| s.parse().ok()),
        has_cover,
    })
}

/// Parse the `<downloads><links>` block of a viewgame iFiction record into the
/// playable download options (see [`is_playable_download`]). A record with no
/// downloads, or only compressed/foreign entries, yields an empty vec.
pub fn parse_download_options(xml: &[u8]) -> Vec<DownloadOption> {
    let Ok(text) = std::str::from_utf8(xml) else { return Vec::new() };
    let Ok(doc) = roxmltree::Document::parse(text) else { return Vec::new() };
    doc.descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "link")
        .filter_map(|link| {
            let url = child_text(link, "url")?;
            let compressed = link
                .children()
                .any(|n| n.is_element() && n.tag_name().name() == "compression");
            if !is_playable_download(&url, compressed) {
                return None;
            }
            let filename = basename_from_url(&url).and_then(|n| sanitize_filename(&n))?;
            Some(DownloadOption {
                filename,
                url,
                format: child_text(link, "format"),
                title: child_text(link, "title").and_then(|t| one_line(&t)),
                desc: child_text(link, "desc").and_then(|d| one_line(&d)),
            })
        })
        .collect()
}

/// Flatten an IFDB per-file `<title>`/`<desc>` to a single display line, or
/// `None` if nothing is left.
///
/// Two passes matter here. The text is double-encoded the same way a game's
/// `<description>` is (see `ifiction`), so literal HTML survives roxmltree's one
/// XML decode and has to be flattened. That flattening yields *newlines* — and
/// these strings really do carry them (IFDB's Invisiclues entry ends in a blank
/// line) — which a one-row list cell cannot show, so every whitespace run then
/// collapses to a single space.
fn one_line(s: &str) -> Option<String> {
    let flat = crate::ifiction::html_to_text(s);
    let joined = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}

/// A download link is offered iff it is uncompressed AND its URL basename has
/// an extension the app can actually open (`crate::picker::has_story_ext`:
/// `.z3`–`.z8`, `.ulx`, `.gblorb`, `.zblorb`, `.blorb`, `.blb`, `.dat`, …).
/// Compressed archives (`.zip`, `.hqx`), executables, source, and documents are
/// deliberately skipped. The reason used to be that the picker could not open
/// them; since SQ-1086 it can (`picker::has_story_ext` admits `.zip`, and
/// `hints::read_story_file` unwraps one), so the reason is now the honest
/// smaller one: an IFDB archive link is usually a BUNDLE — game, walkthrough,
/// source, feelies — and choosing which entry the player meant is a different
/// question from downloading one named file. This chooser offers named files.
/// IFDB's `<isGame/>` flag is a hint but not the gate: the extension is
/// authoritative.
pub fn is_playable_download(url: &str, compressed: bool) -> bool {
    if compressed {
        return false;
    }
    match basename_from_url(url) {
        Some(name) => is_named_story_file(Path::new(&name)),
        None => false,
    }
}

/// A story extension THIS module accepts: `picker::has_story_ext` minus the
/// archives it admits.
///
/// The two questions used to have the same answer and no longer do (SQ-1086).
/// `has_story_ext` asks "is this worth opening during a directory scan?", and a
/// `.zip` is, because the loader unwraps one. This module asks a narrower
/// question — "is this one named story file a chooser can offer and a download
/// can be validated against?" — and an archive is not: it is a bundle whose
/// contents nobody has chosen between, and `looks_like_story_file` below has no
/// extension to sniff it against. Deriving one from the other keeps them from
/// drifting apart; spelling out the difference keeps them from being confused.
fn is_named_story_file(name: &Path) -> bool {
    if name.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
        return false;
    }
    crate::picker::has_story_ext(name)
}

/// Read at most `cap` bytes from `r`; error with [`SearchError::TooLarge`] if
/// the source has more. Reads `cap + 1` so a source of exactly `cap` bytes is
/// accepted while one byte more is rejected — never buffering the whole of an
/// over-cap response.
pub fn read_capped<R: Read>(r: R, cap: u64) -> Result<Vec<u8>, SearchError> {
    let mut buf = Vec::new();
    r.take(cap + 1).read_to_end(&mut buf).map_err(|e| SearchError::Transport(e.to_string()))?;
    if buf.len() as u64 > cap {
        return Err(SearchError::TooLarge);
    }
    Ok(buf)
}

/// The last path segment of a URL, percent-decoded, with any query/fragment
/// stripped. `None` if empty. Not yet sanitized — pass through [`sanitize_filename`].
fn basename_from_url(url: &str) -> Option<String> {
    let no_frag = url.split('#').next().unwrap_or(url);
    let no_query = no_frag.split('?').next().unwrap_or(no_frag);
    let last = no_query.rsplit('/').next().unwrap_or(no_query);
    let decoded = percent_decode(last);
    (!decoded.is_empty()).then_some(decoded)
}

/// Pull a `filename` out of a `Content-Disposition` header value. Handles both
/// `filename="x"` and bare `filename=x`; the value is returned raw for
/// [`sanitize_filename`] to clean.
fn filename_from_disposition(value: &str) -> Option<String> {
    let idx = value.to_ascii_lowercase().find("filename=")?;
    let after = &value[idx + "filename=".len()..];
    let after = after.trim_start();
    let name = if let Some(rest) = after.strip_prefix('"') {
        rest.split('"').next().unwrap_or(rest)
    } else {
        after.split(';').next().unwrap_or(after).trim()
    };
    (!name.is_empty()).then(|| name.to_string())
}

/// The filename a download is saved under (SQ-0659): the Content-Disposition
/// name when it sanitizes to an openable story filename, otherwise the URL
/// basename. The fallback must apply on a *bad* disposition too, not only an
/// absent one — hosts really do send `filename=fetch.php` on links whose URL
/// basename [`is_playable_download`] already vetted, and that must not abort
/// the download.
fn download_filename(disposition: Option<&str>, url: &str) -> Option<String> {
    disposition
        .and_then(filename_from_disposition)
        .and_then(|n| sanitize_filename(&n))
        .or_else(|| basename_from_url(url).and_then(|n| sanitize_filename(&n)))
}

/// Content sniff (SQ-0659): do `bytes` plausibly hold a story of the type
/// `filename`'s extension claims? Mirrors the detection the picker's loader
/// already applies (`hints::extract_story` / `blorb::is_blorb` /
/// `scott::looks_like_scott`): a Z-code image starts with its version byte
/// (1..=8) and carries a full 64-byte header, Glulx starts with `Glul`, a
/// blorb container with `FORM`…`IFRS`; a `.dat` may be either a Scott Adams
/// text database or a Z-code image (some Infocom releases shipped as `.dat`).
pub fn looks_like_story_file(filename: &str, bytes: &[u8]) -> bool {
    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let zcode = bytes.len() >= 64 && matches!(bytes[0], 1..=8);
    match ext.as_str() {
        "z3" | "z4" | "z5" | "z6" | "z7" | "z8" => zcode,
        "ulx" => bytes.starts_with(b"Glul"),
        "gblorb" | "zblorb" | "blorb" | "blb" | "zlb" => blorb::Blorb::is_blorb(bytes),
        "dat" => zcode || std::str::from_utf8(bytes).is_ok_and(scott::looks_like_scott),
        _ => false,
    }
}

/// Reduce an untrusted filename to a safe basename: take only the final path
/// component (defeating `../` and absolute paths from either separator), drop
/// control chars and anything path-significant, and require it to still end in
/// an accepted story extension. `None` if nothing safe/openable remains.
pub fn sanitize_filename(raw: &str) -> Option<String> {
    // Final component under BOTH separators — `a/../b`, `..\\evil`, `/etc/x`.
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw).trim();
    // A lone/leading-dot name (".", "..", ".z5") is never a real story file.
    if base.is_empty() || base.starts_with('.') {
        return None;
    }
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | '\0'))
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() || cleaned == ".." {
        return None;
    }
    is_named_story_file(Path::new(&cleaned)).then_some(cleaned)
}

/// A non-colliding path in `dir` for `filename`: returns `dir/filename` if free,
/// else `dir/stem-2.ext`, `-3`, … Never overwrites an existing file.
pub fn unique_dest(dir: &Path, filename: &str) -> PathBuf {
    let first = dir.join(filename);
    if !first.exists() {
        return first;
    }
    let path = Path::new(filename);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(filename);
    let ext = path.extension().and_then(|s| s.to_str());
    for n in 2..10_000 {
        let candidate = match ext {
            Some(e) => dir.join(format!("{stem}-{n}.{e}")),
            None => dir.join(format!("{stem}-{n}")),
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    // Astronomically unlikely; fall back to the original (write may then fail).
    first
}

/// Percent-encode a query value (hand-rolled — matches `ifdb.rs`, avoids a
/// `percent-encoding` dependency for the one place we build a query string).
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

/// Decode `%xx` escapes (and `+` → space) in a URL path segment. Invalid
/// escapes are left verbatim. Bytes are decoded then re-interpreted as UTF-8,
/// falling back to the raw string if the result isn't valid UTF-8.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// Find a direct child element named `name` and return its trimmed text.
fn child_text<'a>(parent: roxmltree::Node<'a, 'a>, name: &str) -> Option<String> {
    parent
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
        .and_then(|n| n.text())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

// ── Background worker ──────────────────────────────────────────────────────
//
// The picker's search modal never blocks the UI thread on the network. A single
// long-lived worker thread serves one job at a time (search → resolve →
// download), draining results non-blocking — the same shape as `fetch_worker`
// and `cover::CoverDecoder`. One in-flight request honours IFDB's low-volume
// guidance by construction.

use std::sync::mpsc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

/// A unit of work for the [`SearchWorker`].
pub enum SearchJob {
    /// Fetch the "Popular on IFDB" seed list (SQ-0473), issued once when the
    /// search modal opens.
    Seed,
    /// Full-text game search for a typed query.
    Search(String),
    /// Resolve a chosen game's playable download options, by IFDB tuid.
    Resolve(String),
    /// Download a chosen file `url` into `dest`. `record` (SQ-0474) is the
    /// iFiction record resolved alongside the options that produced this
    /// download, if any — used to populate the sidecar + cover after the
    /// file lands, with zero extra requests. Boxed for the same reason as
    /// [`ResolvedGame::record`].
    Download { url: String, dest: PathBuf, record: Option<Box<IFiction>> },
}

/// A completed job's result, drained by the picker each frame.
pub enum SearchEvent {
    Results(Vec<SearchHit>),
    Options(ResolvedGame),
    Downloaded(PathBuf),
    /// A friendly, already-formatted error line (never a raw transport dump).
    Failed(String),
}

/// One serial background worker for the search modal. Dropping it ends the
/// thread (the request channel closes).
pub struct SearchWorker {
    req_tx: mpsc::Sender<SearchJob>,
    res_rx: mpsc::Receiver<SearchEvent>,
    busy: Arc<AtomicBool>,
    _worker: thread::JoinHandle<()>,
}

impl SearchWorker {
    /// `covers` and `data_base` are SQ-0474's post-download metadata/cover
    /// persistence: `covers` is only ever asked for a `fetch_cover` (never
    /// `fetch`/`fetch_by_id`, which would be an extra metadata request —
    /// see [`crate::ifdb::MetadataSource`]); `data_base` locates each
    /// story's `info.json` sidecar the same way the picker/fetch worker do
    /// (`crate::storage::game_dir`).
    pub fn new(source: Box<dyn SearchSource>, covers: Box<dyn MetadataSource>, data_base: PathBuf) -> Self {
        let (req_tx, req_rx) = mpsc::channel::<SearchJob>();
        let (res_tx, res_rx) = mpsc::channel::<SearchEvent>();
        let busy = Arc::new(AtomicBool::new(false));
        let worker_busy = Arc::clone(&busy);
        let worker = thread::spawn(move || {
            while let Ok(job) = req_rx.recv() {
                let event = run_job(source.as_ref(), covers.as_ref(), &data_base, job);
                worker_busy.store(false, Ordering::Relaxed);
                if res_tx.send(event).is_err() {
                    return;
                }
            }
        });
        Self { req_tx, res_rx, busy, _worker: worker }
    }

    /// Queue a job. The modal only ever has one outstanding at a time.
    pub fn request(&self, job: SearchJob) {
        self.busy.store(true, Ordering::Relaxed);
        let _ = self.req_tx.send(job);
    }

    /// Non-blocking drain of all events ready so far.
    pub fn drain(&self) -> Vec<SearchEvent> {
        self.res_rx.try_iter().collect()
    }

    pub fn busy(&self) -> bool {
        self.busy.load(Ordering::Relaxed)
    }
}

/// Run one job to an event, mapping every error to a friendly display string.
fn run_job(source: &dyn SearchSource, covers: &dyn MetadataSource, data_base: &Path, job: SearchJob) -> SearchEvent {
    match job {
        SearchJob::Seed => match source.hot() {
            Ok(hits) => SearchEvent::Results(hits),
            Err(e) => SearchEvent::Failed(e.to_string()),
        },
        SearchJob::Search(q) => match source.search(&q) {
            Ok(hits) => SearchEvent::Results(hits),
            Err(e) => SearchEvent::Failed(e.to_string()),
        },
        SearchJob::Resolve(tuid) => match source.download_options(&tuid) {
            Ok(resolved) => SearchEvent::Options(resolved),
            Err(e) => SearchEvent::Failed(e.to_string()),
        },
        SearchJob::Download { url, dest, record } => match source.download(&url, &dest) {
            Ok(path) => {
                persist_metadata_and_cover(covers, data_base, &path, record.as_deref());
                SearchEvent::Downloaded(path)
            }
            Err(e) => SearchEvent::Failed(e.to_string()),
        },
    }
}

/// SQ-0474: after a successful download, populate its `info.json` sidecar
/// (title/author/…) and, at most once, its cover — reusing the fetch
/// worker's own writers (`write_fetched`/`found_meta`/`maybe_fetch_cover`)
/// so the shape is byte-for-byte what the picker already reads. Best-effort
/// and silent: no `record` (the game had no iFiction data, or the modal
/// never resolved one) simply leaves nothing written, and the picker falls
/// back to its normal lazy `f`/`r` fetch — this never turns a successful
/// download into a failure.
fn persist_metadata_and_cover(
    covers: &dyn MetadataSource,
    data_base: &Path,
    story_path: &Path,
    record: Option<&IFiction>,
) {
    let Some(iff) = record else { return };
    // **Both halves of the key come from the row the browser is about to build
    // for this file, and from nothing else** (SQ-1226). A sidecar is only ever
    // read back at `StoryEntry::game_dir` / `meta.ifid`, and `story_info::load`
    // discards one whose recorded IFID is not the one it was asked for — so a
    // key derived any other way writes a file nobody can open, and the row shows
    // no title, author, blurb or cover at all.
    //
    // Two other derivations looked right and were not. The record's own `<ifid>`
    // is IFDB's Treaty of Babel string, which for a story with no embedded
    // marker is the file's MD5 — `crate::ifid::compute_ifid` answers a
    // `GLULX-…`/`ZCODE-…` of its own and never that, so trusting it (however
    // unambiguous IFDB's list) keys the sidecar under a string the picker cannot
    // ask for. And computing over the DOWNLOADED FILE's bytes is the container's
    // IFID, not the story's: a Blorb begins `FORM`, so a `.gblorb` whose Glulx
    // image carries no `UUID://` marker falls through to the Z-machine
    // derivation reading an IFF header as a story header. *City of Secrets* —
    // Inform 6, Glulx, served as a `.gblorb` — is both at once.
    let Some(entry) = crate::picker::resolve_entry(story_path, data_base) else { return };
    let game_dir = entry.game_dir(data_base);
    let cover = crate::fetch_worker::maybe_fetch_cover(covers, &game_dir, story_path, iff);
    crate::fetch_worker::write_fetched(
        &game_dir,
        &entry.meta.ifid,
        crate::fetch_worker::found_meta(iff, cover),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH: &[u8] = include_bytes!("../tests/fixtures/ifdb-search-games.xml");
    const ZORK: &[u8] = include_bytes!("../tests/fixtures/ifdb-zork1.xml");

    #[test]
    fn parses_a_live_search_reply() {
        let hits = parse_search(SEARCH).unwrap();
        assert_eq!(hits.len(), 3);
        let dsd = &hits[0];
        assert_eq!(dsd.tuid, "k82q3libhff6ks8l");
        assert_eq!(dsd.title, "Deep Space Drifter");
        assert_eq!(dsd.author.as_deref(), Some("Michael J. Roberts and Steve McAdams"));
        assert_eq!(dsd.published.as_deref(), Some("1990"));
        assert_eq!(dsd.star_rating, Some(2.5));
        assert_eq!(dsd.num_ratings, Some(7));
        assert!(dsd.has_cover, "hasCoverArt + coverArtLink → has_cover");
    }

    #[test]
    fn a_game_without_cover_or_ratings_parses_with_none() {
        let hits = parse_search(SEARCH).unwrap();
        // "The Initial State" — has ratings but no cover art.
        let initial = hits.iter().find(|h| h.title == "The Initial State").unwrap();
        assert!(!initial.has_cover);
        // "Eclipse" — empty <starRating></starRating>/<numRatings>0 → None/0.
        let eclipse = hits.iter().find(|h| h.title == "Eclipse").unwrap();
        assert_eq!(eclipse.star_rating, None, "an empty rating element is None, not 0.0");
        assert!(!eclipse.has_cover);
    }

    #[test]
    fn zero_results_is_empty_not_an_error() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?><searchReply xmlns="http://ifdb.org/api/xmlns"><games></games></searchReply>"#;
        let hits = parse_search(xml).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn an_error_reply_is_surfaced_as_remote() {
        let xml = br#"<?xml version="1.0"?><searchReply xmlns="http://ifdb.org/api/xmlns"><error>Bad search</error></searchReply>"#;
        match parse_search(xml) {
            Err(SearchError::Remote(m)) => assert_eq!(m, "Bad search"),
            other => panic!("expected Remote error, got {other:?}"),
        }
    }

    #[test]
    fn malformed_reply_never_panics() {
        assert!(parse_search(b"<searchReply>").is_err());
        assert!(parse_search(b"").is_err());
        assert!(parse_search(b"\xff\xfe garbage").is_err());
    }

    #[test]
    fn download_options_keep_only_uncompressed_openable_files() {
        // Zork's fixture has many links: exactly one is a raw, uncompressed
        // `.z5` story file; the rest are .zip/.hqx/.html/etc.
        let opts = parse_download_options(ZORK);
        assert_eq!(opts.len(), 1, "only the raw .z5 is offered: {opts:?}");
        let o = &opts[0];
        assert_eq!(o.url, "https://eblong.com/infocom/gamefiles/zork1-invclues-r52-s871125.z5");
        assert_eq!(o.filename, "zork1-invclues-r52-s871125.z5");
        assert_eq!(o.format.as_deref(), Some("zcode"));
        // SQ-0597: the per-file title/desc IFDB really sends for this link.
        assert_eq!(o.title.as_deref(), Some("Zork 1"));
        assert_eq!(o.desc.as_deref(), Some("Solid Gold Revision 52"));
    }

    /// SQ-0597: which of `<title>`/`<desc>` actually reaches the chooser row.
    #[test]
    fn subtitle_drops_a_title_that_only_repeats_the_filename() {
        let base = DownloadOption {
            filename: "curses.z5".into(),
            url: "https://x/curses.z5".into(),
            format: Some("zcode".into()),
            title: None,
            desc: None,
        };

        // The common shape: IFDB's <title> IS the filename, so only the
        // description says anything (Curses ships 7 files that differ this way).
        let o = DownloadOption {
            title: Some("curses.z5".into()),
            desc: Some("Release 16: latest version of the game.".into()),
            ..base.clone()
        };
        assert_eq!(o.subtitle().as_deref(), Some("Release 16: latest version of the game."));

        // A title that says something the filename doesn't is kept AND joined —
        // Photopia's Russian build splits the answer across both fields.
        let o = DownloadOption {
            filename: "PhotopiaR.z8".into(),
            title: Some("Russian translation in Z8".into()),
            desc: Some("translated by Vyacheslav Dobranov".into()),
            ..base.clone()
        };
        assert_eq!(
            o.subtitle().as_deref(),
            Some("Russian translation in Z8 — translated by Vyacheslav Dobranov")
        );

        // Either alone.
        let o = DownloadOption { title: Some("Competition version".into()), ..base.clone() };
        assert_eq!(o.subtitle().as_deref(), Some("Competition version"));
        let o = DownloadOption { desc: Some("Photopia v1.30".into()), ..base.clone() };
        assert_eq!(o.subtitle().as_deref(), Some("Photopia v1.30"));

        // Nothing worth a second row.
        assert_eq!(base.subtitle(), None);
        let o = DownloadOption { title: Some("CURSES.Z5".into()), ..base };
        assert_eq!(o.subtitle(), None, "a case-different filename echo is still an echo");
    }

    /// The text is double-encoded HTML inside XML and really does carry
    /// newlines (IFDB's Invisiclues entry ends in a blank line), but a list row
    /// is one line high.
    #[test]
    fn one_line_flattens_markup_and_newlines_to_a_single_row() {
        assert_eq!(one_line("Release 7").as_deref(), Some("Release 7"));
        assert_eq!(
            one_line("Hints for &quot;Zork I&quot;,<br>adapted\n\nwith permission.\n\n").as_deref(),
            Some("Hints for \"Zork I\", adapted with permission."),
        );
        assert_eq!(one_line("   ").as_deref(), None, "whitespace-only is nothing to show");
        assert_eq!(one_line("<br/>").as_deref(), None, "markup-only is nothing to show");
    }

    #[test]
    fn is_playable_download_filters_by_extension_and_compression() {
        assert!(is_playable_download("https://x/y/game.z5", false));
        assert!(is_playable_download("https://x/game.gblorb", false));
        assert!(is_playable_download("https://x/game.ulx?dl=1", false));
        // Compressed → never, even with a story extension inside the name.
        assert!(!is_playable_download("https://x/game.z5", true));
        assert!(!is_playable_download("https://x/zorkvms.zip", false));
        assert!(!is_playable_download("https://x/game.exe", false));
        assert!(!is_playable_download("https://x/readme.txt", false));
    }

    #[test]
    fn sanitize_filename_defeats_path_traversal() {
        assert_eq!(sanitize_filename("game.z5").as_deref(), Some("game.z5"));
        // Path components stripped to the basename.
        assert_eq!(sanitize_filename("../../etc/evil.z5").as_deref(), Some("evil.z5"));
        assert_eq!(sanitize_filename("/abs/path/story.ulx").as_deref(), Some("story.ulx"));
        assert_eq!(sanitize_filename(r"..\\windows\\thing.gblorb").as_deref(), Some("thing.gblorb"));
        // No accepted extension → rejected.
        assert_eq!(sanitize_filename("payload.exe"), None);
        assert_eq!(sanitize_filename("archive.zip"), None);
        // Dotfiles / traversal atoms → rejected.
        assert_eq!(sanitize_filename(".."), None);
        assert_eq!(sanitize_filename(".z5"), None);
        assert_eq!(sanitize_filename(""), None);
    }

    #[test]
    fn basename_from_url_strips_query_and_decodes() {
        assert_eq!(basename_from_url("https://x/a/b/c.z5").as_deref(), Some("c.z5"));
        assert_eq!(basename_from_url("https://x/c.z5?ver=2#frag").as_deref(), Some("c.z5"));
        assert_eq!(basename_from_url("https://x/My%20Game.z5").as_deref(), Some("My Game.z5"));
        assert_eq!(basename_from_url("https://x/").as_deref(), None);
    }

    /// SQ-0659: the disposition name is preferred only when it sanitizes; a
    /// disposition like `filename=fetch.php` must fall back to the URL
    /// basename (which `is_playable_download` already vetted), in that order.
    #[test]
    fn download_filename_tries_disposition_then_falls_back_to_url_basename() {
        // A sane disposition wins over the URL basename.
        assert_eq!(
            download_filename(Some(r#"attachment; filename="renamed.z8""#), "https://x/orig.z5")
                .as_deref(),
            Some("renamed.z8")
        );
        // A disposition that fails to sanitize falls back to the URL basename
        // instead of aborting the download.
        assert_eq!(
            download_filename(Some("attachment; filename=fetch.php"), "https://x/orig.z5")
                .as_deref(),
            Some("orig.z5")
        );
        // No disposition at all: URL basename.
        assert_eq!(download_filename(None, "https://x/orig.z5").as_deref(), Some("orig.z5"));
        // Neither source usable: give up.
        assert_eq!(download_filename(Some("attachment; filename=fetch.php"), "https://x/fetch.php"), None);
        assert_eq!(download_filename(None, "https://x/"), None);
    }

    /// SQ-0659: a 200 HTML landing page must never be written as a story file,
    /// while real magic for each accepted extension passes.
    #[test]
    fn looks_like_story_file_rejects_html_and_accepts_real_magic() {
        let mut html = b"<!DOCTYPE html><html><body>file not found</body></html>".to_vec();
        html.resize(1024, b' '); // plenty long — length alone must not pass it

        // Z-code: version byte + full header.
        let mut z = vec![0u8; 64];
        z[0] = 5;
        assert!(looks_like_story_file("game.z5", &z));
        assert!(!looks_like_story_file("game.z5", &html));
        assert!(!looks_like_story_file("game.z5", &z[..32]), "truncated header");

        // Glulx: `Glul` magic.
        assert!(looks_like_story_file("game.ulx", b"Glul\x00\x03\x01\x02"));
        assert!(!looks_like_story_file("game.ulx", &html));

        // Blorbs: FORM…IFRS.
        let mut blorb_bytes = b"FORM".to_vec();
        blorb_bytes.extend_from_slice(&4u32.to_be_bytes());
        blorb_bytes.extend_from_slice(b"IFRS");
        for ext in ["gblorb", "zblorb", "blorb", "blb", "zlb"] {
            assert!(looks_like_story_file(&format!("g.{ext}"), &blorb_bytes), ".{ext}");
            assert!(!looks_like_story_file(&format!("g.{ext}"), &html), ".{ext} html");
        }

        // .dat: a Scott Adams text database (12 sane header ints) or Z-code.
        let scott = "32767 1 0 1 2 6 1 0 3 125 0 1\n";
        assert!(looks_like_story_file("adv01.dat", scott.as_bytes()));
        assert!(looks_like_story_file("zork.dat", &z), "Infocom .dat releases are Z-code");
        assert!(!looks_like_story_file("adv01.dat", &html));
    }

    #[test]
    fn filename_from_disposition_quoted_and_bare() {
        assert_eq!(
            filename_from_disposition(r#"attachment; filename="zork.z5""#).as_deref(),
            Some("zork.z5")
        );
        assert_eq!(
            filename_from_disposition("inline; filename=game.ulx").as_deref(),
            Some("game.ulx")
        );
        assert_eq!(filename_from_disposition("attachment").as_deref(), None);
    }

    #[test]
    fn read_capped_accepts_up_to_cap_and_rejects_beyond() {
        // Exactly cap: accepted.
        let ok = read_capped(&[7u8; 8][..], 8).unwrap();
        assert_eq!(ok.len(), 8);
        // cap + 1: rejected as TooLarge.
        match read_capped(&[7u8; 9][..], 8) {
            Err(SearchError::TooLarge) => {}
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    #[test]
    fn unique_dest_appends_a_suffix_on_collision() {
        let dir = std::env::temp_dir().join(format!("bm_ifdb_dl_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = unique_dest(&dir, "game.z5");
        assert_eq!(a.file_name().unwrap(), "game.z5");
        std::fs::write(&a, b"x").unwrap();
        let b = unique_dest(&dir, "game.z5");
        assert_eq!(b.file_name().unwrap(), "game-2.z5", "collision → -2 suffix");
        std::fs::write(&b, b"x").unwrap();
        let c = unique_dest(&dir, "game.z5");
        assert_eq!(c.file_name().unwrap(), "game-3.z5");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_url_encodes_the_query_and_targets_the_xml_game_endpoint() {
        assert_eq!(
            search_url("Deep Space Drifter"),
            "https://ifdb.org/search?xml&game&searchfor=Deep%20Space%20Drifter"
        );
        assert!(search_url("A&B").ends_with("searchfor=A%26B"));
    }

    #[test]
    fn hot_url_uses_the_verified_rating_floor_and_browse_sort() {
        assert_eq!(
            hot_url(),
            "https://ifdb.org/search?xml&game&searchfor=rating%3A4-%20%23ratings%3A10-&sortby=ratu"
        );
    }

    #[test]
    fn user_agent_identifies_lanthorn() {
        let ua = user_agent();
        assert!(ua.starts_with("lanthorn/"));
        assert!(ua.contains("github.com/sharkusk/lanthorn"));
    }

    // ── worker roundtrip (no network) ──────────────────────────────────────

    use std::sync::Mutex;
    use crate::ifdb::{FetchError, FetchOutcome};

    struct Fake {
        hits: Vec<SearchHit>,
        options: Vec<DownloadOption>,
        record: Option<IFiction>,
        fail: bool,
    }

    impl SearchSource for Fake {
        fn search(&self, _q: &str) -> Result<Vec<SearchHit>, SearchError> {
            if self.fail {
                return Err(SearchError::Transport("offline".into()));
            }
            Ok(self.hits.clone())
        }
        fn hot(&self) -> Result<Vec<SearchHit>, SearchError> {
            if self.fail {
                return Err(SearchError::Transport("offline".into()));
            }
            Ok(self.hits.clone())
        }
        fn download_options(&self, _tuid: &str) -> Result<ResolvedGame, SearchError> {
            Ok(ResolvedGame { options: self.options.clone(), record: self.record.clone().map(Box::new) })
        }
        fn download(&self, _url: &str, dest: &Path) -> Result<PathBuf, SearchError> {
            let p = dest.join("downloaded.ulx");
            std::fs::create_dir_all(dest).map_err(|e| SearchError::Io(e.to_string()))?;
            // A real, launchable image rather than five bytes of prose: the
            // sidecar is written under the IFID and directory the PICKER
            // resolves for the landed file, so a file no loader accepts gets
            // no row and no sidecar at all (SQ-1226).
            std::fs::write(&p, glulx_image()).map_err(|e| SearchError::Io(e.to_string()))?;
            Ok(p)
        }
    }

    /// A fake [`MetadataSource`] used ONLY for its `fetch_cover` — SQ-0474's
    /// terms posture means a download must never call `fetch`/`fetch_by_id`
    /// on it (that would be a second `viewgame` metadata request).
    struct FakeCovers {
        calls: Arc<Mutex<Vec<String>>>,
        bytes: Vec<u8>,
    }

    impl FakeCovers {
        fn new() -> Self {
            // A real (decodable) PNG: `maybe_fetch_cover` validates the bytes
            // decode before writing (SQ-0660), so raw junk would be dropped.
            let img = image::RgbImage::from_pixel(2, 2, image::Rgb([9, 9, 9]));
            let mut bytes = Vec::new();
            image::DynamicImage::ImageRgb8(img)
                .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
                .unwrap();
            FakeCovers { calls: Arc::new(Mutex::new(Vec::new())), bytes }
        }
    }

    impl MetadataSource for FakeCovers {
        fn fetch(&self, _ifid: &str) -> Result<FetchOutcome, FetchError> {
            unreachable!("SQ-0474: a download must never issue an extra metadata request")
        }
        fn fetch_by_id(&self, _tuid: &str) -> Result<FetchOutcome, FetchError> {
            unreachable!("SQ-0474: a download must never issue an extra metadata request")
        }
        fn fetch_cover(&self, url: &str) -> Result<Vec<u8>, FetchError> {
            self.calls.lock().unwrap().push(url.to_string());
            Ok(self.bytes.clone())
        }
    }

    fn drain_one(w: &SearchWorker) -> SearchEvent {
        for _ in 0..2000 {
            let mut evs = w.drain();
            if !evs.is_empty() {
                return evs.remove(0);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("no event arrived");
    }

    #[test]
    fn worker_runs_search_resolve_download_serially() {
        let hit = SearchHit {
            tuid: "abc".into(),
            title: "Test".into(),
            author: None,
            link: None,
            published: None,
            star_rating: None,
            num_ratings: None,
            has_cover: false,
        };
        let opt = DownloadOption {
            filename: "test.z5".into(),
            url: "https://x/test.z5".into(),
            format: Some("zcode".into()),
            title: None,
            desc: None,
        };
        let source = Fake { hits: vec![hit.clone()], options: vec![opt], record: None, fail: false };
        let data_base = std::env::temp_dir().join(format!("bm_ifdb_worker_data_{}", std::process::id()));
        let w = SearchWorker::new(Box::new(source), Box::new(FakeCovers::new()), data_base.clone());

        w.request(SearchJob::Search("test".into()));
        match drain_one(&w) {
            SearchEvent::Results(hits) => assert_eq!(hits, vec![hit]),
            _ => panic!("expected Results"),
        }

        w.request(SearchJob::Resolve("abc".into()));
        match drain_one(&w) {
            SearchEvent::Options(o) => assert_eq!(o.options.len(), 1),
            _ => panic!("expected Options"),
        }

        let dir = std::env::temp_dir().join(format!("bm_ifdb_worker_{}", std::process::id()));
        w.request(SearchJob::Download { url: "https://x/test.z5".into(), dest: dir.clone(), record: None });
        match drain_one(&w) {
            SearchEvent::Downloaded(p) => assert!(p.exists()),
            _ => panic!("expected Downloaded"),
        }
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&data_base);
    }

    /// Live end-to-end smoke against the real IFDB (network; `--ignored` only).
    /// Documents the contract verified during development: a title search
    /// returns hits, and a hit resolves to at least one playable download.
    #[test]
    #[ignore = "hits the live IFDB network"]
    fn live_search_and_resolve_smoke() {
        let client = IfdbSearchClient::new();
        let hits = client.search("Zork I").expect("search");
        assert!(!hits.is_empty(), "expected some hits for 'Zork I'");
        let zork = hits.iter().find(|h| h.title.starts_with("Zork")).expect("a Zork hit");
        let resolved = client.download_options(&zork.tuid).expect("resolve");
        // Zork I has a raw .z5 on eblong.com; there should be a playable file.
        assert!(
            !resolved.options.is_empty(),
            "expected at least one playable download for {}",
            zork.title
        );
    }

    /// Live smoke for the seed query (network; `--ignored` only): every
    /// returned hit should meet the documented `rating:4- #ratings:10-` floor.
    #[test]
    #[ignore = "hits the live IFDB network"]
    fn live_hot_smoke() {
        let client = IfdbSearchClient::new();
        let hits = client.hot().expect("hot");
        assert!(!hits.is_empty(), "expected some popular games");
        for h in &hits {
            assert!(h.star_rating.unwrap_or(0.0) >= 4.0, "seed row below the rating floor: {h:?}");
            assert!(h.num_ratings.unwrap_or(0) >= 10, "seed row below the ratings-count floor: {h:?}");
        }
    }

    #[test]
    fn worker_maps_a_transport_error_to_a_friendly_failed_event() {
        let source = Fake { hits: vec![], options: vec![], record: None, fail: true };
        let data_base = std::env::temp_dir().join(format!("bm_ifdb_worker_fail_{}", std::process::id()));
        let w = SearchWorker::new(Box::new(source), Box::new(FakeCovers::new()), data_base);
        w.request(SearchJob::Search("x".into()));
        match drain_one(&w) {
            SearchEvent::Failed(msg) => assert_eq!(msg, "IFDB unreachable"),
            _ => panic!("expected Failed"),
        }
    }

    // ── SQ-1226: the sidecar must be keyed the way the PICKER keys it ───────

    /// A minimal launchable Glulx image carrying **no** Inform `UUID://`
    /// marker — the shape of an Inform 6 Glulx game (`crate::ifid`'s module
    /// docs name *Narcolepsy*; *City of Secrets* is another), which is what
    /// makes its IFID a computed `GLULX-…` hash rather than anything IFDB
    /// indexes.
    fn glulx_image() -> Vec<u8> {
        let mut v = vec![0u8; 256];
        v[0..4].copy_from_slice(b"Glul");
        v[4..8].copy_from_slice(&0x0003_0102u32.to_be_bytes()); // version 3.1.2
        v[8..12].copy_from_slice(&256u32.to_be_bytes()); // RAMSTART
        v[12..16].copy_from_slice(&256u32.to_be_bytes()); // EXTSTART
        v[16..20].copy_from_slice(&256u32.to_be_bytes()); // ENDMEM
        v[20..24].copy_from_slice(&256u32.to_be_bytes()); // stack size
        v
    }

    /// `exec` wrapped in a one-resource Blorb — the `.gblorb` an IFDB download
    /// link actually serves, rather than the bare image.
    fn glulx_blorb(exec: &[u8]) -> Vec<u8> {
        let mut chunk = Vec::new();
        chunk.extend_from_slice(b"GLUL");
        chunk.extend_from_slice(&(exec.len() as u32).to_be_bytes());
        chunk.extend_from_slice(exec);
        if exec.len() % 2 == 1 {
            chunk.push(0);
        }
        // The one resource starts past the FORM header (12), the RIdx chunk
        // header (8) and the RIdx data (4 + 12) — 36 bytes in.
        let mut body = Vec::new();
        body.extend_from_slice(b"RIdx");
        body.extend_from_slice(&16u32.to_be_bytes());
        body.extend_from_slice(&1u32.to_be_bytes());
        body.extend_from_slice(b"Exec");
        body.extend_from_slice(&0u32.to_be_bytes());
        body.extend_from_slice(&36u32.to_be_bytes());
        body.extend_from_slice(&chunk);
        let mut out = Vec::new();
        out.extend_from_slice(b"FORM");
        out.extend_from_slice(&((body.len() + 4) as u32).to_be_bytes());
        out.extend_from_slice(b"IFRS");
        out.extend_from_slice(&body);
        out
    }

    /// A source whose `download` writes **real, launchable** story bytes under
    /// a real name. [`Fake`] writes five bytes called `downloaded.z5`, which no
    /// loader accepts and no picker row is ever built from — so nothing that
    /// depends on the downloaded file actually resolving can be asked of it.
    struct FakeStoryDownload {
        name: String,
        bytes: Vec<u8>,
    }

    impl SearchSource for FakeStoryDownload {
        fn search(&self, _q: &str) -> Result<Vec<SearchHit>, SearchError> {
            Ok(Vec::new())
        }
        fn hot(&self) -> Result<Vec<SearchHit>, SearchError> {
            Ok(Vec::new())
        }
        fn download_options(&self, _tuid: &str) -> Result<ResolvedGame, SearchError> {
            Ok(ResolvedGame { options: Vec::new(), record: None })
        }
        fn download(&self, _url: &str, dest: &Path) -> Result<PathBuf, SearchError> {
            std::fs::create_dir_all(dest).map_err(|e| SearchError::Io(e.to_string()))?;
            let p = dest.join(&self.name);
            std::fs::write(&p, &self.bytes).map_err(|e| SearchError::Io(e.to_string()))?;
            Ok(p)
        }
    }

    /// SQ-1226 — *City of Secrets*: an Inform 6 Glulx game served as a
    /// `.gblorb`, whose IFDB record names one `<ifid>` that is not the string
    /// [`crate::ifid::compute_ifid`] produces.
    ///
    /// The sidecar is only ever READ back at the row's `meta.ifid` — the IFID
    /// the picker computes over the story it EXTRACTED from the container — so
    /// that is the only key a write may use. Keying it on the container's own
    /// bytes (a Blorb starts `FORM`, so the Z-machine fallback reads an IFF
    /// header as a story header) or on IFDB's declared IFID both produce a
    /// sidecar `story_info::load`'s identity check discards: the download
    /// succeeds, the file lands, and the row shows no metadata at all.
    #[test]
    fn a_downloaded_blorb_is_keyed_by_the_ifid_the_picker_will_look_it_up_under() {
        let data_base =
            std::env::temp_dir().join(format!("bm_ifdb_blorb_key_{}", std::process::id()));
        let dest = data_base.join("stories");
        let source =
            FakeStoryDownload { name: "CoS.gblorb".into(), bytes: glulx_blorb(&glulx_image()) };
        let w = SearchWorker::new(Box::new(source), Box::new(FakeCovers::new()), data_base.clone());

        let record = IFiction {
            title: Some("City of Secrets".into()),
            author: Some("Emily Short".into()),
            // IFDB carries the Treaty of Babel IFID for a game with no embedded
            // marker (the file's MD5), which `crate::ifid` never produces.
            ifids: vec!["3C5A9A2E9E0F4B0E9E7F1D2C3B4A5968".into()],
            ..Default::default()
        };
        w.request(SearchJob::Download {
            url: "https://x/CoS.gblorb".into(),
            dest: dest.clone(),
            record: Some(Box::new(record)),
        });
        let path = match drain_one(&w) {
            SearchEvent::Downloaded(p) => p,
            _ => panic!("expected Downloaded"),
        };

        // The row the browser builds for the file that just landed.
        let entry = crate::picker::resolve_entry(&path, &data_base)
            .expect("the downloaded blorb scans as a playable story");
        assert!(
            entry.meta.ifid.starts_with("GLULX-"),
            "non-vacuity: the row's IFID is the computed Glulx one, neither the \
             container's nor IFDB's — got {}",
            entry.meta.ifid
        );

        let fetched = crate::story_info::load(&entry.game_dir(&data_base), &entry.meta.ifid)
            .and_then(|i| i.fetched)
            .expect("the row's own IFID finds the sidecar the download wrote");
        assert_eq!(fetched.title.as_deref(), Some("City of Secrets"));
        assert_eq!(fetched.author.as_deref(), Some("Emily Short"));

        let _ = std::fs::remove_dir_all(&data_base);
    }

    // ── SQ-0474: auto-populate the sidecar + cover after a download ─────────

    /// A record carrying both metadata and a `<coverart>` URL: the sidecar
    /// picks up title/author, the cover is fetched exactly once (never a
    /// second `viewgame` request — the fake would panic on `fetch`), and the
    /// cover bytes land beside the sidecar as `cover.png`.
    #[test]
    fn a_download_with_metadata_and_coverart_writes_the_sidecar_and_fetches_the_cover_once() {
        let data_base = std::env::temp_dir().join(format!("bm_ifdb_meta_cover_{}", std::process::id()));
        let dest = data_base.join("stories");
        let covers = FakeCovers::new();
        let cover_calls = Arc::clone(&covers.calls);
        let source = Fake { hits: vec![], options: vec![], record: None, fail: false };
        let w = SearchWorker::new(Box::new(source), Box::new(covers), data_base.clone());

        let record = IFiction {
            title: Some("Deep Space Drifter".into()),
            author: Some("Michael J. Roberts".into()),
            ifdb: Some(crate::ifiction::IfdbExt {
                tuid: "k82q3libhff6ks8l".into(),
                link: None,
                cover_url: Some("https://ifdb.org/coverart?id=k82q3libhff6ks8l&version=1".into()),
                average_rating: None,
                rating_count: None,
            }),
            ..Default::default()
        };
        w.request(SearchJob::Download {
            url: "https://x/game.z5".into(),
            dest: dest.clone(),
            record: Some(Box::new(record)),
        });
        let path = match drain_one(&w) {
            SearchEvent::Downloaded(p) => p,
            _ => panic!("expected Downloaded"),
        };

        let ifid = crate::ifid::compute_ifid(&std::fs::read(&path).unwrap());
        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key_at(&path));
        let info = crate::story_info::load(&game_dir, &ifid).expect("sidecar written");
        let fetched = info.fetched.expect("a fetched block was written");
        assert_eq!(fetched.title.as_deref(), Some("Deep Space Drifter"));
        assert_eq!(fetched.author.as_deref(), Some("Michael J. Roberts"));
        assert_eq!(fetched.cover.as_deref(), Some("cover.png"));
        assert!(game_dir.join("cover.png").exists());
        assert_eq!(
            cover_calls.lock().unwrap().as_slice(),
            &["https://ifdb.org/coverart?id=k82q3libhff6ks8l&version=1".to_string()],
            "exactly one cover request, at the record's own URL"
        );

        let _ = std::fs::remove_dir_all(&data_base);
    }

    /// A record with metadata but no `<coverart>`: the sidecar still picks
    /// up title/author, but no cover request is ever attempted.
    #[test]
    fn a_download_with_metadata_but_no_coverart_writes_metadata_only() {
        let data_base = std::env::temp_dir().join(format!("bm_ifdb_meta_only_{}", std::process::id()));
        let dest = data_base.join("stories");
        let covers = FakeCovers::new();
        let cover_calls = Arc::clone(&covers.calls);
        let source = Fake { hits: vec![], options: vec![], record: None, fail: false };
        let w = SearchWorker::new(Box::new(source), Box::new(covers), data_base.clone());

        let record = IFiction { title: Some("No Cover Game".into()), ..Default::default() };
        w.request(SearchJob::Download {
            url: "https://x/game.z5".into(),
            dest: dest.clone(),
            record: Some(Box::new(record)),
        });
        let path = match drain_one(&w) {
            SearchEvent::Downloaded(p) => p,
            _ => panic!("expected Downloaded"),
        };

        let ifid = crate::ifid::compute_ifid(&std::fs::read(&path).unwrap());
        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key_at(&path));
        let fetched =
            crate::story_info::load(&game_dir, &ifid).and_then(|i| i.fetched).expect("sidecar written");
        assert_eq!(fetched.title.as_deref(), Some("No Cover Game"));
        assert!(fetched.cover.is_none());
        assert!(!game_dir.join("cover.png").exists());
        assert!(cover_calls.lock().unwrap().is_empty(), "no <coverart> → zero cover requests");

        let _ = std::fs::remove_dir_all(&data_base);
    }

    /// IFDB groups several editions under one game page (see the module
    /// header's "(2)" note) — an ambiguous (here: more than one) `<ifid>`
    /// list can't be trusted to name this exact downloaded file, so the
    /// sidecar falls back to the same IFID `resolve_entry` computes from the
    /// bytes, which is what makes the write visible to the picker at all.
    #[test]
    fn a_download_with_an_ambiguous_record_ifid_falls_back_to_the_computed_one() {
        let data_base = std::env::temp_dir().join(format!("bm_ifdb_ifid_fallback_{}", std::process::id()));
        let dest = data_base.join("stories");
        let source = Fake { hits: vec![], options: vec![], record: None, fail: false };
        let w = SearchWorker::new(Box::new(source), Box::new(FakeCovers::new()), data_base.clone());

        let record = IFiction {
            title: Some("Grouped Editions".into()),
            ifids: vec!["ZCODE-1-000001".into(), "ZCODE-2-000002".into()],
            ..Default::default()
        };
        w.request(SearchJob::Download {
            url: "https://x/game.z5".into(),
            dest: dest.clone(),
            record: Some(Box::new(record)),
        });
        let path = match drain_one(&w) {
            SearchEvent::Downloaded(p) => p,
            _ => panic!("expected Downloaded"),
        };

        let computed = crate::ifid::compute_ifid(&std::fs::read(&path).unwrap());
        assert!(!["ZCODE-1-000001", "ZCODE-2-000002"].contains(&computed.as_str()));
        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key_at(&path));
        assert!(
            crate::story_info::load(&game_dir, &computed).and_then(|i| i.fetched).is_some(),
            "sidecar keyed by the computed IFID, which is what resolve_entry will look up"
        );
        assert!(
            crate::story_info::load(&game_dir, "ZCODE-1-000001").is_none(),
            "neither ambiguous record IFID was used as the key"
        );

        let _ = std::fs::remove_dir_all(&data_base);
    }

    /// A record naming exactly ONE edition is not trusted either (SQ-1226).
    /// The number of `<ifid>`s IFDB lists was never the question: the sidecar
    /// is read back at the IFID the picker computes, so a key IFDB declared
    /// and we do not compute is unreachable however unambiguous it is. That is
    /// every game whose Treaty of Babel IFID is not the string
    /// [`crate::ifid::compute_ifid`] produces — an Inform 6 Glulx game's is its
    /// file's MD5, ours a `GLULX-…` hash, and they never coincide.
    #[test]
    fn a_download_with_exactly_one_record_ifid_still_keys_on_the_computed_one() {
        let data_base = std::env::temp_dir().join(format!("bm_ifdb_ifid_single_{}", std::process::id()));
        let dest = data_base.join("stories");
        let source = Fake { hits: vec![], options: vec![], record: None, fail: false };
        let w = SearchWorker::new(Box::new(source), Box::new(FakeCovers::new()), data_base.clone());

        let record = IFiction {
            title: Some("Single Edition".into()),
            ifids: vec!["ZCODE-9-999999-ABCD".into()],
            ..Default::default()
        };
        w.request(SearchJob::Download {
            url: "https://x/game.z5".into(),
            dest: dest.clone(),
            record: Some(Box::new(record)),
        });
        let path = match drain_one(&w) {
            SearchEvent::Downloaded(p) => p,
            _ => panic!("expected Downloaded"),
        };

        let computed = crate::ifid::compute_ifid(&std::fs::read(&path).unwrap());
        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key_at(&path));
        assert!(
            crate::story_info::load(&game_dir, &computed).and_then(|i| i.fetched).is_some(),
            "keyed by the IFID the picker computes, which is the only one it reads back"
        );
        assert!(
            crate::story_info::load(&game_dir, "ZCODE-9-999999-ABCD").is_none(),
            "the record's declared IFID is not the key"
        );

        let _ = std::fs::remove_dir_all(&data_base);
    }

    /// No resolved record at all (e.g. the game had no iFiction data): the
    /// download still succeeds, but nothing is written — no crash, no stray
    /// sidecar, no cover request.
    #[test]
    fn a_download_with_no_record_writes_nothing() {
        let data_base = std::env::temp_dir().join(format!("bm_ifdb_no_record_{}", std::process::id()));
        let dest = data_base.join("stories");
        let covers = FakeCovers::new();
        let cover_calls = Arc::clone(&covers.calls);
        let source = Fake { hits: vec![], options: vec![], record: None, fail: false };
        let w = SearchWorker::new(Box::new(source), Box::new(covers), data_base.clone());

        w.request(SearchJob::Download { url: "https://x/game.z5".into(), dest: dest.clone(), record: None });
        let path = match drain_one(&w) {
            SearchEvent::Downloaded(p) => p,
            _ => panic!("expected Downloaded"),
        };

        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key_at(&path));
        assert!(!game_dir.join("info.json").exists());
        assert!(cover_calls.lock().unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&data_base);
    }
}
