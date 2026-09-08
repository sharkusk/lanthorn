//! Foreign-writer CMem restore proof (SQ-1415 audit item 1).
//!
//! `startsavetest.gblorb` is Andrew Plotkin's own Glulx unit-test game
//! (eblong.com/zarf/glulx/startsavetest.gblorb, `.inf` source alongside it):
//! its executable calls `@restore` on its own embedded Blorb `Data` resource
//! at startup — a save file glulxe's writer produced, whose `CMem` chunk
//! omits the trailing zero run once every remaining byte is unchanged from
//! the original image (`serial.c` `write_memstate`: "It's possible we've got
//! a run left over, but we don't write it"). Before the SQ-1415 fix,
//! `decompress_ram` treated that shortfall as `BadSave("CMem data
//! truncated")`, so the restore failed and the game printed its OWN
//! "didn't work" message instead of the success line below — this is
//! exactly the shape of Counterfeit Monkey's boot-cache save (SQ-0595),
//! just small enough to vendor and drive headlessly in seconds.
//!
//! Freely redistributable (Andrew Plotkin's own public test suite, same
//! footing as `crates/gvm-cli/tests/fixtures/glulxercise.ulx`), so this
//! fixture is committed rather than gitignored — no skip-if-absent needed.

use gvm::{Machine, Memory, StepResult, TestBackend};

fn fixture_bytes() -> Vec<u8> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/startsavetest.gblorb");
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// A Blorb `Data` resource number, its bytes, and whether it's a `TEXT` chunk.
type DataResource = (u32, Vec<u8>, bool);

/// Extract the Glulx executable and every `Data` resource from the Blorb, the
/// latter so a `TestBackend` can serve `glk_stream_open_resource` — mirroring
/// what a real host (`gvm-cli`, `app::glulx_session`) does via the retained
/// Blorb, since `TestBackend` has no Blorb of its own to consult.
fn extract_glulx_and_data(bytes: Vec<u8>) -> (Vec<u8>, Vec<DataResource>) {
    let b = blorb::Blorb::parse(bytes).expect("valid Blorb");
    let image = match b.executable() {
        Ok((blorb::ExecKind::Glulx, data)) => data.to_vec(),
        other => panic!("expected a Glulx Blorb, got {other:?}"),
    };
    let data_resources = b
        .resources()
        .iter()
        .filter(|e| &e.usage == b"Data")
        .map(|e| {
            let (chunk_type, payload) = b.resource(b"Data", e.number).expect("resource listed in the index");
            (e.number, payload.to_vec(), chunk_type == b"TEXT")
        })
        .collect();
    (image, data_resources)
}

/// Drive to the first stable input request (or a fault/quit), capturing every
/// text-buffer window's transcript along the way.
const MAX_STEPS: u64 = 20_000_000;

#[test]
fn startsavetest_autorestores_on_boot() {
    let (image, data_resources) = extract_glulx_and_data(fixture_bytes());
    assert!(!data_resources.is_empty(), "the fixture must carry at least one Data resource (its embedded save)");

    let mem = Memory::new(image).expect("valid Glulx image");
    let mut backend = TestBackend::new();
    for (num, bytes, is_text) in data_resources {
        backend = backend.with_data_resource(num, bytes, is_text);
    }
    let mut m = Machine::with_glk(mem, Box::new(backend));

    let mut steps = 0u64;
    loop {
        match m.step() {
            StepResult::Continue => {
                steps += 1;
                assert!(steps < MAX_STEPS, "runaway: startsavetest did not settle within {MAX_STEPS} steps");
            }
            StepResult::NeedLine { .. } | StepResult::NeedChar { .. } | StepResult::Quit => break,
            other => panic!("unexpected step result: {other:?}"),
        }
    }

    let transcript = m.backend_mut().as_any_mut().downcast_mut::<TestBackend>().unwrap().all_text();
    assert!(
        transcript.contains("The autorestore file has been restored successfully."),
        "expected the autorestore success line; got:\n{transcript}",
    );
}
