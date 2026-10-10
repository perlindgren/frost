//! A little sprite utility: pick PNGs with native file dialogs (rfd),
//! keep up to seven of them side by side, and edit them in a work area
//! above a strip of sprite slots.
//!
//! The window splits into two regions: the **work area** on top, where
//! the active sprite is displayed over a transparency checkerboard with a
//! bounding box around it, and a **slot strip** at the bottom — seven
//! 100x100 px slots, each holding a minimized copy of one sprite as it
//! was loaded. The leftmost slot takes the first sprite. Click a slot to
//! make its sprite the active one (the work area and every operation
//! follow it), and drag a slot onto another to swap the two sprites'
//! positions.
//!
//! Every dialog is a child of the window, so it opens on top of it (a
//! parentless dialog lands at the screen's default spot, far from the
//! window). Dialogs appear only when asked for — **Open**, **Ctrl-O**,
//! **Save as** — never on their own: a launch with no argument simply
//! opens an empty workbench, and canceling a dialog returns to it.
//!
//! The controls are draggable [`frost::Ui`] panels (drag one by its
//! title bar, click the bar to fold it away) — and the view's own state
//! lives in the menu bar: the View menu carries the zoom level and the
//! last click as readouts, and the checker's two grey levels as live
//! [`frost::MenuItem::slider`] lines whose tracks you can grab without
//! leaving the list. Sliders in a menu is an unusual uniform; the menu
//! widget sewed it, and it fits.
//!
//! The File and Operations menus edit the active sprite: **Open**
//! (**Ctrl-O**) adds another sprite to the next free slot — while the
//! slots fill up, the newest sprite becomes the active one; **Crop** cuts the texture to
//! the selection rectangle if one is dragged (a left-drag over the work
//! area draws it; a click that barely moves instead logs the pixel and
//! its slot) — and to the opaque content when there is no selection,
//! trimming the fully transparent borders. The sidecar's positions ride
//! the cut: each shifts by the crop's origin, clamped to the new bounds,
//! so it stays put relative to the pixels that survive. Every crop is
//! remembered on a history that spans the whole bench: **Ctrl-Z** steps
//! back over any command — crop, mark, frame edit, slot move — and
//! **Ctrl-Shift-Z** steps forward again. **Save** writes the texture over the file it came
//! from, and **Save As** asks a native dialog for a new name, defaulted
//! to the sprite's current file name; both ask for confirmation before
//! overwriting an existing file — and, when the sprite has a sidecar
//! `.ron`, writes the tree back out beside the PNG. Each file is written
//! only when its content actually differs from it: a sprite whose pixels
//! and tree match the disk has nothing to save, and **Save** shows that
//! instead of a button (the name's `*` marks the difference, and **Save
//! As** stays a write — a new file always one). A sprite without a
//! sidecar gains one the first time it names a grid: the save that sets
//! the atlas also creates the file it persists in. **Close** takes the
//! active sprite out of its slot, and every later slot shifts left to
//! fill the gap.
//!
//! The "Animation" panel turns the loaded sprites into an animation: a
//! frame is a **set of layers** — snapshots of the active slot's shape,
//! stacked in the work area — and each frame holds for its own time
//! before the next one comes on. Playback either wraps at the ends
//! (**loop**) or walks back through the frames (**ping-pong**), and the
//! play button or space starts and stops the clock. The work area shows
//! the animation's current frame while any frame exists — which is also
//! the frame being edited, so the panel previews as it builds; the solo
//! active sprite returns when the last frame is deleted.
//!
//! The wheel zooms about the cursor (the point under it stays put),
//! **Ctrl-+** and **Ctrl--** step the zoom about the work area's centre
//! (the same characters and keys as the overlay's Alt pair, so a `+`
//! means plus on every layout), a middle-drag — the wheel button held —
//! grabs the work area and moves the sprite with the cursor,
//! and [`frost::Ui::hovering`] is what tells a click on the UI from one
//! on the scene: a press the UI claims never draws a selection, touches
//! a slot or logs a pixel.
//!
//! The checkerboard is the sprite's on-screen bounding box, clipped to
//! the work area, filled with light/dark grey cells that never scale with
//! the sprite, so it reveals the sprite's transparency at any zoom or
//! pan. The fill is a tiny sprite — one texture pixel per cell, nearest-
//! neighbor sampled through [`frost::Shape::sprite_bytes_nearest`] — so
//! the cell edges stay hard at any window size, and the texture is
//! rebuilt only when the visible cell count or a grey level changes. The
//! bounding box itself is four [`frost::Canvas::line`] strokes around the
//! sprite's full on-screen rectangle; the parts outside are clipped.
//!
//! Run with:
//!
//! ```text
//! cargo run -p sprite_util
//! ```
//!
//! (Set `RUST_LOG=info` to see the click lines.)
//!
//! The first sprite's source can be given on the command line with
//! `-i`/`--input`: a PNG file is opened immediately, without any file
//! dialog, and a folder is where the first *Open* dialog will open — a
//! relative path is expanded against the working directory and resolved
//! to an absolute one, because the native dialog (notably on Windows)
//! ignores anything else and opens in its default spot. Without the
//! argument the tool starts empty and dialogs open in the folder it was
//! launched from:
//!
//! ```text
//! cargo run -p sprite_util -- -i assets/sprites/bird1.png
//! cargo run -p sprite_util -- -i assets/sprites
//! ```

