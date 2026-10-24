//! The pages the hill opens onto and the menu they are built of:
//! the page model and its Escape rule, the row vocabulary and its
//! geometry, the rows a page presents, the input that walks them,
//! and every word and stone plate they are painted with — with
//! the tests that hold the menu, the Escape rule, and the
//! settings file to their word.

use crate::*;

/// The menu's memory, grouped off `Demo`: which row is lit, where
/// the pointer stood, the held arrows' repeat clocks, the slider
/// under the button, the progress page's scroll, and the overlay's
/// breath. The pages reach the same record through `Demo::menu`.
pub(crate) struct MenuNav {
    /// The highlighted row — what the arrow keys walk and Enter
    /// presses. The one highlight there is: the pointer takes the
    /// selection over when it moves onto a row, clicks press theirs
    /// and take it too, and pages reset it to the top as they open.
    pub(crate) sel: usize,
    /// Where the pointer stood last menu frame, so motion can be
    /// told from mere presence: a parked cursor must not out-vote
    /// the arrow keys.
    pub(crate) last_pointer: Option<[f32; 2]>,
    /// The repeat clocks of held menu arrows: for each arrow still
    /// down, the time left before its next step. Written by
    /// [`Demo::nav_held`] — the first press answers at once and arms
    /// the delay; each frame held spends the clock down, arming one
    /// beat per step. A released key simply stops being paid; the
    /// next press re-arms the whole delay.
    pub(crate) nav_timer: HashMap<KeyCode, f32>,
    /// The row aim the pad held last menu frame, and its repeat
    /// clock — the pad's share of the arrows' law, kept by
    /// [`Demo::pad_nav`]: a new aim steps at once, a held one walks
    /// on `NAV_RATE` beats after the `NAV_HOLD` delay.
    pub(crate) pad_row: Option<game::Dir>,
    pub(crate) pad_repeat: f32,
    /// The slider currently under the button, if any.
    pub(crate) drag: Option<Slider>,
    /// The first hill row the progress page shows: the window that
    /// slides as the selection (or the wheel) walks past its edges.
    pub(crate) progress_top: usize,
    /// The overlay's breath: every menu — main, settings, the won
    /// hill's panel — fades through this alpha rather than snapping.
    pub(crate) alpha: f32,
    /// Which page the overlay paints: the live one while a menu is
    /// up, and the last one held while the layer fades out.
    pub(crate) page: Page,
}

/// The page the app shows: the hill itself, the menu it opens onto,
/// or the settings the menu leads to. The menu pages pause the hill —
/// no key, click, or queued step reaches the game behind them, and
/// `Resume Game` hands it back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Page {
    Game,
    Main,
    Settings,
    /// The hill is won: the splash's verdict hangs over two rows,
    /// next hill and retry hill — the only way a finished hill is
    /// ever left.
    Won,
    /// The control pages: plain lists answering the menu rows of
    /// the same names. Any row on them steps back to the menu —
    /// they are reading material, and reading ends when you press
    /// anything.
    Keyboard,
    Mouse,
    Gamepad,
    /// The ledger: one row per warm hill — number, name, best run,
    /// and a minimap of the hill itself — a list that can outgrow
    /// the plate, so the window slides with the selection and the
    /// wheel, and a row walked is a door back onto that hill.
    Progress,
}

/// Where the Escape key leads. The rule the menu keeps: from the hill
/// Escape opens the menu, from the menu Escape hands the hill back,
/// and from settings it is a plain back step. Escape never quits —
/// leaving is the exit row's (and the window button's) alone, so the
/// key that pauses is never the key that destroys.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Escape {
    ToMenu,
    Resume,
}

pub(crate) fn escape_nav(from: Page) -> Escape {
    match from {
        Page::Game => Escape::ToMenu,
        Page::Main => Escape::Resume,
        Page::Settings => Escape::ToMenu,
        Page::Keyboard | Page::Mouse | Page::Gamepad => Escape::ToMenu,
        // The ledger is read, not lived in: Escape steps off it onto
        // the menu like everywhere else.
        Page::Progress => Escape::ToMenu,
        // Escape steps off the finished hill onto the menu; the
        // resume row walks back to the won hill and its panel.
        Page::Won => Escape::ToMenu,
    }
}

/// The rows a menu page can answer to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Action {
    /// Move to the next hill — the button N used to be.
    Next,
    /// Replay the hill now underfoot from its own beginning.
    Retry,
    New,
    Resume,
    OpenSettings,
    Exit,
    Back,
    Vsync,
    Filter,
    Fullscreen,
    Quality,
    Leaves,
    /// The halo the ants throw, on and off.
    Glow,
    Shadows,
    /// Sliders: the press starts a drag, the release saves.
    Ambient,
    Light,
    /// The control pages of the menu: keyboard, mouse, gamepad.
    Keyboard,
    Mouse,
    Gamepad,
    /// The ledger page, and one of its hill rows: a hill row replays
    /// the hill it names — the ledger is a door, not a museum.
    OpenProgress,
    Hill(usize),
}

/// One laid-out menu row: a clickable rectangle (centre and half
/// extents in user space) with its text, and for sliders also the
/// track geometry and the current fraction. Input hit-tests rows and
/// paint draws the very same list — the click can never miss the
/// picture.
pub(crate) struct Row {
    pub(crate) action: Action,
    pub(crate) cx: f32,
    pub(crate) cy: f32,
    pub(crate) hw: f32,
    pub(crate) hh: f32,
    pub(crate) text: String,
    /// `Some(fraction)` marks a slider row: the track's live value.
    pub(crate) slider: Option<f32>,
}

/// The settings page's slider track, in user-space x around the panel
/// centre; the label rides left of it. Shared by layout, hit test,
/// and paint so the three can never disagree. The centre is chosen so
/// the WHOLE ink of a slider row — label's left edge to the track's
/// right — spans equally about x = 0: the label draws centred at
/// `cx - half - 160`, so its left edge sits ~380 left of the track
/// centre while the track ends 150 right of it, and 190-115 balances
/// the two. The page is then centred on the plate, not merely its
/// tracks.
pub(crate) const MENU_TRACK_CX: f32 = 115.0;
pub(crate) const MENU_TRACK_HALF: f32 = 150.0;

/// The widest the title band ever gets: the page's 52-point sign
/// draws at `top + 70` and its ink ends by `top + 96`.
pub(crate) const PLATE_TOP_PAD_MAX: f32 = 130.0;

/// The band above the first row, computed from the live view height:
/// a quarter-ish of the screen at full size, shrunk on short windows
/// so the plate's top stays inside the frame, floored where the
/// title's own ink still fits (70 + half a 52-point cap).
pub(crate) fn plate_top_pad(h: f32) -> f32 {
    (h * 0.22).clamp(96.0, PLATE_TOP_PAD_MAX)
}

/// The plate's bottom is one extra row of the page's own stride below
/// the last row — a ledger row's stride is its fat one, a plain
/// page's is its own, compression included.
pub(crate) fn plate_bottom_pad(page: Page, h: f32) -> f32 {
    match page {
        Page::Progress => progress_step(h),
        Page::Main => menu_step_for(8, h),
        Page::Settings => menu_step_for(10, h),
        Page::Keyboard => menu_step_for(6, h),
        Page::Mouse => menu_step_for(4, h),
        Page::Gamepad => menu_step_for(6, h),
        Page::Won => menu_step_for(2, h),
        Page::Game => MENU_STEP,
    }
}

/// The plain menu's row stride at its roomiest: the step every page
/// hangs by, and measures its trailing row with, until a short window
/// says otherwise — see [`menu_step_for`].
pub(crate) const MENU_STEP: f32 = 58.0;
pub(crate) const MENU_STEP_SHORT: f32 = 46.0;

/// The plain menu's stride, solved for the live view: the page's own
/// step when a plate of `n` rows — title band, the gaps between rows,
/// and the trailing row — clears the frame; compressed to the largest
/// stride that does when it does not. A row's ink is 48 tall, so the
/// solve owns a floor of 40: past that a page would rather let the
/// stone's edge be trimmed by the view than braid two rows' texts
/// together.
pub(crate) fn menu_step_for(n: usize, h: f32) -> f32 {
    let wide = if h < 720.0 {
        MENU_STEP_SHORT
    } else {
        MENU_STEP
    };
    let gaps = n.saturating_sub(1) as f32;
    let wanted = (h - 24.0 - plate_top_pad(h) - wide) / gaps;
    wide.min(wanted).max(MENU_STEP_SHORT - 6.0)
}

/// A plain page's first row: its height above the centre when `n`
/// rows hang at `step`, slid so the WHOLE composed plate — title
/// band, rows, trailing row — is symmetric about the view's centre.
/// Centring the rows alone off-centres the stone; centring the stone
/// is centring the menu.
pub(crate) fn plain_top(n: usize, step: f32, h: f32) -> f32 {
    (n.saturating_sub(1)) as f32 * step / 2.0 + (menu_step_for(n, h) - plate_top_pad(h)) / 2.0
}

/// The menu's keyboard walk: arrows wrap around the page's rows, so
/// the selection never falls off the panel and the shortest path from
/// either end to the other is one keypress of the far direction.
pub(crate) fn step_sel(sel: usize, n: usize, dir: i32) -> usize {
    if n == 0 {
        return 0;
    }
    if dir < 0 {
        (sel + n - 1) % n
    } else {
        (sel + 1) % n
    }
}

/// The repeat clock of a held menu arrow: the first press answers at
/// once, and a hand that keeps the key down waits out the delay —
/// then takes one row per beat until it lets go.
pub(crate) const NAV_HOLD: f32 = 0.32;
pub(crate) const NAV_RATE: f32 = 0.08;

/// One frame of that clock: `next` is the time still to wait, and a
/// frame spends it down; at zero the arrow steps and the next step
/// arms one beat later. A hand resting before its decision never
/// pays a step early, and a hand that holds never pays one late.
pub(crate) fn key_repeat(next: &mut f32, dt: f32) -> bool {
    *next -= dt;
    if *next <= 0.0 {
        *next = NAV_RATE;
        true
    } else {
        false
    }
}

/// The ledger's top chevron, kept: where the window must lean and
/// where the selection must land so that pressing up from the first
/// row stands on the last hill of the list. The window bottoms out
/// (`total - visible` rows down) and the selection takes the window's
/// own last hill row — which for a descent shorter than the window
/// is simply the last hill where it already stood.
pub(crate) fn wrap_to_last(total: usize, visible: usize) -> (usize, usize) {
    let top = total.saturating_sub(visible);
    (top, total.saturating_sub(1).saturating_sub(top))
}

/// Which row, if any, lives under a pointer: the one test the click,
/// the highlight-follow, and any future pointer reader all share.
pub(crate) fn row_under(rows: &[Row], p: [f32; 2]) -> Option<usize> {
    rows.iter()
        .position(|r| (p[0] - r.cx).abs() <= r.hw && (p[1] - r.cy).abs() <= r.hh)
}

/// The progress list's first-row height above the centre: the window
/// of `fits` rows plus its pinned `back` row, hung so the composed
/// plate — title band above, one extra row below — is symmetric about
/// the view's centre at every resolution.
pub(crate) fn progress_top_y(h: f32) -> f32 {
    let rows = progress_fits(h) + 1;
    (rows - 1) as f32 * progress_step(h) / 2.0 + (progress_step(h) - plate_top_pad(h)) / 2.0
}

/// The progress list's row stride: wider than the settings step,
/// because each row wears a minimap as tall as its text is not. The
/// maps own this stride — a card leaves two points of a step to
/// its neighbour, never more — and a further step is affordable
/// because the plate holds the same window at full height either way.
pub(crate) fn progress_step(h: f32) -> f32 {
    if h < 720.0 { 54.0 } else { 68.0 }
}

/// How many hill rows the centred block can hold: `n` rows plus the
/// `back` row pinned under them span from `n*step/2` down to
/// `-n*step/2 - 24`, and the block's low light must clear the plate's
/// floor at `-h/2 + 96`. Solved for `n`, floored, clamped to the
/// window's own half so a solve that has run past the floor still
/// yields one row — a ledger of one is the minimum honest list.
pub(crate) fn progress_fits(h: f32) -> usize {
    // The composed ledger: title band above, `back` row and its
    // trailing row below, centred — n is what's left for hill rows.
    let room = (h - 24.0 - plate_top_pad(h) - 2.0 * progress_step(h)).max(0.0);
    ((room / progress_step(h)).floor() as usize).max(1)
}

