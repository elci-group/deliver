//! goya-scene — build a Goya GUI from a RUGID-style scene file.
//!
//! `cargo run --bin scene [--file app.gscene] [--live]`
//!
//! The scene language (GY-LANG/1) adopts RUGID's grammar: `@ontology`,
//! `@cell`, `@button`, `position`/`extent` (absolute points) and
//! `position_p`/`extent_p` (proportional). Buttons are interactive: clicks
//! print their name. Without `--live`, programmatic checks run headless.

use goya_lang::{hit_test, layout, parse, Anchor, DrawNode};
use goya_runtime::Primitive;

const DEFAULT_SCENE: &str = "\
@ontology goya_workspace {
    @cell window {
        position: (0, 0, 0)
        extent: (400, 150, 0)
        fill: #14141c

        @cell title {
            position: (200, 24, 0)
            text: \"Goya Workspace\"
            text-size: 16
            fill: #eaeaef
            text-anchor: middle
        }

        // Three actions, RUGID style — absolute geometry per cell.
        @button open {
            position: (30, 70, 0)
            extent: (100, 40, 0)
            fill: #3267c8
            text: \"Open\"
        }
        @button save {
            position: (150, 70, 0)
            extent: (100, 40, 0)
            fill: #2a8a4a
            text: \"Save\"
        }
        @button quit {
            position: (270, 70, 0)
            extent: (100, 40, 0)
            fill: #8a2a2a
            text: \"Quit\"
        }

        @cell statusbar {
            position: (0, 128, 0)
            extent: (400, 22, 0)
            fill: #1e1e28
            @cell status {
                position_p: (0.5, 0.5, 0)
                text: \"ready\"
                text-size: 9
                fill: #9aa0b0
                text-anchor: middle
            }
        }
    }
}
";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let live = args.iter().any(|a| a == "--live");
    let path = args
        .iter()
        .position(|a| a == "--file")
        .and_then(|i| args.get(i + 1));

    let source = match path {
        Some(p) => std::fs::read_to_string(p).unwrap_or_else(|e| {
            eprintln!("goya-scene: cannot read {p}: {e}");
            std::process::exit(1);
        }),
        None => DEFAULT_SCENE.to_string(),
    };

    let scene = match parse(&source) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("goya-scene: {e}");
            std::process::exit(1);
        }
    };
    println!(
        "goya-scene: ontology '{}' parsed ({} nodes drawn at 400x150)",
        scene.name,
        layout(&scene, 400.0, 150.0).nodes.len()
    );

    if !live {
        // Headless verification: proportional child centred in statusbar.
        let frame = layout(&scene, 400.0, 150.0);
        let status = frame.find("status").expect("status node present");
        // position_p 0.5 places the cell's ORIGIN at the parent midpoint;
        // with anchor=middle the text centres there. The origin check is
        // against parent.x + 0.5*parent.width.
        let statusbar = frame.find("statusbar").expect("statusbar node present");
        let expected_x = statusbar.rect.x + statusbar.rect.width / 2;
        let ok = (status.rect.x - expected_x).abs() < 5_000;
        println!(
            "proportional centring of status cell [{}]",
            if ok { "PASS" } else { "FAIL" }
        );
        let hit = hit_test(&frame, 80_000, 90_000);
        println!(
            "hit test on 'Open' button [{}]",
            if hit.as_deref() == Some("open") {
                "PASS"
            } else {
                "FAIL"
            }
        );
        if !(ok && hit.as_deref() == Some("open")) {
            std::process::exit(1);
        }
        return;
    }

    let mut win = match goya_wayland::GoyaWindow::create("goya-scene", 400, 150) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("goya-scene: live unavailable: {e}");
            return;
        }
    };
    let mut frames = 0u32;
    let mut clicked = std::collections::BTreeMap::new();
    while frames < 1800 && !win.close_requested() {
        for ev in win.poll() {
            if let goya_wayland::PlatformEvent::PointerPress(x, y) = ev {
                let frame = layout(&scene, 400.0, 150.0);
                if let Some(name) = hit_test(
                    &frame,
                    (x as i64) * goya_wayland::MP_PER_PX,
                    (y as i64) * goya_wayland::MP_PER_PX,
                ) {
                    println!("goya-scene: button '{name}' activated");
                    *clicked.entry(name).or_insert(0u32) += 1;
                }
            }
        }
        let frame = layout(&scene, 400.0, 150.0);
        let prims: Vec<Primitive> = frame.nodes.iter().map(present_node).collect();
        let _ = win.present(&prims, [235, 235, 240], [20, 20, 28]);
        win.pump(1);
        frames += 1;
    }
    println!("goya-scene live: {frames} frames, clicks: {clicked:?}");
}

/// Convert a scene draw node into the renderer's primitive vocabulary.
fn present_node(n: &DrawNode) -> Primitive {
    let rect = goya_layout::Rect {
        x: n.rect.x,
        y: n.rect.y,
        width: n.rect.width,
        height: n.rect.height,
    };
    let mut label = n.text.clone().unwrap_or_default();
    if n.is_button && label.is_empty() {
        label = n.name.clone();
    }
    if n.is_button {
        Primitive::Button(rect, label)
    } else if n.text.is_some() {
        Primitive::Text(rect, label)
    } else {
        Primitive::Panel(rect)
    }
}

// Anchor is part of the language surface; keep the import honest until the
// renderer supports anchored text (Wave 4 typography).
#[allow(unused)]
fn _anchor_check(a: Anchor) -> bool {
    matches!(a, Anchor::Start | Anchor::Middle | Anchor::End)
}