//! A sprite may bring a sidecar: when `<name>.png` has a `<name>.ron`
//! beside it, the file — the RON subset `ron_view` speaks, parsed by
//! the crate's own `frost::ron` — opens as a foldable
//! tree — one `Ui` panel per open file, each titled by its file name,
//! cascading from the work area's lower right, dragging and folding like
//! every other panel and scrolling by its own pair of handles (the
//! wheel too, sideways with Shift; rows scroll by the character, so the
//! text never escapes the panel). A corner grip resizes each view —
//! narrower views is exactly what the horizontal handle is for — and
//! every rectangle the tree reads is the plate the UI actually painted,
//! so text and background never disagree. The boxed × in a view's bar
//! closes the file: sprite and sidecar together. Every position the file names — a
//! two-number tuple — is marked on the sprite in its own colour, the
//! very colour its tree rows carry: click a row to pick that position
//! (its marker swells, the row bands, its file becomes the active one),
//! click the sprite to move the position to the clicked pixel, and a
//! second click or Escape lets go. Rows without a position fold on
//! click. No sidecar or an unparseable one: the sprite simply loads.
//!
//! Lists are editable from the tree itself: an open sequence's head row
//! carries a green **+** (append — a copy of the last entry, and of a
//! position list the fresh entry lands picked, waiting for the sprite
//! click that places it), every entry row a red **×**, and pressing an
//! entry row and dragging up or down slides it through the list, other
//! entries making way. On the sprite, an entry of a list wears its
//! seat number beside the marker, so a reorder is legible at a glance.
//! Saving rewrites the sidecar with its comments intact — leads above
//! their value, trails beside it, the header and trailer bookending the
//! tree — only the line layout is the writer's own.
//!
//! The View menu switches the workbench between two views — **Markers**
//! (the full editing desk, the panels below) and **Atlas** (only the
//! Atlas panel, the grid's rows and columns) — and the view's own name
//! is noted at the status band's right end. The tile strip at the bottom
//! belongs to both: a sprite is picked and worked on identically either
//! way. A launch with no argument starts empty, in Markers.
//!
//! A classic menu bar runs across the window's top edge: **File** unfolds
//! **Open / Close / Save / Save as / Quit** and **View** unfolds
//! **Markers / Atlas / Tile Map**; while a list is open the pointer
//! tracks along the bar — sliding onto another title switches the list
//! without another click — hovered lines highlight, and a click anywhere
//! else dismisses. The bar itself is fully part of the app — drawn
//! over every panel, owning every click that lands on it — and its
//! items run their named verbs. Its one note, right-aligned, voices
//! what the file's `shapes:` got wrong; everything else the bar used
//! to carry moved to the bottom of the window, where it belongs:
//!
//! The status band spans the window's bottom edge — the last command
//! at the left, the view's own name pinned at the right — and the slot
//! strip stands on it. A click anywhere on the band opens the log:
//! the last messages as four docked rows above the slots, newest
//! first. The up and down arrows step the highlight, the left and
//! right arrows and the bottom thumb slide long lines, the right
//! thumb and the wheel drive the shown window, and a click lands the
//! highlight on its row; Escape, or another click on the band, puts
//! the panel away. The band remembers fifty messages, collapses
//! repeats into one, and forgets the oldest first.

use clap::Parser;
use std::sync::Arc;

#[path = "../../examples/ron_view/view.rs"]
mod ron_view;