/// The progress page's plate half-width and its column edges, settled
/// once against measured ink rather than guesswork: every width below
/// was laid out by the engine's own text shaper in the shipped
/// Barrfont at the ledger's size (30, weight 500). The widest authored
/// hill name — "Three Cold Nights" — is 370.0, and since
/// [`MOVES_CAP`] guarantees no score wider than "999 moves" at 199.2,
/// the name column gets a real budget (372.0) and the score column is
/// right-aligned, so long names and long scores can never reach each
/// other. The map card caps at 235.0 wide by the cell's own limits.
/// Every gutter below is at least 20 points, and
/// `the_ledger_columns_never_touch` holds all of it: a wider name
/// than the budget, or a shrunken plate, fails the test instead of
/// painting a collision.
pub(crate) const PROGRESS_PLATE: f32 = 473.0;
/// The hill-number column's left edge.
pub(crate) const PROGRESS_NUM_X: f32 = -459.0;
/// The name column's left edge: a 20-point gutter past the widest
/// two-digit hill number.
pub(crate) const PROGRESS_NAME_X: f32 = -391.0;
/// The score column's right edge — scores right-align, so the name
/// column always owns its full budget.
pub(crate) const PROGRESS_SCORE_R: f32 = 206.0;
/// The map column's centre: a 20-point gutter past the score's right
/// edge, plus the widest possible card.
pub(crate) const PROGRESS_MAP_CX: f32 = 344.0;

/// Whether the ledger shows its wrap chevrons. It always does, top
/// and bottom: they no longer answer "is something hidden?" — they
/// promise "the other end is one keypress away", and the arrows keep
/// that promise from any edge by wrapping to the last or first hill
/// of the descent. Barrfont carries no arrow glyph, so the marks are
/// drawn chevrons. A descent of one hill has nothing to wrap to and
/// spends no mark.
pub(crate) fn progress_marks(total: usize) -> (bool, bool) {
    let one = total <= 1;
    (!one, !one)
}

/// A dragged slider, while the left button holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Slider {
    Ambient,
    Light,
}

/// Where on a track a pointer falls, as 0..=1: clamped at both ends,
/// so the knob lives on the track and a click past its tip sets the
/// tip.
pub(crate) fn slide_value(x: f32, track_cx: f32, track_half: f32) -> f32 {
    ((x - (track_cx - track_half)) / (2.0 * track_half)).clamp(0.0, 1.0)
}

impl Demo {
    /// The menu itself. The hill dims behind it, paused and visible —
    /// the point of Resume is that you can see what you are taking
    /// back — and the hovered row lifts a whisper of white.
    pub(crate) fn draw_menu(&self, ctx: &mut Context) {
        let (w, h) = self.view(ctx);
        let z = UI + 10.0;
        // Everything the overlay owns breathes together — veil,
        // plate, rows, verdict — through one alpha held by `process`.
        let a = self.menu.alpha;
        let fade = |c: Color| Color { a: c.a * a, ..c };
        ctx.rectangle(
            0.0,
            0.0,
            w / 2.0,
            h / 2.0,
            Color {
                r: 0.012,
                g: 0.014,
                b: 0.022,
                a: 0.72 * a,
            },
            z,
        );
        let rows = self.menu_rows(self.menu.page, h);
        if rows.is_empty() {
            return;
        }
        let top = rows.iter().map(|r| r.cy).fold(f32::MIN, f32::max);
        let bot = rows.iter().map(|r| r.cy).fold(f32::MAX, f32::min);
        // The plate is as wide as its page's widest claim: the ledger
        // spends a number, a name, a score, and a map, so it takes
        // the widest hall of any page on the menu.
        let plate_hw = match self.menu.page {
            Page::Progress => PROGRESS_PLATE,
            Page::Settings | Page::Keyboard | Page::Mouse => 380.0,
            _ => 300.0,
        };
        // The panel is cut from the slab: one placement of the whole
        // stone at the plate's own size — stretched to whatever
        // rectangle the page hangs, and the moss and lichen do not
        // mind. Where the slab failed to decode, the plate is still
        // the plain ink slab it always was.
        // The plate is deliberately unequal: the title's band above
        // (`PLATE_TOP_PAD`) and one whole extra row of the page's own
        // stride below the last row — a ledger row's stride is its
        // fat one, the plain pages' is the menu step. Centre and
        // half-height both shift by the difference.
        let bottom_pad = plate_bottom_pad(self.menu.page, h);
        let top_pad = plate_top_pad(h);
        let pcx = 0.0;
        let pcy = (top + bot) / 2.0 + (top_pad - bottom_pad) / 2.0;
        let (phw, phh) = (plate_hw, (top - bot) / 2.0 + (top_pad + bottom_pad) / 2.0);
        let mut cut = false;
        if let Some(slab) = &self.slab {
            let mut panel = slab.clone();
            if let frost::Shape::TileMap { tiles, .. } = &mut panel {
                tiles.push(frost::Tile::new(
                    [pcx, pcy],
                    [2.0 * phw, 2.0 * phh],
                    [0.0, 0.0, 1.0, 1.0],
                ));
                ctx.scene().root.children.push(Box::new(SceneNode {
                    order: z + 1.0,
                    modulate: Color {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a,
                    },
                    shape: Some(panel),
                    ..Default::default()
                }));
                cut = true;
            }
        }
        // The panel's own night: once it was the plate itself, and on
        // stone it is a shade — the slab was baked at a third of its
        // daylight, and all this shade has left to do is settle the
        // white lichen behind the words.
        ctx.rectangle(
            pcx,
            pcy,
            phw,
            phh,
            Color {
                r: 0.05,
                g: 0.055,
                b: 0.08,
                a: (if cut { 0.45 } else { 0.94 }) * a,
            },
            z + 1.25,
        );
        // The panel wears the hill it stands on: main and won pages
        // both name their ground — the title answers the same banner
        // the player has been reading all run. Settings is its own
        // room and keeps its own sign.
        let title: String = match self.menu.page {
            Page::Settings => "SETTINGS".to_string(),
            Page::Keyboard => "KEYBOARD".to_string(),
            Page::Mouse => "MOUSE".to_string(),
            Page::Gamepad => "GAMEPAD".to_string(),
            Page::Progress => "PROGRESS".to_string(),
            _ => self
                .game
                .as_ref()
                .map_or("ANTIFROST", |g| g.level.name.as_str())
                .to_string(),
        };
        ctx.text(
            0.0,
            top + 70.0,
            &self.font,
            title,
            52.0,
            700.0,
            fade(C_TEXT),
            z + 2.0,
        );
        // The won hill's verdict: the word its splash just spoke,
        // now read plainly in the strip between the HUD's band and
        // the plate's top — verdict overhead, choices under foot.
        if self.menu.page == Page::Won
            && let Some((word, colour)) = &self.verdict
        {
            // The plate's top and the score banner's bottom are
            // 92 units apart by the won page's hang (see
            // `menu_rows`); the word owns the middle of that
            // strip, full size, every window height.
            let plate_top = top + plate_top_pad(h);
            let band = h / 2.0 - 93.0;
            ctx.text(
                0.0,
                (plate_top + band) / 2.0,
                &self.font,
                word.clone(),
                64.0,
                700.0,
                fade(*colour),
                z + 2.0,
            );
        }
        // Exactly one row wears the highlight: the selected one. The
        // pointer takes the selection over when it moves onto a row
        // (see `menu_input`), so the mouse and the keys can never
        // each light their own row — the two-emission state a separate
        // hover used to allow.
        for (i, r) in rows.iter().enumerate() {
            let hot = i == self.menu.sel;
            // A row with no road left in it: `next` greys when the
            // hill underfoot is all the descent still owes — the
            // same rule the key obeys, asked at the same lip. A
            // locked hill needs no grey: it is not on the board.
            let mute = matches!(r.action, Action::Next) && !self.next_owed();
            if hot {
                ctx.rectangle(
                    r.cx,
                    r.cy,
                    r.hw,
                    24.0,
                    Color {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: (if mute { 0.03 } else { 0.07 }) * a,
                    },
                    z + 2.0,
                );
            }
            if let Action::Hill(hill) = r.action {
                self.draw_ledger_row(ctx, r, hill, hot, z, a);
                continue;
            }
            match r.slider {
                None => ctx.text(
                    r.cx,
                    r.cy - 10.0,
                    &self.font,
                    r.text.clone(),
                    30.0,
                    500.0,
                    if mute {
                        fade(C_MUTE)
                    } else if hot {
                        fade(C_TEXT)
                    } else {
                        fade(C_DIM)
                    },
                    z + 3.0,
                ),
                Some(frac) => {
                    ctx.text(
                        MENU_TRACK_CX - MENU_TRACK_HALF - 160.0,
                        r.cy - 10.0,
                        &self.font,
                        r.text.clone(),
                        30.0,
                        500.0,
                        if hot { fade(C_TEXT) } else { fade(C_DIM) },
                        z + 3.0,
                    );
                    ctx.rectangle(
                        MENU_TRACK_CX,
                        r.cy,
                        MENU_TRACK_HALF,
                        3.0,
                        Color {
                            r: 0.35,
                            g: 0.36,
                            b: 0.42,
                            a,
                        },
                        z + 3.0,
                    );
                    ctx.rectangle(
                        MENU_TRACK_CX - MENU_TRACK_HALF + frac * MENU_TRACK_HALF,
                        r.cy,
                        (frac * MENU_TRACK_HALF).max(0.5),
                        3.0,
                        fade(C_NOTE),
                        z + 3.0,
                    );
                    ctx.rectangle(
                        MENU_TRACK_CX - MENU_TRACK_HALF + frac * 2.0 * MENU_TRACK_HALF,
                        r.cy,
                        5.0,
                        11.0,
                        fade(C_TEXT),
                        z + 3.0,
                    );
                }
            }
        }
        // The maps beside the ledger's rows, in one instanced node:
        // every cell of every visible hill is a tinted placement of
        // one white pixel, and the node's clip cuts the column to
        // the plate — scroll and all of it rides together.
        if self.menu.page == Page::Progress {
            self.draw_minimaps(ctx, &rows, h, z, a);
            self.draw_progress_marks(ctx, &rows, h, z, a);
        }
    }

    /// One ledger row: the hill's number, its name, and — for a hill
    /// the colony warmed — its best run in the par's own colour
    /// language (the yellow a met par wears, the red a missed one).
    /// A hill stood on but not finished keeps its score column
    /// empty: the ledger writes nothing it has not been given. The
    /// runs sit in columns rather than the HUD's one centred
    /// sentence: this is a list to scan, not a banner to read.
    pub(crate) fn draw_ledger_row(
        &self,
        ctx: &mut Context,
        r: &Row,
        hill: usize,
        hot: bool,
        z: f32,
        a: f32,
    ) {
        let score = self
            .progress
            .scores
            .iter()
            .find(|(h, _)| *h == hill)
            .map(|(_, m)| *m);
        let (name, par) = self
            .levels
            .get(hill)
            .map_or(("hill", 0), |l| (l.name.as_str(), l.par));
        // Measured first, drawn after: the columns are left edges,
        // and text draws centred, so every run needs its own width
        // before any of it reaches the canvas.
        let num = (hill + 1).to_string();
        let w_num = ctx.text_width(&self.font, &num, 30.0, 500.0);
        let w_name = ctx.text_width(&self.font, name, 30.0, 500.0);
        let fade = |c: Color| Color { a: c.a * a, ..c };
        let ink = if hot { fade(C_TEXT) } else { fade(C_DIM) };
        let cy = r.cy - 10.0;
        ctx.text(
            PROGRESS_NUM_X + w_num / 2.0,
            cy,
            &self.font,
            num,
            30.0,
            500.0,
            ink,
            z + 3.0,
        );
        ctx.text(
            PROGRESS_NAME_X + w_name / 2.0,
            cy,
            &self.font,
            name,
            30.0,
            500.0,
            ink,
            z + 3.0,
        );
        let Some(moves) = score else { return };
        // The score wears its judgement always and never the
        // highlight's: the best run at or under par — perfect — is
        // yellow, a best that still overshot par is red, the same
        // full hue on every row lit or unlit. The page's own fade
        // is the only hand on either. Right-aligned at the column's
        // edge — a list of numbers reads up its right side.
        let colour = if par > 0 && moves > par {
            C_PAR_MISSED
        } else {
            C_PAR_MET
        };
        // A perfect run wears its number plain; a best that overshot
        // par reads its debt — moves owed against the best known.
        let score_text = if par > 0 && moves > par {
            format!("{moves} of {par}")
        } else {
            format!("{moves} moves")
        };
        let w_score = ctx.text_width(&self.font, &score_text, 30.0, 500.0);
        ctx.text(
            PROGRESS_SCORE_R - w_score / 2.0,
            cy,
            &self.font,
            score_text,
            30.0,
            500.0,
            fade(colour),
            z + 3.0,
        );
    }

