//! `lanthorn-mapgen` — draw a story's whole map from the story file, without
//! playing it (SQ-1306).
//!
//! A thin front end over [`app::mapgen`]: mount, generate, lay out, write. Every
//! decision worth explaining is explained there; this file is argument parsing,
//! one summary block and an exit code.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use app::mapgen::{self, Artefacts, MapgenOptions};

/// The exit code for "this story file has no static map source". Distinct from
/// 1 (an ordinary failure: unreadable file, bad path, write error) so a script
/// sweeping a shelf of stories can tell "cannot map this one" from "something
/// went wrong", and skip rather than stop.
const EXIT_NO_SOURCE: u8 = 2;

#[derive(Parser)]
#[command(
    name = "lanthorn-mapgen",
    version,
    about = "Draw an interactive-fiction story's whole map from the story file, without playing it.",
    long_about = "Draw an interactive-fiction story's whole map from the story file, without \
playing it.\n\n\
Reads the map a story COMPILED — Inform 7's own map table, the Inform 6 library's exit \
properties, Infocom's ZIL exit properties, or a Scott Adams database's room table — lays it out \
with the same automapper the lanthorn interpreter draws with, and writes it out in four formats:\n\n  \
<stem>.map.txt   an annotated text dump with an ASCII drawing of the map\n  \
<stem>.svg       the drawn map as vector graphics\n  \
<stem>.dot       a Graphviz graph, for dot/neato and other graph tools\n  \
<stem>.map.json  a documented, versioned JSON map for other programs to read\n\n\
Understands the same files lanthorn does: bare story files (.z3-.z8, .ulx, .dat), Blorbs \
(.zblorb, .gblorb, .blb), zips, and release disk images.\n\n\
LAYERS. A maze room, and a portal-only region big enough to be a floor plan, are split onto \
their own layer — the same split the interpreter offers a player as a prompt, applied as if \
every prompt were accepted. --no-auto-layers turns this off for one flat map; --layer-min sets \
how big a portal-only region has to be first (mazes have no floor).\n\n\
LIMITS. This is the map as compiled, so a passage a story builds or removes while it runs is \
not in it, and neither is one whose destination a routine decides. Conditional exits and doors \
ARE included, and are marked as such. Some stories declare no map anywhere in the file; \
mapgen exits with status 2 and says so.\n\n\
EXIT STATUS\n  \
0  a map was written\n  \
1  the file could not be read, or an output could not be written\n  \
2  the file was read but declares no map this tool can find"
)]
struct Cli {
    /// The story file to map.
    #[arg(value_name = "STORY")]
    story: PathBuf,

    /// Directory to write the artefacts into (created if absent).
    #[arg(long, value_name = "DIR", default_value = ".")]
    out: PathBuf,

    /// Write the text dump. Naming any format writes only the ones named.
    #[arg(long)]
    dump: bool,

    /// Write the SVG drawing.
    #[arg(long)]
    svg: bool,

    /// Write the Graphviz DOT graph.
    #[arg(long)]
    dot: bool,

    /// Write the JSON map.
    #[arg(long)]
    json: bool,

    /// Skip layout: report the topology only, with no room positions.
    ///
    /// Faster on a very large map, and the right choice when the consumer does
    /// its own layout. The SVG and the text dump's drawing both need positions,
    /// so they are far less useful without it.
    #[arg(long)]
    no_layout: bool,

    /// Do not split mazes and portal-only regions onto their own layers
    /// (SQ-1308): everything lands on one flat map, as before that quest.
    #[arg(long)]
    no_auto_layers: bool,

    /// The smallest portal-only region worth its own layer. Defaults to the
    /// same floor the live app's layer suggestions use
    /// (`mapper::suggest::STRUCTURAL_FLOOR`) so a static map and a played one
    /// agree; a maze region has no floor and always gets its own layer.
    #[arg(long, value_name = "N")]
    layer_min: Option<usize>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let mut opts = MapgenOptions { auto_layers: !cli.no_auto_layers, ..MapgenOptions::default() };
    if let Some(n) = cli.layer_min {
        opts.layer_min = n;
    }

    let map = match mapgen::generate_with_options(&cli.story, !cli.no_layout, &opts) {
        Ok(m) => m,
        Err(e @ mapgen::GenError::NoStaticSource(_)) => {
            eprintln!("lanthorn-mapgen: {}: {e}", cli.story.display());
            return ExitCode::from(EXIT_NO_SOURCE);
        }
        Err(e) => {
            eprintln!("lanthorn-mapgen: {}: {e}", cli.story.display());
            return ExitCode::FAILURE;
        }
    };

    // Naming no format at all means "all of them"; naming any means "only
    // these", so the common case needs no flags and a scripted one is explicit.
    let mut what =
        Artefacts { dump: cli.dump, svg: cli.svg, dot: cli.dot, json: cli.json };
    if what.none_selected() {
        what = Artefacts::default();
    }

    // The story's own file name without its extension, so `advent.blb` writes
    // `advent.map.txt` beside its siblings.
    let stem = cli
        .story
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "map".to_string());

    let written = match mapgen::write_artefacts(&map, &cli.out, &stem, what) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("lanthorn-mapgen: writing to {}: {e}", cli.out.display());
            return ExitCode::FAILURE;
        }
    };

    let layout = match map.layout_time {
        Some(d) => format!("{:.2}s", d.as_secs_f64()),
        None => "skipped".to_string(),
    };
    println!("{}", map.story.file);
    println!("  engine        {}", map.story.engine);
    println!("  source        {}", map.source.as_str());
    if let (Some(r), Some(s)) = (map.story.release, map.story.serial.as_deref()) {
        println!("  release       {r} / serial {s}");
    }
    println!("  rooms         {}", map.graph.rooms().count());
    println!("  named rooms   {}", map.named_rooms());
    println!("  exits         {}", map.facts.len());
    println!("  doors         {}", map.doors());
    println!("  conditionals  {}", map.conditionals());
    println!("  layout        {layout}");
    let mut layer_ids: Vec<_> = map.graph.layers().keys().copied().collect();
    layer_ids.sort_unstable();
    println!("  layers        {}", layer_ids.len());
    for id in layer_ids {
        let n = map.graph.rooms_in_layer(id).len();
        let maze = if map.graph.layer_is_maze(id) { ", maze" } else { "" };
        println!("    {} (id {id}): {n} rooms{maze}", map.graph.layer_name(id));
    }
    for p in &written {
        println!("  wrote         {}", p.display());
    }
    ExitCode::SUCCESS
}