mod actions;
mod art;
use art::*;
mod band;
use band::*;
mod clip;
mod desk;
use desk::*;
mod draw;
mod journal;
mod layout;
mod map;
mod process;
mod ron_panels;
mod sidecar;
use sidecar::*;
mod spots;
use spots::*;
mod strip;
use strip::*;
mod theme;
use theme::*;
mod world;
use world::*;

/// The command line arguments.
#[derive(Parser, Debug)]
#[command(name = "sprite_util")]
struct Args {
    /// A PNG file to open immediately, skipping any file dialog — or a
    /// folder, in which case later Open dialogs open there (a relative
    /// path is expanded against the working directory). Without the
    /// argument the tool starts empty, and dialogs open in the working
    /// directory.
    #[arg(short = 'i', long = "input", value_name = "PATH")]
    input: Option<std::path::PathBuf>,
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    // The folder the dialog opens in when no input folder is given:
    // where the example was launched from, not where its binary lives.
    let cwd = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(err) => {
            log::error!("failed to read the working directory: {err}");
            std::process::exit(1);
        }
    };

    // The first sprite's source: a `-i`/`--input` file is opened
    // immediately, without any file dialog; a `-i`/`--input` folder is
    // where the file dialog opens; and without the argument the dialog
    // opens in the working directory. The dialog folder is resolved to an
    // absolute path first — the native dialog (on Windows at least)
    // ignores relative paths and falls back to its default location, so
    // the path is joined onto the working directory (an absolute input
    // replaces it) and canonicalized.
    let Args { input } = Args::parse();
    // Where the workbench starts. The boot construction below honors
    // it (a command-line sprite must not land on a canvas that starts
    // on the tile-map desk), as does the `Demo`'s own initial state.
    let start_view = View::Markers;
    let (immediate, dialog_dir) = match &input {
        Some(path) if path.is_file() => (Some(path.as_path()), None),
        Some(path) if path.is_dir() => {
            let dir = cwd.join(path).canonicalize().unwrap_or_else(|err| {
                log::error!("failed to resolve the folder '{}': {err}", path.display());
                std::process::exit(1);
            });
            (None, Some(dir))
        }
        Some(path) => {
            log::error!("'{}' is neither a file nor a folder", path.display());
            std::process::exit(1);
        }
        None => (None, Some(cwd)),
    };

    // A file on the command line is loaded here, before the window
    // exists, and takes the leftmost slot; otherwise all slots start
    // empty: the workbench is the tool's neutral state, and the way to
    // fill it is whatever the user asks for next — File / Open, the
    // panel's open button, Ctrl-O — never something the tool does on
    // its own at launch.
    let sprites: Vec<Sprite> = match immediate {
        Some(file) => vec![read_sprite(file).unwrap_or_else(|err| {
            log::error!("{err}");
            std::process::exit(1);
        })],
        None => Vec::new(),
    };
    let dir = sprites
        .first()
        .and_then(|sp| sp.path.parent())
        .filter(|d| !d.as_os_str().is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| dialog_dir.clone());

    // The HUD font, the same monospaced variable font the diagnostics
    // overlay uses — shared by the panel's labels and the menu bar. The
    // path comes from the repository root via `asset_path`, not from
    // `CARGO_MANIFEST_DIR`, which names the tool's package directory.
    let font = asset_path(HUD_FONT);
    let ui = frost::Ui::from_font(&font).expect("failed to load the UI font");
    // The seat numbers' font: one Arc's worth of the same FiraCode,
    // every marker's text cloning the pointer, never re-reading the
    // file.
    let ron_font: Arc<[u8]> =
        Arc::from(std::fs::read(&font).expect("failed to read the sidecar font"));
    // The help line that used to hang under the title bar is gone: the
    // menu bar, with its names and shortcuts, is the map now. The node
    // stays — the scene's fixed indices do not shift for an undrawn
    // shape.
    let help = None;

    // The work area's layer pool comes first: the process points the
    // nodes at the animation's current frame (or the active sprite
    // alone, in the first node). Then the fixed nodes: help, marker and
    // checker.
    let work_shape = sprites.first().map(|sp| sp.shape.clone());
    let mut children: Vec<Box<frost::SceneNode>> = (0..LAYER_NODES)
        .map(|i| {
            Box::new(frost::SceneNode {
                // A command-line sprite is already in the first node —
                // unless the tool boots straight onto the tile-map
                // desk, which shows maps, not tilesets.
                shape: if i == 0 && start_view != View::TileMap {
                    work_shape.clone()
                } else {
                    None
                },
                order: i as f32 * 0.01, // layered, under the strip's 0.2
                ..Default::default()
            })
        })
        .collect();
    children.extend(vec![
        Box::new(frost::SceneNode {
            // The usage line, pinned near the top by the process.
            shape: help,
            order: 1.0,
            ..Default::default()
        }),
        Box::new(frost::SceneNode {
            // The marker dot left at the last click; radius 0 hides it.
            shape: Some(frost::Shape::Circle {
                center: [0.0, 0.0],
                radius: 0.0,
                color: MARKER,
            }),
            order: 2.0,
            ..Default::default()
        }),
        Box::new(frost::SceneNode {
            // The checker backdrop behind the sprite: the process builds
            // its tiny one-texel-per-cell sprite and positions it each
            // frame.
            shape: None,
            order: CHECK_ORDER,
            ..Default::default()
        }),
    ]);
    // One node per slot; the process points them at the thumbnails. A
    // command-line sprite shows in its slot from the first frame.
    children.extend((0..SLOTS).map(|i| {
        Box::new(frost::SceneNode {
            shape: sprites.get(i).map(|sp| sp.thumb.clone()),
            order: THUMB_ORDER,
            ..Default::default()
        })
    }));
    // The position markers' pool: two circles per position — the colour
    // dot and its white centre — and one seat-number text each.
    children.extend((0..2 * SPOTS_MAX).map(|_| {
        Box::new(frost::SceneNode {
            order: SPOT_ORDER,
            ..Default::default()
        })
    }));
    children.extend((0..BAND_CELLS + 2).map(|_| {
        // The tileset band's cells, the cursor ghost, and the held-
        // brush chip: shapeless until the tile-map desk points them
        // at the active tileset.
        Box::new(frost::SceneNode::default())
    }));
    children.extend((0..SPOTS_MAX).map(|_| {
        Box::new(frost::SceneNode {
            order: SPOT_ORDER + 0.05,
            ..Default::default()
        })
    }));

    let scene = frost::Scene::new(frost::SceneNode {
        // Dark background; the node's transform is ignored.
        shape: Some(frost::Shape::Background { color: BG }),
        children,
        ..Default::default()
    });

    if let Err(err) = frost::run_configured(
        scene,
        Demo {
            ui,
            sprites,
            active: 0,
            anim: Anim {
                looping: true,
                dir: 1,
                ..Anim::default()
            },
            zoom: 1.0,
            offset: [0.0, 0.0],
            selection: None,
            drag_from: None,
            slot_drag: None,
            slot_scroll: 0.0,
            slot_scrub: None,
            band_origin: None,
            band_zoom: 1.0,
            band_drag: None,
            was_undo: false,
            was_open: false,
            was_space: false,
            was_esc: false,
            light: GREY_LIGHT,
            dark: GREY_DARK,
            checker_key: (0, 0, 0, 0),
            was_down: false,
            last_mouse: None,
            last_click: None,
            dir,
            view: start_view,
            band_base: None,
            band_key: None,
            cell_shapes: Vec::new(),
            picked_cell: None,
            maps: Vec::new(),
            paint: None,
            first_frame: true,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            was_redo: false,
            was_zoom_in: false,
            was_zoom_out: false,
            brush: 0,
            was_flipx: false,
            was_flipy: false,
            was_turn: false,
            attached: None,
            status: String::new(),
            log: Vec::new(),
            log_seen: String::new(),
            log_open: false,
            log_sel: 0,
            log_top: 0,
            log_sx: 0.0,
            log_grab: None,
            log_arrows: [false; 4],
            show_shapes: true,
            ron_font,
            r_press: None,
            m_press: None,
            was_rdown: false,
            was_mdown: false,
            quit_prompt: false,
            was_esc_prompt: false,
            was_del: false,
            clip: None,
            sel: None,
            sel_drag: None,
            was_block_esc: false,
            ron_drag: None,
            ron_state: frost::TreeState::default(),
            ron_views: Vec::new(),
        },
        // The room a sidecar tree and the art both want — in logical
        // pixels, so on a 2x display this opens as a 1440-point-wide
        // window rendering at 2880 physical: crisp panels and fonts,
        // and it still fits a laptop screen (1920 logical would not).
        frost::Config {
            window_size: Some([1440, 810]),
            // Escape belongs to the editor: the exit box asks before
            // anything is lost — and so does the window's close
            // button, which asks the very same question.
            escape_exits: false,
            close_exits: false,
            ..Default::default()
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