    /// Every minimap the page shows, in one tile-map node: a one-
    /// pixel white atlas tinted per cell — rock, dirt, open tunnel,
    /// and every bed in its ant's own colour. The maps are the
    /// authored hills themselves, tile for tile: the ledger keeps no
    /// picture, because the hill already is one.
    pub(crate) fn draw_minimaps(&self, ctx: &mut Context, rows: &[Row], h: f32, z: f32, a: f32) {
        let mut map = frost::Shape::TileMap {
            data: self.minimap_atlas.clone(),
            width: 1,
            height: 1,
            // One atlas for the game's whole life, behind one Arc: the
            // texture cache's (pointer, generation) identity never
            // changes, so the pixel uploads once and never again.
            generation: 0,
            filter: SpriteFilter::Nearest,
            tiles: Vec::new(),
            clip: Some([
                PROGRESS_MAP_CX - 126.0,
                -h * 0.5 + 70.0,
                PROGRESS_MAP_CX + 126.0,
                progress_top_y(h) + 46.0,
            ]),
            color: Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
        };
        for r in rows {
            let Action::Hill(hill) = r.action else {
                continue;
            };
            let Some(level) = self.levels.get(hill) else {
                continue;
            };
            let (cols, rows_n) = (level.cols as f32, level.rows as f32);
            // The cell answers to both bounds: the map is at most
            // 225 wide — that budget is what holds the card and the
            // column gutters honest — and never taller than the row
            // stride minus its breathing room, so maps of neighbouring
            // rows cannot touch even at the short-window stride.
            let cell = ((progress_step(h) - 12.0) / rows_n)
                .min(225.0 / cols)
                .min(5.6);
            // The card centres on the row itself — the selection band
            // spans ±24 about `r.cy` — not on the row's text, which
            // sits ten low for its baseline. An earlier draft stole
            // four of those points for the baseline too, and the maps
            // read a touch sunken in their rows.
            let (cx, cy) = (PROGRESS_MAP_CX, r.cy);
            // The dark card first, so the tiles land on it: a map
            // reads as a framed thing beside its row, not as spill.
            ctx.rectangle(
                cx,
                cy,
                cols * cell / 2.0 + 5.0,
                rows_n * cell / 2.0 + 5.0,
                Color {
                    r: 0.02,
                    g: 0.022,
                    b: 0.03,
                    a: 0.9 * a,
                },
                z + 1.2,
            );
            for (i, t) in level.tiles.iter().enumerate() {
                let (row, col) = (i / level.cols, i % level.cols);
                let tint = match t {
                    level::Tile::Rock => Color {
                        r: 0.30,
                        g: 0.27,
                        b: 0.25,
                        a: 1.0,
                    },
                    level::Tile::Dirt => Color {
                        r: 0.46,
                        g: 0.34,
                        b: 0.22,
                        a: 1.0,
                    },
                    level::Tile::Tunnel => Color {
                        r: 0.05,
                        g: 0.05,
                        b: 0.06,
                        a: 1.0,
                    },
                    level::Tile::Bed(c) => ant_color(*c),
                };
                let mut tile = frost::Tile::new(
                    [
                        cx + (col as f32 - (cols - 1.0) / 2.0) * cell,
                        cy + ((rows_n - 1.0) / 2.0 - row as f32) * cell,
                    ],
                    [cell, cell],
                    [0.0, 0.0, 1.0, 1.0],
                );
                tile.color = tint;
                map.push_tile(tile);
            }
        }
        ctx.scene().root.children.push(Box::new(SceneNode {
            order: z + 1.5,
            modulate: Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a,
            },
            shape: Some(map),
            ..Default::default()
        }));
    }

    /// The marks that make the descent a ring: a chevron above the
    /// first row and one below the last, always lit, each promising
    /// the far end of the ledger one press away (they share the gap
    /// the pinned `back` row lives in). Barrfont carries letters and
    /// a space and nothing else, so the arrows are drawn as two short
    /// strokes meeting at a tip — each one a polyline node, riding the
    /// page's fade. They sit in the inter-row gaps, where no text
    /// reaches: a row's glyphs stand 25 off its centre and a mark 11
    /// off its own, and the gaps are wider than both.
    pub(crate) fn draw_progress_marks(
        &self,
        ctx: &mut Context,
        rows: &[Row],
        h: f32,
        z: f32,
        a: f32,
    ) {
        let hills = rows
            .iter()
            .filter(|r| matches!(r.action, Action::Hill(_)))
            .count();
        let (up, down) = progress_marks(self.progress.unlocked.min(self.levels.len()));
        if hills == 0 || (!up && !down) {
            return;
        }
        let chevron = |cy: f32, tip: f32, ctx: &mut Context| {
            ctx.scene().root.children.push(Box::new(SceneNode {
                order: z + 3.0,
                modulate: Color {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a,
                },
                shape: Some(frost::Shape::Polyline {
                    points: vec![[-15.0, -tip * 0.55], [0.0, tip], [15.0, -tip * 0.55]],
                    width: 5.0,
                    color: C_TEXT,
                }),
                transform: Transform::translate([0.0, cy]),
                ..Default::default()
            }));
        };
        let top = progress_top_y(h);
        let step = progress_step(h);
        if up {
            chevron(top + step * 0.42, 8.0, ctx);
        }
        if down {
            chevron(top - (hills as f32 - 0.5) * step, -8.0, ctx);
        }
    }

    /// A row's answer: the page rows turn the page, the setting rows
    /// flip the engine the moment they move, and everything the disc
    /// should remember is written before the click is even over —
    /// except the sliders, which save on release.
    pub(crate) fn menu_action(&mut self, ctx: &mut Context, action: Action) {
        match action {
            Action::New => {
                self.new_game();
                self.progress.save();
            }
            Action::Resume => self.page = Page::Game,
            Action::OpenSettings => {
                self.page = Page::Settings;
                self.menu.drag = None;
                self.menu.sel = 0;
            }
            Action::OpenProgress => {
                self.page = Page::Progress;
                self.menu.sel = 0;
                self.menu.progress_top = 0;
            }
            Action::Next => {
                // The greyed row answers nothing, whichever key or
                // click called it: the hill underfoot is all the
                // descent still owes, and `next` leads nowhere new.
                if !self.next_owed() {
                    return;
                }
                // Not the next hill number — the next owed hill: the
                // freshly unlocked one at the frontier, else the
                // nearest without a perfect run. `next_hill` says it
                // at length; this is the same key's short hand.
                let next = self.next_hill();
                self.load(next);
                self.page = Page::Game;
                // Standing on a new hill is the fact the progress
                // file keeps — written the moment it's true.
                self.progress.stood(self.idx);
                self.progress.save();
            }
            Action::Retry => {
                self.load(self.idx);
                self.page = Page::Game;
            }
            Action::Hill(n) => {
                // A locked door answers to nobody: the row shows the
                // hill, the frontier keeps it shut.
                if n >= self.progress.unlocked {
                    return;
                }
                // A ledger row is a door: walking back into a warmed
                // hill is a hill moved, and the file says so.
                self.load(n);
                self.page = Page::Game;
                self.progress.stood(self.idx);
                self.progress.save();
            }
            Action::Exit => ctx.exit(),
            Action::Keyboard => {
                self.page = Page::Keyboard;
                self.menu.sel = 0;
            }
            Action::Mouse => {
                self.page = Page::Mouse;
                self.menu.sel = 0;
            }
            Action::Gamepad => {
                self.page = Page::Gamepad;
                self.menu.sel = 0;
            }
            Action::Back => {
                self.page = Page::Main;
                self.menu.sel = 0;
            }
            Action::Vsync => {
                self.settings.vsync = !self.settings.vsync;
                ctx.set_vsync(self.settings.vsync);
            }
            Action::Filter => {
                self.settings.nearest = !self.settings.nearest;
                ctx.set_blit_filter(if self.settings.nearest {
                    SpriteFilter::Nearest
                } else {
                    SpriteFilter::Linear
                });
            }
            Action::Fullscreen => {
                self.settings.fullscreen = !self.settings.fullscreen;
                self.apply_window_policy(ctx);
            }
            Action::Quality => {
                self.settings.quality = self.settings.quality.next();
                ctx.set_render_size(self.settings.quality.size());
                self.apply_window_policy(ctx);
            }
            Action::Leaves => self.settings.leaves_lit = !self.settings.leaves_lit,
            Action::Glow => self.settings.lamp_glow = !self.settings.lamp_glow,
            Action::Shadows => self.settings.shadows = self.settings.shadows.next(),
            Action::Ambient | Action::Light => return,
        }
        self.settings.save();
    }

    /// The menu's input pass. A slider owns the button from the press
    /// on the track until it lifts — dragging previews live, and only
    /// the release writes the disc. Any other row answers its press at
    /// once.
    pub(crate) fn menu_input(&mut self, ctx: &mut Context, dt: f32) {
        let pointer = ctx.mouse_position();
        if let Some(sl) = self.menu.drag {
            if ctx.mouse_button_down(MouseButton::Left) {
                if let Some([x, _]) = pointer {
                    let frac = slide_value(x, MENU_TRACK_CX, MENU_TRACK_HALF);
                    match sl {
                        Slider::Ambient => self.settings.ambient = frac,
                        Slider::Light => self.settings.light = frac * 2.0,
                    }
                }
            } else {
                self.menu.drag = None;
                self.settings.save();
            }
        }
        // The keyboard's turn: arrows walk the rows — and a held
        // arrow keeps walking, one row per beat after a first delay —
        // Enter presses the highlighted one, and left/right nudge a
        // slider by five points when the highlight stands on its row.
        // The same `menu_rows` list the mouse reads, so the two never
        // diverge.
        let h = self.view(ctx).1;
        let rows = self.menu_rows(self.page, h);
        if !rows.is_empty() {
            self.menu.sel = self.menu.sel.min(rows.len() - 1);
            // The ledger leans: at its window's edges the arrow keys
            // slide the window over the hills instead of wrapping —
            // and the wheel drags the window directly, lines for
            // lines. Past the true ends the two standing chevrons
            // keep their promise: up from the first hill wraps to
            // the last, down off the bottom comes home to the first.
            // Every other page keeps the plain walk.
            let ku = self.nav_held(ctx, dt, KeyCode::ArrowUp);
            let kd = self.nav_held(ctx, dt, KeyCode::ArrowDown);
            // The pad's stick and d-pad walk the same rows the
            // arrows walk — same wrap, same ledger lean, because
            // their steps arrive as the same two booleans.
            let (pu, pd, pl, pr) = self.pad_nav(ctx, dt);
            let up = ku || pu;
            let down = kd || pd;
            if self.page == Page::Progress {
                let hills = rows.len() - 1; // the pinned back row aside
                let more = self.levels.len() > self.menu.progress_top + hills;
                if up && self.menu.sel == 0 && self.menu.progress_top > 0 {
                    self.menu.progress_top -= 1;
                } else if down && self.menu.sel + 1 == hills && more {
                    self.menu.progress_top += 1;
                } else if up && self.menu.sel == 0 {
                    // The top chevron's promise: the last hill of
                    // the ledger — a hill, not the back row — and the
                    // window leans so that hill stands in view.
                    let (top, sel) = wrap_to_last(self.levels.len(), hills);
                    self.menu.progress_top = top;
                    self.menu.sel = sel;
                } else if down && self.menu.sel + 1 == rows.len() {
                    // And the bottom chevron's: home to the first
                    // hill, window and all.
                    self.menu.progress_top = 0;
                    self.menu.sel = 0;
                } else {
                    // The plain walk between the ends: within the
                    // window the arrows do what they do everywhere
                    // else, and from the last visible hill the walk
                    // steps down onto the pinned back row.
                    if up {
                        self.menu.sel = step_sel(self.menu.sel, rows.len(), -1);
                    }
                    if down {
                        self.menu.sel = step_sel(self.menu.sel, rows.len(), 1);
                    }
                }
            } else {
                if up {
                    self.menu.sel = step_sel(self.menu.sel, rows.len(), -1);
                }
                if down {
                    self.menu.sel = step_sel(self.menu.sel, rows.len(), 1);
                }
            }
            if self.page == Page::Progress {
                let wheel = ctx.mouse_wheel();
                if wheel != 0.0 {
                    let last = self.progressed().saturating_sub(progress_fits(h));
                    let top = self.menu.progress_top as i32 - wheel.round() as i32;
                    self.menu.progress_top = top.clamp(0, last as i32) as usize;
                }
            }
            let nudge = match (
                self.pressed(ctx, KeyCode::ArrowLeft) || pl,
                self.pressed(ctx, KeyCode::ArrowRight) || pr,
            ) {
                (true, false) => Some(-1.0),
                (false, true) => Some(1.0),
                _ => None,
            };
            if let Some(dir) = nudge
                && rows[self.menu.sel].slider.is_some()
            {
                match rows[self.menu.sel].action {
                    Action::Ambient => {
                        self.settings.ambient = (self.settings.ambient + dir * 0.05).clamp(0.0, 1.0)
                    }
                    _ => self.settings.light = (self.settings.light + dir * 0.10).clamp(0.0, 2.0),
                }
                self.settings.save();
            }
            if self.pressed(ctx, KeyCode::Enter)
                || self.pressed(ctx, KeyCode::NumpadEnter)
                || self.pad_pressed(ctx, frost::Button::South)
            {
                let action = rows[self.menu.sel].action;
                self.menu_action(ctx, action);
            }
            // The finished hill answers `R` directly: it is the key
            // the hill already takes mid-play as its restart, so a
            // win that wants an immediate redo needs no walk to the
            // row — the verdict plate is still up, the hill hits its
            // beginning on the same key that clears it.
            if self.page == Page::Won && self.pressed(ctx, KeyCode::KeyR) {
                self.menu_action(ctx, Action::Retry);
            }
            // Tab turns the highlighted ledger row over: its recorded
            // best run replays itself at the click walker's pace.
            // A hill without a recording stays as it was — that row
            // is a door to playing, not yet to showing.
            if self.page == Page::Progress
                && self.pressed(ctx, KeyCode::Tab)
                && let Some(&Action::Hill(h)) = rows.get(self.menu.sel).map(|r| &r.action)
            // A locked row replays nothing: no door, no walk.
            && h < self.progress.unlocked
            {
                self.start_replay(h);
            }
        }
        // The pointer moving onto a row takes the highlight with it —
        // motion only, so a cursor parked on `resume` cannot out-vote
        // the arrow keys walking to `exit`. One row is ever lit.
        if pointer != self.menu.last_pointer
            && let Some(p) = pointer
            && let Some(i) = row_under(&rows, p)
        {
            self.menu.sel = i;
        }
        self.menu.last_pointer = pointer;
        // The mouse's turn: a click presses its row's action and takes
        // the highlight with it.
        if !self.pressed_mouse(ctx, MouseButton::Left) {
            return;
        }
        let Some(p) = pointer else { return };
        if let Some(i) = row_under(&rows, p) {
            let r = &rows[i];
            self.menu.sel = i;
            match r.action {
                Action::Ambient | Action::Light => {
                    self.menu.drag = Some(match r.action {
                        Action::Ambient => Slider::Ambient,
                        _ => Slider::Light,
                    });
                    let frac = slide_value(p[0], MENU_TRACK_CX, MENU_TRACK_HALF);
                    match r.action {
                        Action::Ambient => self.settings.ambient = frac,
                        _ => self.settings.light = frac * 2.0,
                    }
                }
                action => self.menu_action(ctx, action),
            }
        }
    }

    /// The rows of the page currently up, laid out in user space —
    /// the list the pointer hit-tests and the paint draws, so a click
    /// can never miss the picture it answers to. `h` is the window's
    /// user height: the block hangs from its top, and shrinks its
    /// stride on short windows (a 540p quality buffer still wears the
    /// whole menu).
    pub(crate) fn menu_rows(&self, page: Page, h: f32) -> Vec<Row> {
        let st = &self.settings;
        let mut rows: Vec<Row> = Vec::new();

        let push = |rows: &mut Vec<Row>, action, cy, hw, text: String, slider: Option<f32>| {
            rows.push(Row {
                action,
                cx: if slider.is_some() { MENU_TRACK_CX } else { 0.0 },
                cy,
                hw,
                hh: 24.0,
                text,
                slider,
            })
        };
        match page {
            Page::Game => {}
            Page::Main => {
                // Eight rows, hung so the composed plate — title
                // band above, one trailing row below — sits centred
                // on the view at every resolution.
                let step = menu_step_for(8, h);
                let top = plain_top(8, step, h);
                push(&mut rows, Action::New, top, 240.0, "new game".into(), None);
                push(
                    &mut rows,
                    Action::Resume,
                    top - step,
                    240.0,
                    "resume game".into(),
                    None,
                );
                push(
                    &mut rows,
                    Action::OpenSettings,
                    top - 2.0 * step,
                    240.0,
                    "settings".into(),
                    None,
                );
                push(
                    &mut rows,
                    Action::OpenProgress,
                    top - 3.0 * step,
                    240.0,
                    "progress".into(),
                    None,
                );
                push(
                    &mut rows,
                    Action::Keyboard,
                    top - 4.0 * step,
                    240.0,
                    "keyboard".into(),
                    None,
                );
                push(
                    &mut rows,
                    Action::Mouse,
                    top - 5.0 * step,
                    240.0,
                    "mouse".into(),
                    None,
                );
                push(
                    &mut rows,
                    Action::Gamepad,
                    top - 6.0 * step,
                    240.0,
                    "gamepad".into(),
                    None,
                );
                push(
                    &mut rows,
                    Action::Exit,
                    top - 7.0 * step,
                    240.0,
                    "exit".into(),
                    None,
                );
            }
            Page::Won => {
                // The won page hangs lower than its siblings: the
                // 315-unit drop leaves a constant 92-unit strip —
                // plate top to the HUD's lowest banner (the score,
                // h/2 - 78 with a 30-unit body: bottom at h/2 - 93)
                // — and the verdict, at its full 64, is centred in
                // that strip at any window height. The old 120-unit
                // hang made the strip negative at 720p, and the
                // word lay across both banners.
                let top = (h * 0.5 - 315.0).min(120.0);
                let step = menu_step_for(2, h);
                push(
                    &mut rows,
                    Action::Next,
                    top,
                    240.0,
                    "next hill".into(),
                    None,
                );
                push(
                    &mut rows,
                    Action::Retry,
                    top - step,
                    240.0,
                    "retry hill".into(),
                    None,
                );
            }
            Page::Settings => {
                // Ten rows now that the glow has a rung of its own —
                // composed-plate centred like every other page.
                let step = menu_step_for(10, h);
                let top = plain_top(10, step, h);
                let on = |b: bool| if b { "on" } else { "off" };
                push(
                    &mut rows,
                    Action::Vsync,
                    top,
                    250.0,
                    format!("vsync   {}", on(st.vsync)),
                    None,
                );
                push(
                    &mut rows,
                    Action::Filter,
                    top - step,
                    250.0,
                    format!(
                        "filter   {}",
                        if st.nearest { "nearest" } else { "bilinear" }
                    ),
                    None,
                );
                push(
                    &mut rows,
                    Action::Fullscreen,
                    top - 2.0 * step,
                    250.0,
                    format!("fullscreen   {}", on(st.fullscreen)),
                    None,
                );
                push(
                    &mut rows,
                    Action::Quality,
                    top - 3.0 * step,
                    250.0,
                    format!("resolution   {}", st.quality.label()),
                    None,
                );
                push(
                    &mut rows,
                    Action::Ambient,
                    top - 4.0 * step,
                    MENU_TRACK_HALF + 12.0,
                    format!("ambient   {}", (st.ambient * 100.0).round() as i32),
                    Some(st.ambient),
                );
                push(
                    &mut rows,
                    Action::Light,
                    top - 5.0 * step,
                    MENU_TRACK_HALF + 12.0,
                    format!("light fx   {}", (st.light * 50.0).round() as i32),
                    Some(st.light / 2.0),
                );
                push(
                    &mut rows,
                    Action::Glow,
                    top - 6.0 * step,
                    250.0,
                    format!("lamp glow   {}", on(st.lamp_glow)),
                    None,
                );
                push(
                    &mut rows,
                    Action::Leaves,
                    top - 7.0 * step,
                    250.0,
                    format!("leaf lighting   {}", on(st.leaves_lit)),
                    None,
                );
                push(
                    &mut rows,
                    Action::Shadows,
                    top - 8.0 * step,
                    250.0,
                    format!("lamp shadows   {}", st.shadows.label()),
                    None,
                );
                push(
                    &mut rows,
                    Action::Back,
                    top - 9.0 * step,
                    250.0,
                    "back".into(),
                    None,
                );
            }
            Page::Keyboard => {
                // Lists the keyboard's whole vocabulary. Every row is
                // the way out: these pages are read, not used.
                let step = menu_step_for(6, h);
                let top = plain_top(6, step, h);
                for (i, line) in [
                    "wasd or arrows to move",
                    "space to detach",
                    "shift wasd or arrows to push",
                    "z to undo",
                    "r to reset level",
                    "back",
                ]
                .into_iter()
                .enumerate()
                {
                    push(
                        &mut rows,
                        Action::Back,
                        top - i as f32 * step,
                        280.0,
                        line.to_string(),
                        None,
                    );
                }
            }
            Page::Mouse => {
                let step = menu_step_for(4, h);
                let top = plain_top(4, step, h);
                for (i, line) in [
                    "left click to move",
                    "right click to detach",
                    "shift click to push",
                    "back",
                ]
                .into_iter()
                .enumerate()
                {
                    push(
                        &mut rows,
                        Action::Back,
                        top - i as f32 * step,
                        280.0,
                        line.to_string(),
                        None,
                    );
                }
            }
            Page::Gamepad => {
                // The pad's whole vocabulary, in the same voice as
                // the other control pages — and in the same font-safe
                // words: Barrfont owns letters, digits and spaces,
                // so the dpad, the trigger and the DualSense's own
                // names keep their shorthand.
                let step = menu_step_for(6, h);
                let top = plain_top(6, step, h);
                for (i, line) in [
                    "left stick or dpad to move",
                    "r1 or r2 with a direction to push",
                    "square detaches",
                    "l1 to undo",
                    "l2 to restart",
                    "back",
                ]
                .into_iter()
                .enumerate()
                {
                    push(
                        &mut rows,
                        Action::Back,
                        top - i as f32 * step,
                        280.0,
                        line.to_string(),
                        None,
                    );
                }
            }
            Page::Progress => {
                // The descent, cut to its window and ordered by the
                // work it still owes: hills not yet perfected come
                // first in hill order, the perfected ones follow in
                // hill order, so the top of the list is always the
                // run still to earn and the clean sweeps settle to
                // the bottom. A hill stood on but not warmed owns no
                // run — and no run is no perfect run — so it leads
                // with the imperfect. `perfect` is the exact
                // negation of the score's red, so a hill's place in
                // the order and its colour can never disagree. The
                // `back` row stays pinned under the last visible one,
                // where it can never scroll out of reach; the window
                // and the scroll marks count rows, and a reorder
                // moves hills between them, never a row out.
                let top = progress_top_y(h);
                let pstep = progress_step(h);
                // The board shows the descent the player owns: the
                // open hills and no others. A hill past the frontier
                // is not on the board at all — the ledger is ground
                // one can walk, not a census of the hill list. A
                // hill with no run wears no score — the column stays
                // empty until a run is earned — and a best that
                // overshot par reads its debt plainly.
                // The descent shown is the descent open: the window,
                // the marks, and the rows all count unlocked hills.
                let total = self.progress.unlocked.min(self.levels.len());
                let mut order: Vec<usize> = (0..total).collect();
                order.sort_by_key(|&i| (self.perfect(i), i));
                let start = self.menu.progress_top.min(total);
                let n = progress_fits(h).min(total - start);
                for (row, &i) in order[start..start + n].iter().enumerate() {
                    let name = self.levels.get(i).map_or("hill", |l| l.name.as_str());
                    let score = match self.progress.scores.iter().find(|(h, _)| *h == i) {
                        Some((_, moves)) => {
                            let par = self.levels.get(i).map_or(0, |l| l.par);
                            if par > 0 && *moves > par {
                                format!("   {moves} of {par}")
                            } else {
                                format!("   {moves} moves")
                            }
                        }
                        None => String::new(),
                    };
                    push(
                        &mut rows,
                        Action::Hill(i),
                        top - row as f32 * pstep,
                        PROGRESS_PLATE - 12.0,
                        format!("{name}{score}"),
                        None,
                    );
                }
                push(
                    &mut rows,
                    Action::Back,
                    top - n as f32 * pstep,
                    240.0,
                    "back".to_string(),
                    None,
                );
            }
        }
        rows
    }
}

