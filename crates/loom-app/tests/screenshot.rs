//! Smoke test for `loom-app --screenshot`: it renders an example patch to a
//! PNG file and exits with status 0. Skipped where no GPU adapter is found.

use std::path::{Path, PathBuf};
use std::process::Command;

fn subtractive() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/patches/subtractive.ron")
}

/// The subtractive example with modulation, so it has badges, modulation
/// lines, and range arcs. Cutoff has two sources, so an arc drag that
/// picks one changes its arc. Module 10 is a button whose `on` is
/// modulated, which has no range arc. Written next to the images, as
/// `name`: tests run in parallel, so each needs its own file.
fn modulated(name: &str) -> PathBuf {
    let source = std::fs::read_to_string(subtractive()).expect("example patch");
    let last_module = "PatchModule(id: 9, kind: \"loom.io.audio-out@1\"),";
    let source = source.replace(
        last_module,
        &format!("{last_module}\n        PatchModule(id: 10, kind: \"loom.ui.button@1\"),"),
    );
    let end = source.rfind(')').expect("a patch ends with `)`");
    let patch = format!(
        "{}    modulation: [\n        \
         (source: (5, \"out\"), target: (4, \"cutoff\"), depth: 0.25),\n        \
         (source: (7, \"out\"), target: (4, \"cutoff\"), depth: -0.5),\n        \
         (source: (7, \"out\"), target: (4, \"resonance\"), depth: 0.4),\n        \
         (source: (5, \"out\"), target: (10, \"on\"), depth: 1.0),\n    ],\n)\n",
        &source[..end]
    );
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, patch).expect("patch written");
    path
}

/// Runs the app on the subtractive example with these arguments and returns
/// the decoded image as (width, height, RGBA pixels), or `None` when there
/// is no GPU.
fn screenshot(name: &str, args: &[&str]) -> Option<(u32, u32, Vec<u8>)> {
    screenshot_of(&subtractive(), name, args)
}

fn screenshot_of(patch: &Path, name: &str, args: &[&str]) -> Option<(u32, u32, Vec<u8>)> {
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_file(&out);
    let result = Command::new(env!("CARGO_BIN_EXE_loom-app"))
        .arg(patch)
        .arg("--screenshot")
        .arg(&out)
        .args(args)
        .output()
        .expect("loom-app runs");
    let stderr = String::from_utf8_lossy(&result.stderr);
    // Must match `screenshot::NO_GPU`.
    if stderr.contains("no GPU adapter") {
        eprintln!("skipped: {stderr}");
        return None;
    }
    assert!(result.status.success(), "loom-app failed: {stderr}");

    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&out).expect("PNG written")));
    let mut reader = decoder.read_info().expect("a PNG header");
    let mut pixels = vec![0; reader.output_buffer_size().expect("a sane size")];
    let info = reader.next_frame(&mut pixels).expect("PNG data");
    assert_eq!(info.color_type, png::ColorType::Rgba);
    pixels.truncate(info.buffer_size());
    Some((info.width, info.height, pixels))
}

/// Number of distinct colours, to tell a drawn patch from a flat fill.
fn colours(pixels: &[u8]) -> usize {
    let mut seen: Vec<[u8; 4]> = pixels.as_chunks::<4>().0.to_vec();
    seen.sort_unstable();
    seen.dedup();
    seen.len()
}

#[test]
fn renders_an_example_patch() {
    let Some((width, height, pixels)) = screenshot("subtractive.png", &["--size", "320x200"]) else {
        return;
    };
    assert_eq!((width, height), (320, 200));
    // Tiles, cables, knobs, and text give many colours. A blank canvas has
    // only the background and the grid dots.
    assert!(colours(&pixels) > 50, "only {} colours", colours(&pixels));
}

#[test]
fn renders_at_a_scale_with_focus() {
    let focused = |name, focus| {
        screenshot(
            name,
            &["--size", "160x100", "--scale", "2", "--zoom", "2", "--focus", focus],
        )
    };
    let Some((width, height, cutoff)) = focused("focus-cutoff.png", "4:cutoff") else {
        return;
    };
    assert_eq!((width, height), (320, 200));
    assert!(colours(&cutoff) > 50, "only {} colours", colours(&cutoff));
    // Both views centre on the same module at the same zoom, so only the
    // focus ring and the value text move. If focus were not drawn, the
    // images would be the same.
    let (_, _, resonance) = focused("focus-resonance.png", "4:resonance").expect("a GPU, as above");
    assert_ne!(cutoff, resonance, "focus changed nothing in the image");
}

/// The pointer, a pinned badge, hover text, and a drag change the image.
#[test]
fn renders_pointer_pin_and_drags() {
    let patch = modulated("modulated-drags.ron");
    let view = ["--size", "350x200", "--zoom", "2", "--center", "4:cutoff"];
    let image = |name: &str, extra: &[&str]| screenshot_of(&patch, name, &[&view[..], extra].concat()).map(|i| i.2);
    let Some(plain) = image("plain.png", &[]) else {
        return;
    };
    for (name, extra) in [
        ("hover.png", &["--pointer", "4:cutoff", "--hover-text"][..]),
        ("pin.png", &["--pin", "5:out"]),
        ("wire.png", &["--pointer", "4:resonance", "--wire", "3:out"]),
        // Pulls the cable from the saw off the filter input.
        ("pulled.png", &["--pointer", "4:resonance", "--wire", "4:in"]),
    ] {
        let pixels = image(name, extra).expect("a GPU, as for the plain image");
        assert_ne!(pixels, plain, "{name} looks like the plain image");
    }

    // With 7:out pinned, an arc drag on cutoff shows only 7's assignment,
    // so the arc changes. Nothing else differs between the two images.
    let pinned = image("pinned.png", &["--pin", "7:out"]).expect("a GPU");
    let dragging = image("depth.png", &["--pin", "7:out", "--depth", "4:cutoff"]).expect("a GPU");
    assert_ne!(dragging, pinned, "the arc drag did not change the arc");
}

fn fails(patch: &Path, args: &[&str]) -> String {
    let result = Command::new(env!("CARGO_BIN_EXE_loom-app"))
        .arg(patch)
        .arg("--screenshot")
        .arg(PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("unused.png"))
        .args(args)
        .output()
        .expect("loom-app runs");
    assert!(!result.status.success());
    String::from_utf8_lossy(&result.stderr).into_owned()
}

#[test]
fn an_unknown_control_fails() {
    assert!(fails(&subtractive(), &["--focus", "4:nope"]).contains("no control for parameter `nope`"));
    assert!(fails(&subtractive(), &["--pointer", "4:nope"]).contains("no control or port `nope`"));
}

#[test]
fn a_depth_drag_needs_a_knob() {
    let error = fails(&modulated("modulated-knob.ron"), &["--depth", "10:on"]);
    assert!(error.contains("is not a knob"), "{error}");
}

#[test]
fn a_pin_without_a_badge_fails() {
    assert!(fails(&subtractive(), &["--pin", "5:out"]).contains("no badge to pin"));
}
