//! Treaty of Babel iFiction metadata.
//!
//! One parser for two sources: a blorb's `IFmd` chunk and IFDB's
//! `viewgame?ifiction` response are the same format. IFDB additionally carries an
//! `<ifdb>` extension element in its own namespace, which an IFmd chunk lacks.

const BABEL_NS: &str = "http://babel.ifarchive.org/protocol/iFiction/";
const IFDB_NS: &str = "http://ifdb.org/api/xmlns";

/// Parsed iFiction. Every field is optional: an IFmd chunk may carry only a
/// subset, and thin metadata must never fail a scan.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IFiction {
    pub ifids: Vec<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub language: Option<String>,
    pub first_published: Option<String>,
    pub genre: Option<String>,
    pub description: Option<String>,
    /// From the ifdb.org extension namespace; absent in an IFmd chunk.
    pub ifdb: Option<IfdbExt>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IfdbExt {
    pub tuid: String,
    pub link: Option<String>,
    pub cover_url: Option<String>,
    /// IFDB's community average rating on a 1–5 scale (`<averageRating>`).
    /// `None` when the game has no ratings — IFDB simply omits the element, and
    /// "unrated" must never collapse into a real `0.0`.
    pub average_rating: Option<f32>,
    /// How many ratings that average is over (`<ratingCountAvg>`). Not
    /// displayed, but it breaks ties in the picker's rating sort so two 4.6s
    /// don't order arbitrarily.
    pub rating_count: Option<u32>,
}

#[derive(Debug)]
pub enum IFictionError {
    Xml(roxmltree::Error),
    /// Well-formed XML that is not an iFiction document.
    NotIFiction,
}

/// Flatten HTML fragment text (an IFDB description) to plain text: `<br>` and
/// block tags become newlines, other tags are dropped, and HTML entities are
/// decoded. Deliberately small — it handles the tags and entities IFDB actually
/// emits, not a general HTML parser, so it stays dependency-free.
///
/// Shared with `ifdb_search`, whose per-file `<title>`/`<desc>` text is
/// double-encoded exactly the same way (see the `description` call below).
pub(crate) fn html_to_text(s: &str) -> String {
    // 1. Tags → newline for line/paragraph breaks, empty for the rest.
    let mut no_tags = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            let mut tag = String::new();
            for t in chars.by_ref() {
                if t == '>' {
                    break;
                }
                tag.push(t);
            }
            let name: String = tag
                .trim_start_matches('/')
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '/')
                .collect::<String>()
                .to_ascii_lowercase();
            if matches!(name.as_str(), "br" | "p" | "div" | "li") {
                no_tags.push('\n');
            }
        } else {
            no_tags.push(c);
        }
    }
    // 2. Decode the entities IFDB uses. `&amp;` LAST so a literal "&amp;lt;"
    //    can't be turned into a "<".
    let decoded = decode_entities(&no_tags);
    // 3. Collapse the runs of blank lines the tag pass can leave, and trim.
    let mut out = String::with_capacity(decoded.len());
    let mut blank_run = 0;
    for line in decoded.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