/// The menu, the Escape rule, and the settings file — the parts the
/// request asked for that are pure CPU and can be nailed down without
/// a window.
#[cfg(test)]
mod menu_tests {
    use super::*;

    fn demo() -> Demo {
        let frames = ANT_FRAMES
            .iter()
            .map(|png| ant_frame(png).expect("shipped ant frames bake"))
            .collect();
        let stencils = ANT_STENCILS
            .iter()
            .map(|png| ant_frame(png).expect("shipped ant stencils bake"))
            .collect();
        let soil = frost::Shape::sprite_bytes(SOIL).expect("shipped soil bakes");
        Demo::new(
            frames,
            stencils,
            soil,
            Settings::default(),
            // The test player has walked everything: doors open as
            // far as the hill list, so ledger tests read the ledger
            // and not the lock.
            Progress {
                unlocked: usize::MAX,
                ..Default::default()
            },
        )
    }

    #[test]
    fn the_progress_file_counts_hills_as_the_hud_does() {
        assert_eq!(
            Progress::default().text(),
            "(\n    hill: 1,\n    reached: 1,\n    unlocked: 0,\n    scores: [],\n    routes: [],\n)\n",
            "a player who never moved stands on the first hill, ledger empty"
        );
        let p = Progress::parse("(\n    hill: 3,\n)\n");
        assert_eq!(p.hill, 2, "the file counts from one, the code from zero");
        assert_eq!(p.reached, 2, "a pre-depth file adopts its standing hill");
        assert!(
            p.scores.is_empty(),
            "a file from before the ledger is no scores, not a broken file"
        );
        assert!(
            p.routes.is_empty(),
            "a file from before the routes is no routes either"
        );
        assert_eq!(
            Progress {
                hill: 1,
                reached: 1,
                ..Default::default()
            }
            .text(),
            "(\n    hill: 2,\n    reached: 2,\n    unlocked: 0,\n    scores: [],\n    routes: [],\n)\n"
        );
        assert_eq!(
            Progress::parse("garbage").hill,
            0,
            "a broken file means the first hill, not a failed launch"
        );
        assert_eq!(
            Progress::parse("(\n    hill: 0,\n)\n").hill,
            0,
            "nothing sits below the first hill"
        );
    }

    #[test]
    fn the_ledger_keeps_the_best_run_of_every_warm_hill() {
        let mut p = Progress::default();
        p.record(2, 40);
        p.record(0, 55);
        p.record(2, 34);
        p.record(2, 41);
        assert_eq!(
            p.scores,
            vec![(0, 55), (2, 34)],
            "sorted, one entry per hill, the fewest moves kept"
        );
        let back = Progress::parse(&p.text());
        assert_eq!(back, p, "the ledger round-trips through the file");
        // And it reads back a hand-written list the same way it reads
        // its own: quoted pairs counting hills from one, out-of-order
        // folds to sorted, a hill named twice keeps its better number.
        let p =
            Progress::parse("(\n    hill: 4,\n    scores: [\"3 31\", \"1 47\", \"3 29\"],\n)\n");
        assert_eq!(p.scores, vec![(0, 47), (2, 29)]);
    }

    #[test]
    fn warmth_opens_the_descent_one_hill_at_a_time() {
        let mut p = Progress::default();
        assert_eq!(p.unlocked, 0, "an empty ledger opens nothing by itself");
        p.record(0, 30);
        assert_eq!(p.unlocked, 2, "the first hill won opens the door below");
        p.record(0, 25);
        assert_eq!(p.unlocked, 2, "warming a hill twice opens no extra doors");
        p.record(4, 40);
        assert_eq!(
            p.unlocked, 6,
            "the frontier answers to warmth and never retreats"
        );
        let back = Progress::parse(&p.text());
        assert_eq!(back.unlocked, 6, "the lock survives the file");
        assert_eq!(
            Progress::parse("(\n    hill: 3,\n)\n").unlocked,
            usize::MAX,
            "a file from before the lock keeps the freedom it was written in"
        );
    }

    #[test]
    fn new_game_locks_the_descent_behind_its_school() {
        let mut d = demo();
        d.progress.record_run(9, 20, "");
        assert!(
            d.progress.unlocked > TUTORIAL_HILLS,
            "a deep player owns deep doors"
        );
        d.new_game();
        assert_eq!(
            d.progress.unlocked, TUTORIAL_HILLS,
            "new game leaves the teaching hills open and slams the descent shut"
        );
        assert!(
            d.progress.scores.is_empty(),
            "and the fresh ledger keeps no numbers for the walk ahead"
        );
    }

    #[test]
    fn a_locked_hill_is_not_on_the_board_at_all() {
        let mut d = demo();
        d.progress.record(3, 21);
        d.progress.unlocked = 2;
        let rows = d.menu_rows(Page::Progress, 720.0);
        assert!(
            rows.iter().any(|r| r.action == Action::Hill(1)),
            "an open hill holds its row"
        );
        assert!(
            !rows
                .iter()
                .any(|r| matches!(r.action, Action::Hill(i) if i >= 2)),
            "the board shows the open descent and nothing past it —
             no locked rows, no census of the hill list"
        );
        // The score of the hidden hill is not deleted, only unseen:
        // the lock hides the door, the ledger keeps the number.
        assert_eq!(
            d.progress.scores,
            vec![(3, 21)],
            "hiding a hill hides its row, not its history"
        );
    }

    #[test]
    fn a_route_is_walkable_or_it_is_nothing() {
        let run = [
            game::Act::Move(game::Dir::Left),
            game::Act::Move(game::Dir::Down),
            game::Act::Push(game::Dir::Up),
            game::Act::Detach,
        ];
        let glyphs = encode_run(&run);
        assert_eq!(
            glyphs, "ldUx",
            "walks lowercase, shoves uppercase, leave-behind x"
        );
        assert_eq!(decode_run(&glyphs).as_deref(), Some(&run[..]));
        assert_eq!(
            decode_run("ldU!x"),
            None,
            "one stray letter is not a route with a typo — it is not a route"
        );
        // And the ledger keeps only walks: a hand may write a number,
        // but it cannot plant a route the hill could never show.
        let mut p = Progress::default();
        p.remember(0, "qzzling");
        assert!(
            p.routes.is_empty(),
            "unwalkable glyphs are refused at the door"
        );
    }

    #[test]
    fn the_walk_changes_only_when_the_score_does() {
        let mut p = Progress::default();
        p.record_run(1, 50, "rrrddd");
        assert_eq!(
            p.route_of(1),
            Some("rrrddd"),
            "a first win deposits number and walk"
        );
        p.record_run(1, 52, "ddd");
        assert_eq!(p.scores, vec![(1, 50)], "a worse run moves no number");
        assert_eq!(
            p.route_of(1),
            Some("rrrddd"),
            "and carries no walk anywhere"
        );
        p.record_run(1, 50, "uuu");
        assert_eq!(
            p.route_of(1),
            Some("rrrddd"),
            "a tie over a kept walk leaves the elder in"
        );
        p.record_run(1, 44, "rrdd");
        assert_eq!(
            p.route_of(1),
            Some("rrdd"),
            "a better score carries a new walk in"
        );
        // A hill whose number predates the recordings backfills its
        // walk on a tie — and only on a tie, because the walk that
        // joins the ledger must walk exactly the count it joins.
        let mut old = Progress::parse("(\n    hill: 2,\n    scores: [\"2 40\"],\n)\n");
        assert_eq!(
            old.route_of(1),
            None,
            "the old file has scores and no walks"
        );
        old.record_run(1, 41, "ldu");
        assert_eq!(
            old.route_of(1),
            None,
            "a run worse than the old number records nothing"
        );
        old.record_run(1, 40, "ruru");
        assert_eq!(old.route_of(1), Some("ruru"), "the tie backfills the walk");
        assert_eq!(old.scores, vec![(1, 40)], "and the number does not move");
        old.record_run(1, 40, "dud");
        assert_eq!(
            old.route_of(1),
            Some("ruru"),
            "the elder backfill keeps its seat"
        );
        let back = Progress::parse(&old.text());
        assert_eq!(
            back, old,
            "numbers and routes round-trip through the file together"
        );
    }

    #[test]
    fn a_showing_walks_its_recorded_run_and_scores_nothing() {
        let mut d = demo();
        d.load(0);
        // A hand walks a short run on First Warm, exactly as the keys
        // would: right, right, up from the start cell (1,3).
        {
            let g = d.game.as_mut().expect("a hill stands loaded");
            for a in [
                game::Act::Move(game::Dir::Right),
                game::Act::Move(game::Dir::Right),
                game::Act::Move(game::Dir::Up),
            ] {
                g.act(a).expect("legal on First Warm");
            }
        }
        let walked = d.game.as_ref().expect("still loaded");
        assert_eq!(walked.log.len(), 3, "the played hill wrote its walk down");
        d.progress.remember(0, &encode_run(&walked.log));
        let scores_before = d.progress.scores.clone();
        // Now the ledger shows it back.
        d.start_replay(0);
        assert_eq!(d.page, Page::Game, "the showing opens on the hill");
        assert!(
            !d.game.as_ref().expect("replaying").record,
            "a showing records nothing"
        );
        for _ in 0..3 {
            d.replay_advance();
        }
        let shown = d.game.as_ref().expect("still replaying");
        assert_eq!(
            shown.player,
            game::Cell::new(3, 2),
            "the walk lands where it was walked"
        );
        assert_eq!(shown.moves, 3);
        assert_eq!(d.replay.as_ref().expect("shown").shown, 3);
        assert!(
            !d.replay.as_ref().expect("arrived").playing,
            "the run holds at its end"
        );
        // And the wheel runs backwards exactly: the hill rebuilt from
        // its beginning, one action short.
        d.replay_retreat();
        let back = d.game.as_ref().expect("still replaying");
        assert_eq!(
            back.player,
            game::Cell::new(3, 3),
            "one step back, one cell back"
        );
        assert_eq!(back.moves, 2);
        // The audience scored nothing: the ledger never shifted, and
        // the won page never claimed the hill.
        assert_eq!(d.progress.scores, scores_before, "a showing is not a run");
        assert_eq!(d.page, Page::Game, "no verdict for a spectator");
    }

    #[test]
    fn the_game_opens_where_the_player_stood() {
        let frames = ANT_FRAMES
            .iter()
            .map(|png| ant_frame(png).expect("shipped ant frames bake"))
            .collect();
        let stencils = ANT_STENCILS
            .iter()
            .map(|png| ant_frame(png).expect("shipped ant stencils bake"))
            .collect();
        let soil = frost::Shape::sprite_bytes(SOIL).expect("shipped soil bakes");
        let d = Demo::new(
            frames,
            stencils,
            soil,
            Settings::default(),
            Progress {
                hill: 1,
                ..Default::default()
            },
        );
        assert_eq!(d.idx, 1, "the saved hill is the opened hill");
        assert!(d.game.is_some(), "and it arrives with its game loaded");
        let mut d = d;
        d.new_game();
        assert_eq!(d.idx, 0, "New Game restarts the descent at the first hill");
    }

    #[test]
    fn one_row_lives_under_the_pointer() {
        let d = demo();
        let rows = d.menu_rows(d.page, 1080.0);
        for (i, r) in rows.iter().enumerate() {
            assert_eq!(
                row_under(&rows, [r.cx, r.cy]),
                Some(i),
                "a row's own center answers to it alone"
            );
        }
        let above = [rows[0].cx, rows[0].cy - rows[0].hh - 1.0];
        assert_eq!(
            row_under(&rows, above),
            None,
            "just clear of the top row is nobody's"
        );
    }

    #[test]
    fn escape_walks_the_pages() {
        // The hill yields to the menu, the menu hands the hill back,
        // settings step back — the key never quits, only the exit row
        // does.
        assert_eq!(escape_nav(Page::Game), Escape::ToMenu);
        assert_eq!(escape_nav(Page::Main), Escape::Resume);
        assert_eq!(escape_nav(Page::Settings), Escape::ToMenu);
    }

    #[test]
    fn the_hill_opens_paused_behind_the_menu() {
        let d = demo();
        assert_eq!(d.page, Page::Main, "launch is the menu, not the walk");
        assert!(d.game.is_some(), "the hill is loaded and visible behind it");
    }

    #[test]
    fn new_game_clears_the_moves() {
        let mut d = demo();
        {
            let g = d.game.as_mut().expect("a hill is loaded");
            // The click tests' First Warm: the queen walks the shaft up.
            assert!(g.move_player(game::Dir::Up).is_ok());
            assert_eq!(g.moves, 1, "one step on the board");
        }
        d.new_game();
        assert_eq!(
            d.game.as_ref().unwrap().moves,
            0,
            "the board forgot the step"
        );
        assert_eq!(d.page, Page::Game, "and the hill plays on");
    }

    #[test]
    fn new_game_restores_the_empty_ledger() {
        // The menu's New Game row rewinds the save to its empty
        // default: every number, walk and depth the ledger ever
        // kept is forgotten, and the descent restarts its record
        // from the first hill. The settings keep their own file and
        // are never part of the rewind.
        let mut d = demo();
        d.progress.record_run(3, 40, "rrrddd");
        d.progress.stood(2);
        assert!(
            !d.progress.scores.is_empty() && d.progress.reached > 0,
            "there must be a history to erase"
        );
        d.new_game();
        assert_eq!(
            d.progress,
            Progress {
                unlocked: TUTORIAL_HILLS,
                ..Default::default()
            },
            "the save returns to its empty default — open only as far as its school"
        );
        assert_eq!(d.idx, 0, "the descent restarts at the first hill");
    }

    #[test]
    fn the_first_hill_lessons_its_walk_card_by_card() {
        let mut d = demo();
        d.page = Page::Game;
        let script: Vec<game::Act> = FIRST_WARM_LESSON.iter().map(|l| l.act).collect();
        assert_eq!(
            d.lesson().map(|l| l.act),
            script.first().copied(),
            "the hill opens on its first card"
        );
        for (i, a) in script.iter().enumerate() {
            d.game
                .as_mut()
                .expect("the hill stands loaded")
                .act(*a)
                .expect("the lesson's walk is legal");
            let next = d.lesson().map(|l| l.act);
            if i + 1 < script.len() {
                assert_eq!(
                    next,
                    script.get(i + 1).copied(),
                    "card {i} hands over the next"
                );
            } else {
                assert_eq!(next, None, "the won hill keeps no card");
            }
        }
    }