/// Decode the small set of HTML entities IFDB descriptions carry, including
/// numeric `&#NN;` / `&#xNN;`. Unknown entities are left verbatim.
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp..];
        let Some(semi) = after.find(';').filter(|&i| i <= 10) else {
            out.push('&');
            rest = &after[1..];
            continue;
        };
        let ent = &after[1..semi]; // between & and ;
        let decoded = match ent {
            "quot" => Some('"'),
            "apos" => Some('\''),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "nbsp" => Some(' '),
            _ => ent
                .strip_prefix('#')
                .and_then(|n| {
                    n.strip_prefix(['x', 'X'])
                        .and_then(|h| u32::from_str_radix(h, 16).ok())
                        .or_else(|| n.parse::<u32>().ok())
                })
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &after[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Finds a direct child element named `name` in the Babel namespace (or no
/// namespace at all), and returns its trimmed text — `None` if absent or blank.
fn child_text<'a>(parent: roxmltree::Node<'a, 'a>, name: &str) -> Option<String> {
    parent
        .children()
        .find(|n| {
            n.is_element()
                && n.tag_name().name() == name
                && matches!(n.tag_name().namespace(), Some(BABEL_NS) | None)
        })
        .and_then(|n| n.text())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        // IFDB serves titles and authors with HTML entities inside the XML
        // text ("&quot;Do Not Meddle&quot;", "Grit &amp; Glory"), which the
        // XML parser has already unescaped once and the picker then printed
        // as they came. The same decoding the description gets.
        .map(decode_entities)
}

/// Finds a direct child element named `name`, ignoring namespace. Used inside
/// the `<ifdb>` subtree, which is unambiguously scoped to `IFDB_NS` already.
fn child_text_any_ns<'a>(parent: roxmltree::Node<'a, 'a>, name: &str) -> Option<String> {
    parent
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
        .and_then(|n| n.text())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(decode_entities)
}

pub fn parse(xml: &[u8]) -> Result<IFiction, IFictionError> {
    let text = std::str::from_utf8(xml).map_err(|_| IFictionError::NotIFiction)?;
    let doc = roxmltree::Document::parse(text).map_err(IFictionError::Xml)?;

    let story = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "story")
        .ok_or(IFictionError::NotIFiction)?;

    let mut result = IFiction::default();

    if let Some(bibliographic) = story
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "bibliographic")
    {
        result.title = child_text(bibliographic, "title");
        result.author = child_text(bibliographic, "author");
        result.language = child_text(bibliographic, "language");
        result.first_published = child_text(bibliographic, "firstpublished");
        result.genre = child_text(bibliographic, "genre");
        // IFDB double-encodes the description: it HTML-encodes the prose (tags
        // like `<br/>`, entities like `&quot;`/`&#039;`) and then XML-encodes
        // that, so after roxmltree's one XML decode the text still carries
        // literal HTML. Flatten it to plain text for a terminal panel.
        result.description = child_text(bibliographic, "description").map(|d| html_to_text(&d));
    }

    if let Some(identification) = story
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "identification")
    {
        result.ifids = identification
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "ifid")
            .filter_map(|n| n.text())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
    }

    if let Some(ifdb) = story.children().find(|n| {
        n.is_element() && n.tag_name().name() == "ifdb" && n.tag_name().namespace() == Some(IFDB_NS)
    }) {
        if let Some(tuid) = child_text_any_ns(ifdb, "tuid") {
            let link = child_text_any_ns(ifdb, "link");
            let cover_url = ifdb
                .children()
                .find(|n| n.is_element() && n.tag_name().name() == "coverart")
                .and_then(|coverart| child_text_any_ns(coverart, "url"));
            // An unrated game omits these (or leaves them empty), which
            // `child_text_any_ns` already reports as None — so a parse failure
            // and an absent rating land on the same, correct answer.
            let average_rating =
                child_text_any_ns(ifdb, "averageRating").and_then(|s| s.parse::<f32>().ok());
            let rating_count =
                child_text_any_ns(ifdb, "ratingCountAvg").and_then(|s| s.parse::<u32>().ok());
            result.ifdb =
                Some(IfdbExt { tuid, link, cover_url, average_rating, rating_count });
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZORK: &[u8] = include_bytes!("../tests/fixtures/ifdb-zork1.xml");

    /// The live IFDB response. Guards the many-<title> trap: the fixture has 26
    /// <title> elements, all but one inside <downloads><link>, and only
    /// <bibliographic><title> is the game's.
    /// IFDB double-encodes: the XML text of a title arrives as
    /// `&amp;quot;Do Not Meddle&amp;quot;`, so after the XML parser one layer of
    /// entities is still there. Ten titles in a 2,500-game library printed
    /// with literal `&quot;` and `&amp;` until this decoded them.
    #[test]
    fn titles_and_authors_are_entity_decoded_like_descriptions() {
        let xml = br#"<?xml version="1.0"?><ifindex xmlns="http://babel.ifarchive.org/protocol/iFiction/"><story><bibliographic><title>&amp;quot;Do Not Meddle&amp;quot;</title><author>Grit &amp;amp; Glory</author></bibliographic></story></ifindex>"#;
        let f = parse(xml).expect("parses");
        assert_eq!(f.title.as_deref(), Some("\"Do Not Meddle\""));
        assert_eq!(f.author.as_deref(), Some("Grit & Glory"));
    }

    #[test]
    fn parses_the_live_ifdb_response() {
        let f = parse(ZORK).expect("fixture parses");
        assert_eq!(f.title.as_deref(), Some("Zork I"), "the bibliographic title, not a download's");
        assert_eq!(f.author.as_deref(), Some("Marc Blank and Dave Lebling"));
        assert_eq!(f.first_published.as_deref(), Some("1980"));
        assert_eq!(f.genre.as_deref(), Some("Zorkian/Cave crawl"));
        assert!(
            f.description.as_deref().unwrap().starts_with("Many strange tales"),
            "the blurb: {:?}", f.description
        );
        assert!(f.ifids.contains(&"ZCODE-52-871125".to_string()), "IFDB groups editions: {:?}", f.ifids);
    }

    #[test]
    fn extracts_the_ifdb_extension_block() {
        let f = parse(ZORK).unwrap();
        let ext = f.ifdb.expect("ifdb extension present");
        assert_eq!(ext.tuid, "0dbnusxunq7fw5ro");
        assert_eq!(
            ext.cover_url.as_deref(),
            Some("https://ifdb.org/coverart?id=0dbnusxunq7fw5ro&version=45"),
            "entity-decoded: &amp; must become &"
        );
    }

    /// SQ-0529: the community rating rides in the same `<ifdb>` block as the
    /// tuid. Pinned against the captured live response, not from memory —
    /// `averageRating` is the value shown (3.81858407 → "3.8"), NOT the
    /// pre-rounded `starRating` of 4.
    #[test]
    fn extracts_the_ifdb_average_rating_and_count() {
        let ext = parse(ZORK).unwrap().ifdb.expect("ifdb extension present");
        let avg = ext.average_rating.expect("Zork I is rated");
        assert!((avg - 3.818_584).abs() < 1e-4, "the raw average, unrounded: {avg}");
        assert_eq!(format!("{avg:.1}"), "3.8", "one decimal is what the list shows");
        assert_eq!(ext.rating_count, Some(226), "ratingCountAvg, the sort tiebreak");
    }

    /// An unrated game still has an `<ifdb>` block (tuid, link, cover) — IFDB
    /// just omits the rating elements. That must read as None, never 0.0: a
    /// list column cannot tell "nobody rated it" from "everybody hated it".
    #[test]
    fn an_ifdb_block_without_rating_elements_yields_none_not_zero() {
        let xml = br#"<ifindex version="1.0" xmlns="http://babel.ifarchive.org/protocol/iFiction/">
            <story><bibliographic><title>Unrated</title></bibliographic>
            <ifdb xmlns="http://ifdb.org/api/xmlns"><tuid>abc123</tuid>
            <link>https://ifdb.org/viewgame?id=abc123</link></ifdb></story></ifindex>"#;
        let ext = parse(xml).unwrap().ifdb.expect("the block is present, just rating-free");
        assert_eq!(ext.tuid, "abc123", "the rest of the block still parses");
        assert_eq!(ext.average_rating, None);
        assert_eq!(ext.rating_count, None);
    }

    /// An empty `<averageRating/>` (IFDB's other way of saying "no ratings",
    /// as seen in the search feed) is also None, not a parse of "".
    #[test]
    fn an_empty_rating_element_is_none() {
        let xml = br#"<ifindex><story><bibliographic><title>Blank</title></bibliographic>
            <ifdb xmlns="http://ifdb.org/api/xmlns"><tuid>t</tuid>
            <averageRating></averageRating><ratingCountAvg></ratingCountAvg></ifdb></story></ifindex>"#;
        let ext = parse(xml).unwrap().ifdb.unwrap();
        assert_eq!((ext.average_rating, ext.rating_count), (None, None));
    }

    /// A blorb's own IFmd chunk has no `<ifdb>` block at all, so there is no
    /// rating to reach for — and asking for one must not panic or invent 0.0.
    #[test]
    fn no_ifdb_block_means_no_rating_to_read() {
        let xml = br#"<ifindex version="1.0" xmlns="http://babel.ifarchive.org/protocol/iFiction/">
            <story><bibliographic><title>Local Only</title></bibliographic></story></ifindex>"#;
        let f = parse(xml).unwrap();
        assert!(f.ifdb.is_none());
        assert_eq!(f.ifdb.as_ref().and_then(|e| e.average_rating), None);
    }

    /// A thin IFmd chunk carrying only a title must parse, not error — most of
    /// the struct is legitimately absent.
    #[test]
    fn a_minimal_ifmd_chunk_parses_with_everything_else_none() {
        let xml = br#"<ifindex version="1.0" xmlns="http://babel.ifarchive.org/protocol/iFiction/">
            <story><bibliographic><title>Curses</title></bibliographic></story></ifindex>"#;
        let f = parse(xml).unwrap();
        assert_eq!(f.title.as_deref(), Some("Curses"));
        assert!(f.author.is_none() && f.description.is_none() && f.ifdb.is_none());
    }

    /// An IFmd chunk with no default namespace (some tools emit bare iFiction).
    /// Accepted: namespace-absent is not namespace-wrong.
    #[test]
    fn a_namespaceless_chunk_still_parses() {
        let xml = br#"<ifindex><story><bibliographic><title>Bare</title></bibliographic></story></ifindex>"#;
        assert_eq!(parse(xml).unwrap().title.as_deref(), Some("Bare"));
    }

    #[test]
    fn html_to_text_flattens_ifdb_description_markup() {
        // The shape IFDB actually emits (post-XML-decode): HTML entities and
        // literal <br/> tags, as seen in Planetfall's blurb.
        let raw = "&quot;Join the Patrol!&quot;<br/>You took the poster&#039;s advice.<br /><i>Later:</i> scrubbing decks &amp; sweeping.";
        let out = html_to_text(raw);
        assert!(!out.contains('<'), "no tags survive: {out:?}");
        assert!(!out.contains("&quot;") && !out.contains("&#039;") && !out.contains("&amp;"), "entities decoded: {out:?}");
        assert!(out.contains('"') && out.contains('\''), "entities became real chars: {out:?}");
        assert!(out.contains("decks & sweeping"), "&amp; → &: {out:?}");
        assert!(out.contains("\nYou took"), "<br/> became a newline: {out:?}");
    }

    #[test]
    fn html_to_text_leaves_plain_prose_alone() {
        // Zork's blurb is already plain; flattening must not mangle it.
        let plain = "Many strange tales have been told of the fabulous treasure.";
        assert_eq!(html_to_text(plain), plain);
    }

    #[test]
    fn html_to_text_and_decode_never_panic_on_odd_input() {
        for s in ["&", "&;", "&#;", "&#xZZ;", "<unclosed", "&nosuchentity;", "&#99999999999;"] {
            let _ = html_to_text(s); // must not panic
        }
    }

    #[test]
    fn malformed_xml_errors_and_never_panics() {
        assert!(parse(b"<ifindex><story>").is_err());
        assert!(parse(b"").is_err());
        assert!(parse(b"\xff\xfe\x00garbage").is_err());
    }

    /// Whitespace around element text is incidental in XML; a title of
    /// "\n  Zork I\n  " must not reach the UI.
    #[test]
    fn text_is_trimmed_and_blanks_become_none() {
        let xml = br#"<ifindex><story><bibliographic>
            <title>  Spaced  </title><author>   </author>
        </bibliographic></story></ifindex>"#;
        let f = parse(xml).unwrap();
        assert_eq!(f.title.as_deref(), Some("Spaced"));
        assert!(f.author.is_none(), "whitespace-only is absent, not Some(\"\")");
    }
}