    #[test]
    fn the_second_hill_lessons_the_straight_hall() {
        let mut d = demo();
        d.page = Page::Game;
        d.load(1);
        let script: Vec<game::Act> = ATTUNEMENT_LESSON.iter().map(|l| l.act).collect();
        assert_eq!(
            d.lesson().map(|l| l.act),
            script.first().copied(),
            "the hall opens on its first card"
        );
        for (i, a) in script.iter().enumerate() {
            assert!(d.rail_opens(*a), "the hall's rail asks card {i}");
            assert!(
                !d.rail_opens(game::Act::Detach),
                "and nothing else: card {i} admits the hall's one verb"
            );
            d.game
                .as_mut()
                .expect("the hall stands loaded")
                .act(*a)
                .expect("the hall's walk is legal");
            if i == 1 {
                // The lesson's own point, caught in the act: the
                // collected ant sleeps on R in the queen's wake, no
                // steps to spare.
                assert_eq!(
                    d.game.as_ref().expect("still walking").ants_asleep(),
                    1,
                    "the ant fell asleep on R in the wake"
                );
            }
        }
        let g = d.game.as_ref().expect("still loaded");
        assert!(g.won, "five steps east and the hall is warm");
        assert!(d.lesson().is_none(), "a warmed hall keeps no card");
        assert!(
            d.rail_opens(game::Act::Detach),
            "and lifts its rail with the lesson"
        );
    }

    #[test]
    fn the_third_hill_lessons_the_dunk() {
        let mut d = demo();
        d.page = Page::Game;
        // The dunk lesson is the hill's lesson after the cold tour;
        // a warm hill — however it was warmed — opens on these cards.
        d.progress.record(2, 40);
        d.load(2);
        let script: Vec<game::Act> = FIRST_COLD_LESSON.iter().map(|l| l.act).collect();
        assert_eq!(
            d.lesson().map(|l| l.act),
            script.first().copied(),
            "the cold ring opens on its first card"
        );
        for (i, a) in script.iter().enumerate() {
            assert!(d.rail_opens(*a), "the ring's rail asks card {i}");
            if *a != game::Act::Detach {
                // The leave-behind answers only when the card asks
                // for it: the verb is taught, not lying about loose.
                assert!(
                    !d.rail_opens(game::Act::Detach),
                    "card {i} gates the leave-behind"
                );
            }
            d.game
                .as_mut()
                .expect("the ring stands loaded")
                .act(*a)
                .expect("the ring's walk is legal");
            if i == 4 {
                assert_eq!(
                    d.game.as_ref().expect("still walking").ants_asleep(),
                    1,
                    "the second shove dunks r onto R"
                );
            }
        }
        let g = d.game.as_ref().expect("still loaded");
        assert!(g.won, "nine acts and the cold ring is warm");
        assert!(d.lesson().is_none(), "a warmed ring keeps no card");
        assert!(
            d.rail_opens(game::Act::Detach),
            "and lifts its rail with the lesson"
        );
    }

    #[test]
    fn the_oops_deck_forces_both_mistakes_and_teaches_the_restart() {
        let mut d = demo();
        d.page = Page::Game;
        d.load(2);
        // First Cold opens on the wrong foot, on purpose: the card
        // asks for the wrong step, and nothing else answers the
        // gate.
        assert_eq!(
            d.lesson().map(|l| l.act),
            Some(MISTAKE_CARD.act),
            "the ring opens on a mistake, asked for out loud"
        );
        assert!(!d.rail_opens(game::Act::Move(game::Dir::Left)));
        // The step lands and the hill sulks: no card is open, and
        // the rail answers nothing — not even the step straight
        // back — until the undo comes.
        d.game
            .as_mut()
            .expect("the ring stands loaded")
            .act(game::Act::Move(game::Dir::Right))
            .expect("the forced step is still a legal one");
        assert!(d.mistake_standing(), "the mistake is the whole walk");
        assert!(d.lesson().is_none(), "a sulking hill keeps no card");
        assert!(!d.rail_opens(game::Act::Move(game::Dir::Left)));
        assert!(!d.rail_opens(game::Act::Move(game::Dir::Right)));
        assert!(!d.rail_opens(game::Act::Detach));
        // The Z takes the step back — no bill, moves and record
        // rolled under — and the hill re-baits the very same trap:
        // the wrong step is asked again, and this time it is asked
        // twice.
        d.mistake_shown = true;
        let g = d.game.as_mut().expect("still sulking");
        assert!(g.undo());
        assert_eq!(g.moves, 0, "the mistake leaves no bill");
        assert_eq!(
            d.lesson().map(|l| l.act),
            Some(MISTAKE_CARD.act),
            "the same trap, re-baited"
        );
        let g = d.game.as_mut().expect("re-baited");
        g.act(game::Act::Move(game::Dir::Right))
            .expect("the second first step is legal");
        assert_eq!(
            d.lesson().map(|l| l.act),
            Some(DOUBLE_CARD.act),
            "and the pair asks its second step"
        );
        let g = d.game.as_mut().expect("mid-pair");
        g.act(game::Act::Move(game::Dir::Right))
            .expect("the second wrong step is legal too");
        // The deep sulk: the pair stands, no card answers, and the
        // rail is shut as tight as before — this time for the R.
        assert!(d.disaster_standing(), "twice right is twice wrong");
        assert!(d.lesson().is_none(), "the deep sulk keeps no card");
        assert!(!d.rail_opens(game::Act::Move(game::Dir::Left)));
        assert!(!d.rail_opens(game::Act::Move(game::Dir::Right)));
        assert!(!d.rail_opens(game::Act::Detach));
        // The R — what the restart handler retires, this test
        // mirrors by hand — restarts the hill onto the cold tour,
        // at its own first card, never re-baited again; the
        // answer's words stay unspent until the tour earns them.
        d.oops_retired = true;
        let level = d.game.as_ref().expect("disastered").level.clone();
        d.game = Some(game::Game::start(level));
        assert!(!d.oopsy_visit(), "a retired deck never re-baits");
        assert_eq!(
            d.lesson().map(|l| l.act),
            Some(COLD_TOUR_LESSON[0].act),
            "the restart lands on the long walk, as promised"
        );
        assert!(d.rail_opens(COLD_TOUR_LESSON[0].act));
    }

    #[test]
    fn the_cold_tour_walks_the_long_way_before_the_lesson() {
        let mut d = demo();
        d.page = Page::Game;
        d.load(2);
        // The oops deck had its whole turn first: both mistakes
        // made, the restart restarted out of them — the deck's own
        // test walks that above.
        d.mistake_shown = true;
        d.oops_retired = true;
        let script: Vec<game::Act> = COLD_TOUR_LESSON.iter().map(|l| l.act).collect();
        assert_eq!(
            d.lesson().map(|l| l.act),
            script.first().copied(),
            "the cold ring opens on its long way"
        );
        for (i, a) in script.iter().enumerate() {
            assert!(d.rail_opens(*a), "the rail asks tour card {i}");
            d.game
                .as_mut()
                .expect("the ring stands loaded")
                .act(*a)
                .expect("the tour's walk is legal");
            if i == 4 {
                assert_eq!(
                    d.game.as_ref().expect("still walking").ants_asleep(),
                    1,
                    "the wake carries r home while the walk goes on"
                );
            }
        }
        let (won, moves) = {
            let g = d.game.as_ref().expect("still walking");
            (g.won, g.moves)
        };
        assert!(won, "the long way is still a way");
        assert_eq!(moves, 18, "eighteen moves where nine will do");
        // And the tour's ledger is the lesson's opening: the update
        // loop records this same walk and restarts the hill on the
        // win tick; a warm hill answers only to the better script.
        d.progress.record(2, moves);
        d.load(2);
        d.game
            .as_mut()
            .expect("reloaded")
            .act(game::Act::Move(game::Dir::Left))
            .expect("both decks open with the same grab");
        assert_eq!(
            d.lesson().map(|l| l.act),
            Some(game::Act::Detach),
            "the warmed hill teaches the leave-behind, not the walk"
        );
    }

    #[test]
    fn the_dig_tour_walks_the_long_way_then_digs_the_short() {
        let mut d = demo();
        d.page = Page::Game;
        d.load(3);
        let tour: Vec<game::Act> = DIG_TOUR_LESSON.iter().map(|l| l.act).collect();
        assert_eq!(
            d.lesson().map(|l| l.act),
            tour.first().copied(),
            "the cheap dig opens on its long way"
        );
        for (i, a) in tour.iter().enumerate() {
            assert!(d.rail_opens(*a), "the rail asks tour card {i}");
            d.game
                .as_mut()
                .expect("the dig stands loaded")
                .act(*a)
                .expect("the tour's walk is legal");
        }
        let (won, moves, dirt) = {
            let g = d.game.as_ref().expect("still walking");
            (g.won, g.moves, g.tile(game::Cell::new(5, 2)))
        };
        assert!(won, "the long way is still a way");
        assert_eq!(moves, 14, "fourteen moves where ten will do");
        assert!(
            matches!(dirt, level::Tile::Dirt),
            "the long walk misses the dirt entirely"
        );
        // Warm now, the hill re-deals on the cheap script — and
        // this time the wall's dirt cell is the lesson.
        d.progress.record(3, moves);
        d.load(3);
        let dig: Vec<game::Act> = DIG_LESSON.iter().map(|l| l.act).collect();
        assert_eq!(
            d.lesson().map(|l| l.act),
            dig.first().copied(),
            "the warmed hill teaches the dig"
        );
        for (i, a) in dig.iter().enumerate() {
            assert!(d.rail_opens(*a), "the rail asks dig card {i}");
            d.game
                .as_mut()
                .expect("the dig stands warm")
                .act(*a)
                .expect("the dig's walk is legal");
        }
        let (won, moves, gave_way) = {
            let g = d.game.as_ref().expect("still digging");
            (g.won, g.moves, g.tile(game::Cell::new(5, 2)))
        };
        assert!(won, "the dig wins");
        assert_eq!(moves, 10, "ten moves, every one of them needed");
        assert!(
            matches!(gave_way, level::Tile::Tunnel),
            "walking into dirt was the dig"
        );
        assert!(d.lesson().is_none(), "the lesson ends with the walk");
    }

    #[test]
    fn a_step_off_the_lesson_silences_it_and_undo_calls_it_back() {
        let mut d = demo();
        d.page = Page::Game;
        assert!(d.lesson().is_some(), "the first hill opens with a card");
        // The first card says right; a walk up is legal and off the
        // line, and the hill keeps its own counsel about it.
        d.game
            .as_mut()
            .expect("a hill is loaded")
            .act(game::Act::Move(game::Dir::Up))
            .expect("the shaft above the start is open");
        assert!(d.lesson().is_none(), "off the script, silence");
        d.game.as_mut().expect("still loaded").undo();
        assert_eq!(
            d.lesson().map(|l| l.act),
            Some(game::Act::Move(game::Dir::Right)),
            "the undone step puts the card back"
        );
    }

    #[test]
    fn no_other_hill_and_no_other_page_keeps_a_lesson_open() {
        let mut d = demo();
        d.page = Page::Game;
        d.load(4);
        assert!(d.lesson().is_none(), "the fifth hill teaches nothing");
        d.load(3);
        assert!(
            d.lesson().is_some(),
            "the fourth hill keeps its cards since the swap"
        );
        d.load(2);
        assert!(d.lesson().is_some(), "the third hill keeps its cards too");
        d.load(0);
        assert!(d.lesson().is_some(), "back on the first, the card is up");
        d.page = Page::Main;
        assert!(d.lesson().is_none(), "behind the menu the lesson waits");
    }

    #[test]
    fn the_pool_hands_on_no_shadow_listeners() {
        // The deep parallax sheet listens on the shadow layer from
        // the pool's last slot, and a bigger hill hands that very
        // node to another role. Fitting the pool must hand the
        // listener back to plain layer 0 — a stale one would draw the
        // hill's tile shadows onto an ant shell or a bed, surfaces
        // the hill shadows nothing.
        let mut root = SceneNode::default();
        fit_pool(&mut root, 3);
        root.children[2].lit_layers = LightLayers::default() | BG_SHADOW_LAYER;
        fit_pool(&mut root, 5);
        assert!(
            root.children[..5]
                .iter()
                .all(|n| n.lit_layers == LightLayers::default()),
            "the grown pool carries no leftover listener"
        );
        // And the sweep asks nothing of the slot: a listener planted
        // anywhere in the pool answers to the default after a fit,
        // whatever the pool's new size.
        root.children[0].lit_layers = BG_SHADOW_LAYER;
        fit_pool(&mut root, 2);
        assert_eq!(root.children.len(), 2, "the fit hands back the size");
        assert_eq!(
            root.children[0].lit_layers,
            LightLayers::default(),
            "the shrunken pool carries no leftover listener either"
        );
    }

    #[test]
    fn the_wash_takes_the_light_back_at_the_threshold() {
        // The bug this pays for: a dig wore its fx at a tail slot,
        // the next hill counts smaller, and the backdrop now dresses
        // that very slot — shape, colour, light — while `emission`
        // and `glow` stay from the dig and read as sky on fire.
        let rest = Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        };
        let mut root = SceneNode::default();
        fit_pool(&mut root, 4);
        root.children[3].emission = Color {
            r: 1.0,
            g: 0.4,
            b: 0.14,
            a: 1.0,
        };
        root.children[3].glow = Some(Color {
            r: 0.9,
            g: 0.6,
            b: 0.2,
            a: 1.0,
        });
        // A hill swap — smaller this time — slides the tail down over
        // the worn slot; the wash is what the draw spends per swap.
        fit_pool(&mut root, 3);
        wash_pool(&mut root);
        for (i, n) in root.children.iter().enumerate() {
            assert_eq!(n.emission, rest, "slot {i} still holds emission");
            assert!(n.glow.is_none(), "slot {i} still wears a halo");
        }
        // And the wash asks nothing else of a dressed node: the
        // backdrop's own colour and depth survive the threshold,
        // because every other field belongs to its band, not the
        // pool's memory.
        root.children[2].modulate = Color {
            r: 0.45,
            g: 0.4,
            b: 0.38,
            a: 1.0,
        };
        root.children[2].order = -0.5;
        root.children[2].lit = true;
        wash_pool(&mut root);
        assert_eq!(root.children[2].order, -0.5, "the wash touches no dress");
        assert!(root.children[2].lit, "the wash touches no dress");
        assert_eq!(
            root.children[2].modulate.a, 1.0,
            "the wash touches no dress"
        );
        // A grown pool needs no wash to be clean — new slots are born
        // at rest — but a wash spends nothing on the pretence.
        fit_pool(&mut root, 6);
        assert_eq!(root.children[5].emission, rest);
        assert_eq!(root.children[5].glow, None);
    }

    #[test]
    fn the_tutorial_hill_answers_to_the_card_alone() {
        let mut d = demo();
        d.page = Page::Game;
        // The first card walks right; nothing else opens the gate.
        for a in [
            game::Act::Move(game::Dir::Left),
            game::Act::Move(game::Dir::Up),
            game::Act::Move(game::Dir::Down),
            game::Act::Push(game::Dir::Right),
            game::Act::Detach,
        ] {
            assert!(
                !d.rail_opens(a),
                "the rail refuses {a:?} while the card says right"
            );
        }
        assert!(d.rail_opens(game::Act::Move(game::Dir::Right)));
        // The gate follows the cards: a log walked through the
        // script is asked each act in its turn — and once the hill
        // sleeps, the gate lifts with the lesson.
        for l in &FIRST_WARM_LESSON {
            assert!(d.rail_opens(l.act), "the open card admits its own act");
            d.game
                .as_mut()
                .expect("the hill stands loaded")
                .act(l.act)
                .expect("the script is legal");
        }
        assert!(d.rail_opens(game::Act::Detach), "a won hill keeps no rail");
    }

    #[test]
    fn the_tutorial_rail_walks_its_click_route_by_the_card() {
        let mut d = demo();
        d.page = Page::Game;
        // The open card walks right; a click route that says left
        // dies at the gate — no hop, no turn, and the hill says
        // where to look.
        d.route = vec![game::Dir::Left];
        d.walk_the_route();
        assert!(d.route.is_empty(), "the off-card route is cut");
        assert!(d.spin.is_none(), "and it never even began to turn");
        assert_eq!(
            d.game.as_ref().expect("a hill is loaded").log.len(),
            0,
            "no step was taken"
        );
        assert_eq!(d.note, RAIL_NOTE, "the gate answered with its word");
        // The card's own step walks.
        d.note.clear();
        d.note_t = 0.0;
        d.route = vec![game::Dir::Right];
        d.walk_the_route();
        let g = d.game.as_ref().expect("still loaded");
        assert_eq!(g.log.len(), 1, "the card's step was taken");
        assert_eq!(g.log[0], game::Act::Move(game::Dir::Right));
        assert!(d.note.is_empty(), "a walked card needs no word");
        // And the other hills keep no rail: every verb answers.
        d.load(4);
        assert!(
            d.rail_opens(game::Act::Detach) && d.rail_opens(game::Act::Move(game::Dir::Down)),
            "the other forty-six answer to everything"
        );
    }

    #[test]
    fn settings_round_trip_through_their_own_text() {
        let s = Settings {
            vsync: false,
            nearest: true,
            fullscreen: false,
            quality: Quality::Ultra,
            ambient: 0.25,
            light: 1.75,
            lamp_glow: false,
            leaves_lit: false,
            shadows: Shadow::Four,
        };
        assert_eq!(Settings::parse(&s.text()), s, "what is written is read");
    }

    #[test]
    fn a_broken_file_is_answered_with_defaults() {
        assert_eq!(
            Settings::parse("not ron at all ((("),
            Settings::default(),
            "a garbage file must not out-vote the launch"
        );
        // A half-known file keeps what it knows, defaults the rest,
        // and clamps what wandered off its range.
        let s = Settings::parse("(vsync: false, ambient: 9.0, bogus: 1)");
        assert!(!s.vsync, "one line survived");
        assert_eq!(s.ambient, 1.0, "clamped back onto the slider's top");
        assert_eq!(s.quality, Quality::High, "the untouched rows stand");
    }

    #[test]
    fn the_ambient_slider_hinges_on_the_shipped_night() {
        // The middle of the track — the default — is exactly the
        // night the game shipped with; the bottom is the full dark;
        // the top leaves no dimming at all.
        assert_eq!(
            Settings::default().ambient,
            0.5,
            "the slider starts on the shipped look"
        );
        let mid = ambient_floor(0.5);
        assert_eq!((mid.r, mid.g, mid.b), (C_NIGHT.r, C_NIGHT.g, C_NIGHT.b));
        let dark = ambient_floor(0.0);
        assert_eq!((dark.r, dark.g, dark.b), (0.0, 0.0, 0.0));
        let day = ambient_floor(1.0);
        assert!(
            (day.r - 1.0).abs() < 1e-6 && (day.g - 1.0).abs() < 1e-6 && (day.b - 1.0).abs() < 1e-6,
            "at full track the floor is white: nothing dims"
        );
    }

    #[test]
    fn quality_tiers_round_trip() {
        for q in [
            Quality::Medium,
            Quality::High,
            Quality::VeryHigh,
            Quality::Ultra,
        ] {
            assert_eq!(
                Quality::parse(q.label()),
                Some(q),
                "the file speaks in labels"
            );
        }
        assert_eq!(
            Quality::High.next(),
            Quality::VeryHigh,
            "the shipped tier steps up"
        );
        assert_eq!(Quality::Ultra.next(), Quality::Medium, "and the row cycles");
        assert_eq!(
            Quality::Ultra.size(),
            None,
            "native is no fixed buffer at all"
        );
        assert_eq!(
            Quality::Medium.size(),
            Some([1280, 720]),
            "720p is the floor"
        );
        assert_eq!(Quality::parse("bogus"), None, "unknown words name no tier");
        let fresh = Settings::parse("(\n    resolution: \"1920 x 1080\",\n)\n");
        assert_eq!(
            fresh.quality,
            Quality::High,
            "the file speaks in resolutions"
        );
    }
    #[test]
    fn a_slide_clamps_to_its_track() {
        assert_eq!(slide_value(-999.0, MENU_TRACK_CX, MENU_TRACK_HALF), 0.0);
        assert_eq!(slide_value(999.0, MENU_TRACK_CX, MENU_TRACK_HALF), 1.0);
        let mid = slide_value(MENU_TRACK_CX, MENU_TRACK_CX, MENU_TRACK_HALF);
        assert!((mid - 0.5).abs() < 1e-6, "the middle of the track is half");
    }

    #[test]
    fn the_selection_walks_wraps_the_page() {
        // Arrows wrap: one step down from the last row is the first,
        // one step up from the first is the last, and an empty page
        // holds still.
        assert_eq!(step_sel(0, 4, 1), 1);
        assert_eq!(step_sel(3, 4, 1), 0, "past the bottom wraps to the top");
        assert_eq!(step_sel(0, 4, -1), 3, "past the top wraps to the bottom");
        assert_eq!(step_sel(2, 4, -1), 1);
        assert_eq!(step_sel(7, 0, 1), 0, "no rows, no movement");
    }

    #[test]
    fn the_control_pages_list_the_controls_in_font_safe_words() {
        let mut d = demo();
        for (page, firsts) in [
            (
                Page::Keyboard,
                &[
                    "wasd or arrows to move",
                    "space to detach",
                    "shift wasd or arrows to push",
                    "z to undo",
                    "r to reset level",
                ][..],
            ),
            (
                Page::Mouse,
                &[
                    "left click to move",
                    "right click to detach",
                    "shift click to push",
                ][..],
            ),
            (
                Page::Gamepad,
                &[
                    "left stick or dpad to move",
                    "r1 or r2 with a direction to push",
                    "square detaches",
                    "l1 to undo",
                    "l2 to restart",
                ][..],
            ),
        ] {
            d.page = page;
            let rows = d.menu_rows(d.page, 720.0);
            let texts: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
            for want in firsts {
                assert!(texts.contains(want), "{page:?} must list {want:?}");
            }
            assert_eq!(texts.last(), Some(&"back"), "every list ends the way out");
            for t in texts {
                assert!(
                    t.chars().all(|c| c.is_ascii_alphanumeric() || c == ' '),
                    "{t:?} must wear only glyphs the font owns"
                );
            }
        }
    }

    #[test]
    fn escape_leaves_the_control_pages_for_the_menu() {
        assert_eq!(escape_nav(Page::Keyboard), Escape::ToMenu);
        assert_eq!(escape_nav(Page::Mouse), Escape::ToMenu);
        assert_eq!(escape_nav(Page::Gamepad), Escape::ToMenu);
        assert_eq!(escape_nav(Page::Progress), Escape::ToMenu);
    }

    #[test]
    fn the_plate_centres_itself_at_every_height() {
        // The composed plate — title band, rows, trailing row — must
        // hang symmetric about the view's centre whatever the height,
        // and the whole menu block must clear the view at any height
        // the game will actually run (540 and up).
        for h in [540.0, 720.0, 1080.0, 2160.0] {
            let d = demo();
            for page in [
                Page::Main,
                Page::Settings,
                Page::Keyboard,
                Page::Mouse,
                Page::Gamepad,
            ] {
                let rows = d.menu_rows(page, h);
                let top = rows.iter().map(|r| r.cy).fold(f32::MIN, f32::max);
                let bot = rows.iter().map(|r| r.cy).fold(f32::MAX, f32::min);
                let plate_top = top + plate_top_pad(h);
                let plate_bot = bot - plate_bottom_pad(page, h);
                assert!(
                    (plate_top + plate_bot).abs() < 0.01,
                    "{page:?} at {h}: the plate hangs off-centre: {plate_top} / {plate_bot}"
                );
                let lowest_ink = bot - 24.0;
                let highest_ink = top + plate_top_pad(h).min(96.0);
                assert!(
                    plate_bot.min(lowest_ink) >= -h / 2.0 && highest_ink <= h / 2.0,
                    "{page:?} at {h}: ink escaped the view"
                );
            }
        }
    }

    /// The ledger's columns, held apart by measured ink: these four
    /// budgets are the real widths of the widest things a row can
    /// carry, laid out by the engine's shaper in the shipped Barrfont
    /// at the ledger's size — the longest authored name (370.0,
    /// "Three Cold Nights"), the fullest score the moves cap allows
    /// (199.2, "999 moves"), a two-digit hill number (46.3), and the
    /// card that the cell's own caps can never grow past (190.0). If a
    /// new hill out-owns this ink, the first assertion fails here
    /// rather than painting a collision.
    #[test]
    #[allow(clippy::assertions_on_constants)] // constant-on-constant is the point: these are layout guards over `const` widths, and the test earns its keep the day one of them moves
    fn the_ledger_columns_never_touch() {
        const NUM: f32 = 48.0;
        const NAME: f32 = 372.0;
        const SCORE: f32 = 205.0;
        const CARD: f32 = 235.0;
        assert!(
            PROGRESS_NUM_X + NUM + 20.0 <= PROGRESS_NAME_X,
            "the number column must hand off to the name with room"
        );
        assert!(
            PROGRESS_NAME_X + NAME + 20.0 <= PROGRESS_SCORE_R - SCORE,
            "the widest name must stop short of the shortest score's left edge"
        );
        assert!(
            PROGRESS_SCORE_R + 20.0 <= PROGRESS_MAP_CX - CARD / 2.0,
            "the score must stop short of the map card"
        );
        assert!(
            PROGRESS_MAP_CX + CARD / 2.0 + 10.0 <= PROGRESS_PLATE,
            "the map card must sit inside the plate"
        );
        assert!(
            PROGRESS_PLATE <= 480.0,
            "the plate must fit the half-width of the smallest window the game lays out for"
        );
    }

    #[test]
    fn the_wrap_chevrons_stand_at_both_ends() {
        // The chevrons no longer ask whether something hides beyond
        // the window — they promise the far end is one press away —
        // so they light on every descent with a far end to reach.
        assert_eq!(
            progress_marks(1),
            (false, false),
            "a hill of one has no other end"
        );
        assert_eq!(
            progress_marks(2),
            (true, true),
            "two hills promise each other"
        );
        assert_eq!(progress_marks(20), (true, true), "at any depth");
        // The top chevron's landing: the last hill of the ledger, and
        // the window leans so that hill stands in view — bottom row
        // of a window bottomed out.
        assert_eq!(wrap_to_last(20, 12), (8, 11));
        // A descent shallower than the window leans nowhere: the last
        // hill already stands.
        assert_eq!(wrap_to_last(3, 12), (0, 2));
        // And an empty ledger stands still.
        assert_eq!(wrap_to_last(0, 12), (0, 0));
    }

    #[test]
    fn a_held_arrow_waits_then_walks() {
        // The first repeat waits out the whole delay — a hand still
        // travelling to the key pays no steps — ...
        let mut left = NAV_HOLD;
        let mut frames = 0;
        while !key_repeat(&mut left, 1.0 / 60.0) {
            frames += 1;
            assert!(frames < 600, "the delay never let go");
        }
        assert!(
            (frames as f32 + 1.0) / 60.0 >= NAV_HOLD,
            "the first repeat cheated the delay: {frames} frames"
        );
        // ...and then the clock walks, one step per beat, no beat
        // wider than the rate by more than a frame's rounding.
        for beat in 1..6 {
            let mut gap = 0;
            while !key_repeat(&mut left, 1.0 / 60.0) {
                gap += 1;
                assert!(gap < 60, "beat {beat} never came");
            }
            let span = (gap + 1) as f32 / 60.0;
            assert!(
                (NAV_RATE..=NAV_RATE + 2.0 / 60.0).contains(&span),
                "beat {beat} came at {span}"
            );
        }
    }

    #[test]
    fn the_ledger_never_writes_more_than_the_cap() {
        let mut p = Progress::default();
        p.record(0, 1234);
        assert_eq!(
            p.scores,
            vec![(0, MOVES_CAP)],
            "a wandering win records the cap, not the wander"
        );
        p.record(0, 940);
        assert_eq!(p.scores, vec![(0, 940)], "and a better run still beats it");
        let p = Progress::parse(&format!(
            "(\n    hill: 1,\n    scores: [\"1 {}\"],\n)\n",
            MOVES_CAP + 235
        ));
        assert_eq!(
            p.scores,
            vec![(0, MOVES_CAP)],
            "the file passes the same door"
        );
    }

    #[test]
    fn the_progress_page_is_a_window_over_the_descent() {
        let mut d = demo();
        d.page = Page::Progress;
        // Even with nothing warm, the hill you stand on owns a row.
        let rows = d.menu_rows(d.page, 1080.0);
        assert_eq!(
            rows.first().map(|r| r.action),
            Some(Action::Hill(0)),
            "the first hill is stood on, so it is listed"
        );
        assert_eq!(rows.last().map(|r| r.action), Some(Action::Back));
        for hill in 0..20 {
            d.progress.record(hill, 30 + hill as i32);
        }
        let fits = progress_fits(1080.0);
        let rows = d.menu_rows(d.page, 1080.0);
        assert_eq!(
            rows.len(),
            fits + 1,
            "the window shows its plate-full of hills and pins back under them"
        );
        assert_eq!(
            rows.last().map(|r| r.action),
            Some(Action::Back),
            "and the way out rides below the last visible row"
        );
        d.menu.progress_top = 10;
        let rows = d.menu_rows(d.page, 1080.0);
        assert_eq!(
            rows.first().map(|r| r.action),
            Some(Action::Hill(10)),
            "slide the window and its rows come with it"
        );
        let lowest = rows.iter().map(|r| r.cy).fold(f32::MAX, f32::min);
        assert!(
            lowest >= progress_top_y(1080.0) - fits as f32 * progress_step(1080.0) - 0.01,
            "the pinned back row escaped its place: {lowest}"
        );
    }

    #[test]
    fn the_whole_descent_hangs_open_and_unwarmed_hills_keep_silence() {
        let mut d = demo();
        d.page = Page::Progress;
        d.load(2); // three hills stood on, none warmed.
        d.progress.stood(2);
        let rows = d.menu_rows(d.page, 1080.0);
        // The ledger is no gate: every hill is selectable from the
        // first day, so the window fills with the descent itself,
        // not with the ground already walked.
        assert_eq!(
            rows.iter()
                .filter(|r| matches!(r.action, Action::Hill(_)))
                .count(),
            progress_fits(1080.0),
            "the window hangs full of descent, standing or not"
        );
        assert!(
            !rows[0].text.contains("moves"),
            "an unscored hill keeps its score column empty"
        );
        // And the last hill of the descent is a door like the first:
        // scrolled to the bottom, it stands in the window.
        d.menu.progress_top = d.levels.len() - 1;
        assert!(
            d.menu_rows(d.page, 1080.0)
                .iter()
                .any(|r| r.action == Action::Hill(d.levels.len() - 1)),
            "the deepest hill is selectable like the shallowest"
        );
        d.menu.progress_top = 0;
        // A run that overshot par reads its debt; its amount belongs
        // to `the_ledger_owes_the_imperfect_runs_first`'s fixture.
        d.progress.record(1, 40);
        let rows = d.menu_rows(d.page, 1080.0);
        assert_eq!(rows[1].action, Action::Hill(1));
        assert!(
            rows[1].text.contains("40 of 5"),
            "a best over par wears `moves of par`: {}",
            rows[1].text
        );
        assert!(
            !rows[2].text.contains("moves"),
            "and its neighbours keep their word"
        );
    }

    #[test]
    fn the_ledger_owes_the_imperfect_runs_first() {
        let mut d = demo();
        d.page = Page::Progress;
        d.load(5);
        d.progress.stood(5); // six hills stood on, of fifty open.
        // Perfect the 2nd and 4th (their pars are 5 and 10), overshoot
        // the 5th, and leave the rest of the descent unwarmed.
        d.progress.record(1, 3);
        d.progress.record(3, 8);
        d.progress.record(4, 30);
        let rows = d.menu_rows(d.page, 1080.0);
        let acts: Vec<Action> = rows.iter().map(|r| r.action).collect();
        // The owed runs — over-par with the never-walked — lead in
        // hill order; the perfected sink below the fold of a descent
        // that no longer closes.
        assert_eq!(
            &acts[..4],
            &[
                Action::Hill(0),
                Action::Hill(2),
                Action::Hill(4),
                Action::Hill(5),
            ],
            "the owed runs lead in hill order — the unwarmed with the over-par"
        );
        assert!(
            !acts.contains(&Action::Hill(1)) && !acts.contains(&Action::Hill(3)),
            "the perfected have sunk below this window"
        );
        // And the sweep is only resting, not gone: scrolled to the
        // very bottom of the descent, the perfected rows wait there
        // in their own hill order.
        d.menu.progress_top = d.levels.len() - 2;
        let last = d.menu_rows(d.page, 1080.0);
        assert_eq!(
            last.iter().map(|r| r.action).collect::<Vec<_>>(),
            vec![Action::Hill(1), Action::Hill(3), Action::Back],
            "the clean sweeps settle to the bottom of the scroll"
        );
    }

    #[test]
    fn next_leads_the_owed_hill_or_the_new_one() {
        let mut d = demo();
        // Warm the first two hills and stand on the deepest: at the
        // frontier of the descent, the never-walked hill below
        // answers first, whatever the hills behind may owe.
        d.load(2);
        d.progress.stood(2);
        d.progress.record(0, 1);
        d.progress.record(1, 1);
        assert_eq!(
            d.next_hill(),
            3,
            "at the frontier of the descent, the never-walked hill below answers first"
        );
        // Walk down to the sixth hill and back to the first, leaving
        // the third unwarmed: from shallow ground `next` leads to the
        // nearest owed hill ahead, not past it to the new one.
        d.load(5);
        d.progress.stood(5);
        for hill in 3..6 {
            d.progress.record(hill, 1);
        }
        d.load(0);
        assert_eq!(d.next_hill(), 2, "the hill still owed ahead leads");
        // Warm that one too, and the walked ground holds nothing to
        // owe: the new hill below takes the key.
        d.progress.record(2, 1);
        d.load(1);
        assert_eq!(
            d.next_hill(),
            6,
            "nothing owed on walked ground: the new hill below answers"
        );
        // And when the whole authored descent is perfect, `next` is
        // what it always was: the plain next hill, wrapping.
        for hill in 0..d.levels.len() {
            d.progress.record(hill, 1);
        }
        assert_eq!(
            d.next_hill(),
            2,
            "a perfected descent: `next` just moves on"
        );
    }

    #[test]
    fn next_greys_when_the_hill_underfoot_is_all_that_is_owed() {
        // A player who never moved has whole hills still to walk:
        // the key is wide awake.
        assert!(demo().next_owed());
        let mut d = demo();
        let last = d.levels.len() - 1;
        // Walk the whole descent and perfect every hill but the
        // third. With that one hill underfoot, `next` leads nowhere
        // the player is not already standing — and greys.
        d.load(last);
        d.progress.stood(last);
        for hill in 0..d.levels.len() {
            if hill != 2 {
                d.progress.record(hill, 1);
            }
        }
        d.load(2);
        assert!(
            !d.next_owed(),
            "the only owed hill is the one underfoot: `next` is spent"
        );
        // Step aside to a perfected hill and the owed one waits
        // ahead again — the grey is the hill's, not the page's.
        d.load(3);
        assert!(d.next_owed(), "from perfected ground the owed hill waits");
        // Warm the last owed hill and the descent owes nothing; a
        // perfected `next` has nowhere to move on to either.
        d.load(2);
        d.progress.record(2, 1);
        assert!(
            !d.next_owed(),
            "a perfected descent leaves `next` nothing to lead to"
        );
    }

    #[test]
    fn the_settings_page_offers_every_setting() {
        let mut d = demo();
        d.page = Page::Settings;
        let rows = d.menu_rows(d.page, 1080.0);
        let actions: Vec<Action> = rows.iter().map(|r| r.action).collect();
        for a in [
            Action::Vsync,
            Action::Filter,
            Action::Fullscreen,
            Action::Quality,
            Action::Ambient,
            Action::Light,
            Action::Leaves,
            Action::Back,
        ] {
            assert!(actions.contains(&a), "the page must answer {a:?}");
        }
        assert_eq!(
            rows.iter().filter(|r| r.slider.is_some()).count(),
            2,
            "two sliders: ambient and light"
        );
    }

    #[test]
    fn the_main_menu_is_the_seven_rows_requested() {
        let d = demo();
        let actions: Vec<Action> = d
            .menu_rows(d.page, 1080.0)
            .iter()
            .map(|r| r.action)
            .collect();
        assert_eq!(
            actions,
            vec![
                Action::New,
                Action::Resume,
                Action::OpenSettings,
                Action::OpenProgress,
                Action::Keyboard,
                Action::Mouse,
                Action::Gamepad,
                Action::Exit,
            ]
        );
    }

    #[test]
    fn the_won_page_hangs_low_enough_for_its_verdict() {
        // At 720 the verdict's strip runs from the plate's top
        // (top + 130) to the score banner's bottom (h/2 - 93) and
        // must hold the full 64-unit word clear of both.
        let mut d = demo();
        d.page = Page::Won;
        let rows = d.menu_rows(d.page, 720.0);
        let plate_top = rows.iter().map(|r| r.cy).fold(f32::MIN, f32::max) + plate_top_pad(1080.0);
        let band = 720.0 / 2.0 - 93.0;
        assert!(
            band - plate_top >= 64.0 + 16.0,
            "the strip holds only {}",
            band - plate_top
        );
    }

    #[test]
    fn a_finished_hill_offers_next_and_retry() {
        // The two ways off a won hill — and the only two: they hang
        // under the verdict splash, not on the main menu.
        let mut d = demo();
        d.page = Page::Won;
        let actions: Vec<Action> = d
            .menu_rows(d.page, 1080.0)
            .iter()
            .map(|r| r.action)
            .collect();
        assert_eq!(actions, vec![Action::Next, Action::Retry]);
        assert_eq!(escape_nav(Page::Won), Escape::ToMenu);
    }
}
