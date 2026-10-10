//! The frame: `frost::Process` for the desk, every handler of
//! the one process pass.
// The subjects this one reads.
use crate::actions::*;
use crate::art::*;
use crate::band::*;
use crate::clip::*;
use crate::desk::*;
use crate::journal::*;
use crate::layout::*;
use crate::map::*;
use crate::ron_panels::*;
use crate::ron_view;
use crate::sidecar::*;
use crate::spots::*;
use crate::strip::*;
use crate::theme::*;
use crate::world::*;
use frost::ron as ron_tree;

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        // The animation clock runs on wall time; when it rolls to a new
        // frame, the work area repaints its layer pool to that frame.
        if self.anim.tick(dt) {
            self.sync_work(ctx);
        }

        // The log captures the bench's voice: whatever the last
        // handlers settled on is this frame's message, and only a
        // change of it is an event worth remembering.
        log_note(&mut self.log, &mut self.log_seen, &self.status);

        let (w, h) = ctx.size();
        // The exit guard: Escape asks, the box answers. While the box
        // stands every other key is dead and the pointer belongs to
        // the box alone — Enter or Y leaves, Escape or N stays. An
        // Escape pressed when nothing is held opens the box; while a
        // picked position or a held block stands, Escape still lets
        // go of that first, as ever.
        let esc_now = ctx.key_down_raw(frost::KeyCode::Escape);
        let esc_edge = esc_now && !self.was_esc_prompt;
        self.was_esc_prompt = esc_now;
        if self.quit_prompt {
            ctx.set_keys_frozen(true);
            if ctx.key_down_raw(frost::KeyCode::Enter) || ctx.key_down_raw(frost::KeyCode::KeyY) {
                self.quit_prompt = false;
                ctx.exit();
            } else if esc_edge || ctx.key_down_raw(frost::KeyCode::KeyN) {
                self.quit_prompt = false;
                self.status = String::from("staying");
            }
            // The box ate this Escape: the let-go handlers further
            // down must not also hear it.
            self.was_esc = esc_now;
            self.was_block_esc = esc_now;
        } else {
            ctx.set_keys_frozen(false);
            // The log panel answers Escape before the bench does: it
            // closes, and the closing is the key's whole meaning this
            // frame — not a let-go, and not the quit question.
            if esc_edge && self.log_open {
                self.log_open = false;
                self.was_esc = esc_now;
                self.was_block_esc = esc_now;
            } else {
                let holding = self.clip.is_some()
                    || self.sel.is_some()
                    || self.sel_drag.is_some()
                    || self.ron().is_some_and(|d| d.edit.is_some());
                if esc_edge && !holding {
                    self.quit_prompt = true;
                    self.was_esc = true;
                    self.was_block_esc = true;
                    self.status = String::from("quit? — Enter/Y leaves, Escape/N stays");
                }
            }
        }
        // The window manager's close button asks the same question:
        // an explicit request, like the menu's Quit — the box opens
        // whatever the bench holds, and a repeat request while the
        // box stands only asks again.
        if ctx.close_requested() && !self.quit_prompt {
            self.quit_prompt = true;
            self.status = String::from("quit? — Enter/Y leaves, Escape/N stays");
        }
        let box_up = self.quit_prompt;
        let down = ctx.mouse_button_down(frost::MouseButton::Left) && !box_up;
        let rdown = ctx.mouse_button_down(frost::MouseButton::Right) && !box_up;
        let mdown = ctx.mouse_button_down(frost::MouseButton::Middle) && !box_up;
        let rpressed = rdown && !self.was_rdown;
        let rreleased = !rdown && self.was_rdown;
        self.was_rdown = rdown;
        let mpressed = mdown && !self.was_mdown;
        let mreleased = !mdown && self.was_mdown;
        self.was_mdown = mdown;
        let pressed = down && !self.was_down;
        let released = !down && self.was_down;
        self.was_down = down;
        let pos = ctx.mouse_position();
        let size = self.work_size();
        // The active sprite's center in window coordinates: the work
        // area's center plus the pan offset.
        let view = [self.offset[0], self.offset[1] + WORK_Y];
        // The tile-map desk's canvas behavior: no sprite to pick, drag
        // or crop — the canvas is a map camera now.
        let tile_view = self.view == View::TileMap;
        if self.first_frame {
            self.first_frame = false;
            self.sync_work(ctx);
        }
        self.sync_band();

        // Every sidecar tree re-pins its scroll every frame, so folding
        // a node, resizing a panel or closing a file can never strand a
        // view past its rows. The view rectangles the wheel and the pan
        // guard read are LAST frame's; clicks read the fresh ones, laid
        // out below.
        for sp in self.sprites.iter_mut() {
            if let Some(doc) = &mut sp.ron {
                doc.clamp_scroll();
            }
        }
        let over_ron = pos.is_some_and(|p| self.ron_views.iter().any(|v| in_rect(v.panel, p)));

        // The wheel zooms: multiplicative per line (up zooms in), clamped
        // to the slider's range, anchored so the texture point under the
        // cursor stays put — the offset absorbs the scale change the cursor
        // itself would have drifted. With the cursor outside the window the
        // anchor is the work area's center, so only the zoom changes.
        let wheel = if self.quit_prompt {
            0.0
        } else {
            ctx.mouse_wheel()
        };
        // The wheel scrolls the sidecar tree under the cursor —
        // sideways with Shift — while it rests on an open view's body;
        // anywhere else it keeps zooming.
        let mut scrolled = false;
        if wheel != 0.0 && self.log_open && pos.is_some_and(|p| in_rect(log_panel_rect(w, h), p)) {
            // The log answers the wheel over its own pixels: the
            // window steps two rows a line, Shift slides sideways.
            if ctx.key_down(frost::KeyCode::ShiftLeft) || ctx.key_down(frost::KeyCode::ShiftRight) {
                let body = log_body(log_panel_rect(w, h));
                let wide = log_wide(&self.log) as f32 * RON_ADV;
                let max = (wide - (body[2] - body[0])).max(0.0);
                self.log_sx = (self.log_sx - wheel * 2.0 * RON_ADV).clamp(0.0, max);
            } else {
                let step = (wheel * 2.0).round() as i64;
                let len = self.log.len();
                self.log_top =
                    (self.log_top as i64 - step).clamp(0, len.saturating_sub(1) as i64) as usize;
            }
            scrolled = true;
        }
        if wheel != 0.0
            && !scrolled
            && let Some(p) = pos
        {
            let slot = self
                .ron_views
                .iter()
                .find(|v| !v.folded && in_rect(v.body, p))
                .map(|v| v.slot);
            if let Some(doc) = slot.and_then(|s| self.sprites[s].ron.as_mut()) {
                if ctx.key_down(frost::KeyCode::ShiftLeft)
                    || ctx.key_down(frost::KeyCode::ShiftRight)
                {
                    doc.sx -= wheel * 2.0 * RON_ADV;
                } else {
                    doc.scroll -= wheel * RON_LINE * 2.0;
                }
                doc.clamp_scroll();
                scrolled = true;
            }
        }
        // The slot strip's window for this frame: its plate count —
        // one per sprite plus the spare loader plate — the pool's
        // visible width, and the scroll clamped to what they allow.
        // (`f32::MAX` parked in the scroll means "ride to the end",
        // how a freshly loaded sprite earns its view.)
        let slot_count = self.sprites.len() + 1;
        let sv = strip_view(w, slot_count);
        self.slot_scroll = self.slot_scroll.clamp(0.0, sv.scroll_max);
        // The tileset panel's frame-state: its grid, its cell size at
        // the panel's own zoom, and where it stands — the live dragged
        // spot while a drag runs, the stored home otherwise, always
        // clamped into the desk. Press, wheel, ghost and paint all
        // read this one geometry; nobody recomputes it.
        let (brows, bcols) = self.active().and_then(|sp| sp.atlas).unwrap_or((1, 1));
        let bcell = band_cell(brows, bcols, w, h, self.band_zoom);
        let bhome = self
            .band_origin
            .unwrap_or_else(|| band_origin_default(w, h));
        let bor = band_clamp(
            match (self.band_drag, pos) {
                (Some((_, from, origin0)), Some(p)) => {
                    [origin0[0] + (p[0] - from[0]), origin0[1] + (p[1] - from[1])]
                }
                _ => bhome,
            },
            bcell,
            brows,
            bcols,
            w,
            h,
        );
        let brects = band_rects(brows, bcols, bcell, bor);
        let bcells = self.cell_shapes.len().min(brects.len());
        let bplate = if tile_view {
            band_plate(&brects, bcells)
        } else {
            None
        };
        if wheel != 0.0 && !scrolled {
            let over_strip = pos.is_some_and(|p| p[1] < floor_y(h) + STRIP_H);
            let over_band = bplate.is_some_and(|r| p_in(pos, r));
            if over_band {
                // Over the tileset panel the wheel belongs to the
                // panel: the rack's cells grow and shrink, the canvas
                // does not care.
                self.band_zoom =
                    (self.band_zoom * WHEEL_ZOOM.powf(wheel)).clamp(BAND_ZOOM_MIN, BAND_ZOOM_MAX);
            } else if over_strip && sv.scroll_max > 0.0 {
                // The wheel belongs to the strip while the cursor
                // rests on it and there is something to scroll.
                self.slot_scroll = (self.slot_scroll - wheel * 48.0).clamp(0.0, sv.scroll_max);
            } else {
                let z1 = (self.zoom * WHEEL_ZOOM.powf(wheel)).clamp(ZOOM_MIN, ZOOM_MAX);
                if z1 != self.zoom {
                    let k = z1 / self.zoom;
                    let [ax, ay] = pos.unwrap_or([0.0, WORK_Y]);
                    self.offset = [
                        ax - (ax - self.offset[0]) * k,
                        (ay - WORK_Y) - ((ay - WORK_Y) - self.offset[1]) * k,
                    ];
                    self.zoom = z1;
                }
            }
        }

        // A middle-drag — the wheel button held — grabs the work area:
        // the sprite follows the cursor's frame-to-frame movement, so it
        // always lands under the pointer. The Atlas view opts out: its
        // tileset is nailed down, and the wheel is the only way the
        // picture moves.
        if mdown
            && !over_ron
            && self.view != View::Atlas
            && let (Some([lx, ly]), Some([mx, my])) = (self.last_mouse, pos)
        {
            self.offset[0] += mx - lx;
            self.offset[1] += my - ly;
        }
        // The middle click — the wheel button pressed and released
        // without travel — moves the held block, when there is one:
        // the frame it was copied from is swept, the block lands
        // cursor-anchored, and the frame travels with the drop. One
        // gesture, one undo step. A middle press that travels is the
        // pan above, never a move.
        if mpressed && tile_view && self.clip.is_some() && !self.ui.hovering() && !over_ron {
            self.m_press = pos;
        } else if mpressed {
            self.m_press = None;
        }
        if mreleased
            && tile_view
            && let (Some(from), Some(p)) = (self.m_press.take(), pos)
            && self.clip.is_some()
            && ((p[0] - from[0]).powi(2) + (p[1] - from[1]).powi(2)).sqrt() < CLICK_TOL
            && !self.ui.hovering()
            && !self.log_open
            && !over_ron
            && let Some((col, row)) = desk_cell(self.active(), p, view, self.zoom)
        {
            let before = self.capture();
            let clip = self.clip.clone().expect("a block is held");
            let (n, erased, sel) = clip_move(&mut self.maps, &clip, self.sel, col, row);
            self.sel = sel;
            log::info!("block: moved to ({col},{row}) wrote {n} cut {erased}");
            if n > 0 || erased {
                push_step(&mut self.undo_stack, &mut self.redo_stack, before);
                self.status = format!(
                    "moved {n} cell{} — the block is still held",
                    if n == 1 { "" } else { "s" }
                );
                self.sync_work(ctx);
            }
        }

        // The right button picks a position back from the sprite: the
        // marker nearest the click lights up its row — the path unfolds
        // and the view scrolls that row to its centre — and the
        // position is picked, so the next LEFT click moves it. A press
        // that travels is a pan attempt, not a pick.
        if rpressed
            && !tile_view
            && let Some(p) = pos
            && !over_ron
        {
            self.r_press = Some(p);
        }
        if rreleased
            && let (Some(from), Some(p)) = (self.r_press.take(), pos)
            && ((p[0] - from[0]).powi(2) + (p[1] - from[1]).powi(2)).sqrt() < CLICK_TOL
            && !self.sprites.is_empty()
        {
            let [px, py] = tex_point(p, size, view, self.zoom);
            let inside = px >= 0.0 && px <= size[0] && py >= 0.0 && py <= size[1];
            if inside && self.sprites[self.active].ron.is_some() {
                let slot = self.active;
                let mut spots: Vec<Spot> = Vec::new();
                if let Some(d) = self.sprites[slot].ron.as_ref() {
                    scan_spots(&d.root, &mut Vec::new(), "", &mut spots);
                }
                if let Some(pi) = spot_pick(&spots, p, size, view, self.zoom, SPOT_HIT_R) {
                    let (path, label) = (spots[pi].path.clone(), spots[pi].label.clone());
                    if let Some(doc) = self.sprites[slot].ron.as_mut() {
                        doc.edit = Some(pi);
                        unfold_path(&mut doc.root, &path);
                        doc.rows = ron_view::layout(&doc.root);
                        if let Some(i) = doc.rows.iter().position(|r| r.path == path) {
                            let track = self
                                .ron_views
                                .iter()
                                .find(|v| v.slot == slot)
                                .filter(|v| !v.folded && v.body != [0.0; 4])
                                .map(|v| v.body[3] - v.body[1]);
                            if let Some(track) = track {
                                let total = doc.rows.len() as f32 * RON_LINE + RON_PAD;
                                doc.scroll = ((i as f32 + 0.5) * RON_LINE + RON_PAD - track / 2.0)
                                    .clamp(0.0, (total - track).max(0.0));
                            }
                        }
                        doc.clamp_scroll();
                        self.status = format!("editing '{label}' — click the sprite to move it");
                    }
                }
            }
        }

        // The UI frame: the panels claim the mouse over their bodies — a
        // press the UI holds is never a selection drag, a slot touch or a
        // sprite click.
        self.ui.begin(ctx);
        // While the exit box stands, the UI owns the window whole:
        // every widget outside the box is drawn but inert.
        self.ui.set_modal(self.quit_prompt);
        // Which panels this frame declares: Markers keeps the whole
        // editing desk, Atlas keeps only its grid. The slots below and
        // the work area are the views' common ground — a sprite is
        // picked and worked on the same way in either.
        let markers_view = self.view == View::Markers;
        // The Animation panel: a frame is a set of layers holding for
        // its own time; playback wraps (loop) or bounces (ping-pong).
        // Buttons set flags acted on after the panels.
        let mut want_prev_frame = false;
        let mut want_next_frame = false;
        let mut want_add_frame = false;
        let mut want_del_frame = false;
        let mut want_add_layer = false;
        let mut want_del_layer = false;
        let mut want_play = false;
        let mut looping = self.anim.looping;
        let mut next_time = self
            .anim
            .frames
            .get(self.anim.frame)
            .map_or(TIME_DEFAULT, |f| f.next_time);
        let anim_line = if self.anim.frames.is_empty() {
            String::from("no frames yet")
        } else {
            let f = &self.anim.frames[self.anim.frame];
            format!(
                "frame {}/{} · {} layer(s) · {}",
                self.anim.frame + 1,
                self.anim.frames.len(),
                f.layers.len(),
                if self.anim.playing {
                    "playing"
                } else {
                    "paused"
                }
            )
        };
        let play_label = if self.anim.playing { "stop" } else { "play" };
        if markers_view {
            self.ui.panel(
                ctx,
                "Animation",
                [w / 2.0 - PANEL_W / 2.0 - 20.0, h / 2.0 - 120.0],
                PANEL_W,
                |ui, ctx| {
                    ui.label(ctx, &anim_line);
                    ui.table(
                        ctx,
                        "anim",
                        &[
                            frost::Col::auto(frost::Align::Center),
                            frost::Col::auto(frost::Align::Center),
                            frost::Col::auto(frost::Align::Center),
                        ],
                        |ui, ctx| {
                            if ui.button(ctx, "prev") {
                                want_prev_frame = true;
                            }
                            if ui.button(ctx, "add frame") {
                                want_add_frame = true;
                            }
                            if ui.button(ctx, "del frame") {
                                want_del_frame = true;
                            }
                            if ui.button(ctx, "next") {
                                want_next_frame = true;
                            }
                            if ui.button(ctx, "add layer") {
                                want_add_layer = true;
                            }
                            if ui.button(ctx, "del layer") {
                                want_del_layer = true;
                            }
                            ui.label(ctx, "till next");
                            ui.slider_track(ctx, "frame time", &mut next_time, TIME_MIN, TIME_MAX);
                            ui.readout(ctx, &format!("{next_time:.2}s"));
                        },
                    );
                    ui.space(6.0);
                    ui.checkbox(ctx, "loop  (off = ping-pong)", &mut looping);
                    if ui.button(ctx, play_label) {
                        want_play = true;
                    }
                },
            );
        }

        // The Atlas panel: how the active sprite splits into tiles. The
        // sliders set the grid's rows and columns, the grid draws over
        // the sprite, and the grid persists as the sidecar root's
        // `atlas` field, which Save carries with the PNG. 1 x 1 is no
        // grid.
        let (mut atlas_rows, mut atlas_cols) = self
            .active()
            .and_then(|sp| sp.atlas)
            .map_or((1.0, 1.0), |(rows, cols)| (rows as f32, cols as f32));
        let mut want_clear_atlas = false;
        let atlas_line = match self.active().and_then(|sp| sp.atlas) {
            Some((rows, cols)) => format!("{rows} x {cols} · {} tiles", rows * cols),
            None => String::from("no grid · 1 x 1 is none"),
        };
        if !markers_view {
            self.ui.panel(
                ctx,
                "Atlas",
                [w / 2.0 - PANEL_W / 2.0 - 20.0, h / 2.0 - 330.0],
                PANEL_W,
                |ui, ctx| {
                    ui.label(ctx, &atlas_line);
                    ui.table(
                        ctx,
                        "atlas",
                        &[
                            frost::Col::auto(frost::Align::Left),
                            frost::Col::stretch(1.0, frost::Align::Left),
                            frost::Col::auto(frost::Align::Center),
                        ],
                        |ui, ctx| {
                            ui.label(ctx, "rows");
                            ui.slider_track(ctx, "atlas rows", &mut atlas_rows, 1.0, ATLAS_MAX);
                            ui.readout(ctx, &format!("{atlas_rows:.0}"));
                            ui.label(ctx, "cols");
                            ui.slider_track(ctx, "atlas cols", &mut atlas_cols, 1.0, ATLAS_MAX);
                            ui.readout(ctx, &format!("{atlas_cols:.0}"));
                        },
                    );
                    ui.space(6.0);
                    if ui.button(ctx, "clear") {
                        want_clear_atlas = true;
                    }
                },
            );
        }
        // The sliders' read-back: when the grid moved, write it into the
        // active sprite — and, beside its sidecar's root, so Save
        // carries it with the PNG. The sidecar's rows rebuild from the
        // tree, the way every other sidecar edit does.
        let rows = atlas_rows.round().max(1.0) as usize;
        let cols = atlas_cols.round().max(1.0) as usize;
        let next = (rows > 1 || cols > 1).then_some((rows, cols));
        let setting = self.active().is_some_and(|sp| sp.atlas != next);
        let clearing = want_clear_atlas && self.active().is_some_and(|sp| sp.atlas.is_some());
        if setting || clearing {
            // The grid moved — by slider step or by clear: one step of
            // history per movement, the way any other command counts.
            self.stamp();
        }
        if let Some(sp) = self.sprites.get_mut(self.active) {
            if sp.atlas != next {
                sp.atlas = next;
                if let Some(doc) = sp.ron.as_mut()
                    && set_atlas(&mut doc.root, next)
                {
                    doc.rows = ron_view::layout(&doc.root);
                    doc.clamp_scroll();
                }
            }
            if want_clear_atlas {
                sp.atlas = None;
                if let Some(doc) = sp.ron.as_mut()
                    && set_atlas(&mut doc.root, None)
                {
                    doc.rows = ron_view::layout(&doc.root);
                    doc.clamp_scroll();
                }
            }
        }

        // Ctrl-O's and Ctrl-Z's rising edges: the open dialog, and one
        // step back through the active sprite's crops.
        let ctrl =
            ctx.key_down(frost::KeyCode::ControlLeft) || ctx.key_down(frost::KeyCode::ControlRight);
        let open_key = ctrl && ctx.key_down(frost::KeyCode::KeyO);
        let shift =
            ctx.key_down(frost::KeyCode::ShiftLeft) || ctx.key_down(frost::KeyCode::ShiftRight);
        let undo_key = ctrl && ctx.key_down(frost::KeyCode::KeyZ) && !shift;
        let redo_key = ctrl && shift && ctx.key_down(frost::KeyCode::KeyZ);
        if open_key && !self.was_open {
            self.open_dialog(ctx);
        }
        if undo_key && !self.was_undo {
            self.undo(ctx);
        }
        if redo_key && !self.was_redo {
            self.redo(ctx);
        }
        // Ctrl-+ and Ctrl-- step the zoom about the work area's centre,
        // one wheel-notch per press. The characters lead, exactly as for
        // the overlay's Alt pair: a physical code names a US-ANSI
        // position, so the Swedish `+` key reports the US `-` code and a
        // physical fallback would zoom the WRONG way on that layout —
        // only the typed character follows the user's keys. The set is
        // every character the +/− keys type with Ctrl held across
        // platforms (the plain glyphs, the `=` the US + key carries,
        // and the macOS Option weavings); the numpad codes, whose signs
        // are layout-independent, join as the safety net.
        let zoom_in = ctrl
            && (ctx.char_down('+')
                || ctx.char_down('=')
                || ctx.char_down('\u{b1}')
                || ctx.char_down('\u{2212}')
                || ctx.key_down(frost::KeyCode::NumpadAdd));
        let zoom_out = ctrl
            && (ctx.char_down('-')
                || ctx.char_down('_')
                || ctx.char_down('\u{2013}')
                || ctx.char_down('\u{2014}')
                || ctx.key_down(frost::KeyCode::NumpadSubtract));
        if zoom_in && !self.was_zoom_in {
            self.zoom = (self.zoom * WHEEL_ZOOM).clamp(ZOOM_MIN, ZOOM_MAX);
        }
        if zoom_out && !self.was_zoom_out {
            self.zoom = (self.zoom / WHEEL_ZOOM).clamp(ZOOM_MIN, ZOOM_MAX);
        }
        self.was_open = open_key;
        self.was_undo = undo_key;
        self.was_redo = redo_key;
        self.was_zoom_in = zoom_in;
        self.was_zoom_out = zoom_out;
        // The brush's orientation, on the tile desk only: X and Y
        // mirror the cell, R turns a quarter counterclockwise, Shift-R back.
        // Characters lead, as everywhere the keyboard turns — a letter
        // follows the user's layout — but the turn's DIRECTION reads
        // the physical Shift, so a Caps-Locked keyboard still turns
        // the plain way on a plain R.
        let flipx_key = tile_view && (ctx.char_down('x') || ctx.char_down('X'));
        let flipy_key = tile_view && (ctx.char_down('y') || ctx.char_down('Y'));
        let turn_key = tile_view && (ctx.char_down('r') || ctx.char_down('R'));
        if flipx_key && !self.was_flipx {
            self.brush = tfm_flip_x(self.brush);
            self.status = if self.brush & 1 != 0 {
                "flip x on".to_owned()
            } else {
                "flip x off".to_owned()
            };
        }
        if flipy_key && !self.was_flipy {
            self.brush = tfm_flip_y(self.brush);
            self.status = if self.brush & 2 != 0 {
                "flip y on".to_owned()
            } else {
                "flip y off".to_owned()
            };
        }
        if turn_key && !self.was_turn {
            // The keyboard brings no Alt or Control to R — but the
            // turn's direction still comes from the one table, so
            // key and mouse can never disagree about "plain".
            self.brush = dress_brush(self.brush, dress_gesture(shift, false, false));
            self.status = format!("turned {}", turn_note(self.brush));
        }
        self.was_flipx = flipx_key;
        self.was_flipy = flipy_key;
        self.was_turn = turn_key;

        // Space's rising edge starts and stops the animation clock.
        let space = ctx.key_down(frost::KeyCode::Space);
        if space && !self.was_space {
            self.anim.playing = !self.anim.playing;
            self.anim.dir = 1;
        }
        self.was_space = space;

        // The log's arrows: while the panel stands, up and down step
        // the highlight one message a press (holding does not run),
        // and left and right slide long lines. Closed, or asked by
        // the quit box, the arrows rest — and their held state parks,
        // so reopening never fires an old press.
        if self.log_open && !self.quit_prompt {
            let arrows = [
                ctx.key_down(frost::KeyCode::ArrowUp),
                ctx.key_down(frost::KeyCode::ArrowDown),
                ctx.key_down(frost::KeyCode::ArrowLeft),
                ctx.key_down(frost::KeyCode::ArrowRight),
            ];
            let len = self.log.len();
            if arrows[0] && !self.log_arrows[0] && len > 0 {
                self.log_sel = self.log_sel.saturating_sub(1);
                self.log_top = log_reveal(len, self.log_top, self.log_sel);
            }
            if arrows[1] && !self.log_arrows[1] && len > 0 {
                self.log_sel = (self.log_sel + 1).min(len - 1);
                self.log_top = log_reveal(len, self.log_top, self.log_sel);
            }
            if arrows[2] && !self.log_arrows[2] || arrows[3] && !self.log_arrows[3] {
                let step = if arrows[2] { -4.0 } else { 4.0 } * RON_ADV;
                let wide = log_wide(&self.log) as f32 * RON_ADV;
                let body = log_body(log_panel_rect(w, h));
                let max = (wide - (body[2] - body[0])).max(0.0);
                self.log_sx = (self.log_sx + step).clamp(0.0, max);
            }
            self.log_arrows = arrows;
        } else {
            self.log_arrows = [false; 4];
        }

        // Escape's rising edge lets go of a picked position.
        let esc = ctx.key_down(frost::KeyCode::Escape);
        if esc && !self.was_esc {
            let mut was = false;
            if let Some(doc) = self.ron_mut() {
                was = doc.edit.take().is_some();
            }
            if was {
                self.status = String::from("edit: released");
            }
        }
        self.was_esc = esc;

        let (row_h, pad) = {
            let st = self.ui.style_mut();
            (st.row_h, st.pad)
        };
        // The sidecar views: one `Ui` panel per open file, each titled
        // by its file's name — every bar drags its panel and folds it
        // whole, like the View and Animation panels, and the views
        // cascade from the work area's lower right. A panel is declared
        // at its document's own size, and every rectangle the tree,
        // handles and close marks work from is the plate the UI ACTUALLY
        // painted this frame — the plate grows from the measured height
        // of the frame before, and only that matches the pixels.
        self.ron_views.clear();
        let open_docs: Vec<(usize, String, f32, f32)> = self
            .sprites
            .iter()
            .enumerate()
            .filter(|_| markers_view)
            .filter_map(|(i, sp)| sp.ron.as_ref().map(|d| (i, d.name.clone(), d.vw, d.vh)))
            .collect();
        for (i, title, vw, vh) in open_docs {
            let mut open = false;
            self.ui.panel(
                ctx,
                &title,
                ron_default(w, h, row_h, pad, self.ron_views.len()),
                vw,
                |ui, _ctx| {
                    ui.space(vh);
                    open = true;
                },
            );
            if let Some(panel) = self.ui.panel_rect(&title) {
                // The body is the plate between the title bar and the
                // plate's bottom edge, each side padded; a plate that
                // is still just its bar (the snap-open frame) has no
                // body worth laying out.
                let body = if open {
                    [panel[0], panel[1] + pad, panel[2], panel[3] - row_h - pad]
                } else {
                    [0.0; 4]
                };
                let body = if body[3] - body[1] >= RON_LINE {
                    body
                } else {
                    [0.0; 4]
                };
                let close = [panel[2] - 17.0, panel[3] - row_h / 2.0];
                self.ron_views.push(RonView {
                    slot: i,
                    body,
                    panel,
                    close,
                    folded: !open,
                });
            }
        }

        // The authored shapes: the active sprite's sidecar read once
        // per frame — the desk below paints its geometry, the bar's
        // notes voice its refusals. The read runs whether or not the
        // overlay is checked: validation is a voice, not a view option,
        // and a mis-authored hit box must not go quiet because someone
        // turned the drawing off. On the tile desk the sprite is the
        // paint source, not the picture — its pixels are cells, and
        // per-tile shapes wait for the map format.
        let (authored, shape_errs) = match self.active().and_then(|sp| sp.ron.as_ref()) {
            Some(doc) if !tile_view => frost::read_shapes(&doc.root),
            _ => (Vec::new(), frost::ShapeErrors::default()),
        };
        let shapes_note = shapes_note(&shape_errs);

        // The menu bar, declared after every panel so it is the last
        // declared — which is what lets an open list float above the
        // plates and win their clicks. Declaring it here (after all the
        // panels, before the collected presses) puts its whole strip into
        // `ui.hovering()` too: no slot or panel drag acts through it.
        // The bar's one note: the shapes that refused to read. The
        // last command speaks on the status band now, and the view's
        // own name is pinned at the band's right edge — the menu bar
        // is for menus, and for what the file got wrong.
        let mut notes: Vec<&str> = Vec::new();
        if let Some(note) = shapes_note.as_deref() {
            notes.push(note);
        }
        // The View menu is assembled each frame: the three named corners
        // all switch the workbench, and beneath them the view's own
        // state —
        // the zoom as a readout, the checker's two greys as live
        // sliders. The panel that carried all of it is gone; the menu
        // wears its clothes. The last click needs no line: the bar's
        // status note already tells it.
        let zoom_note = format!("{:.2}", self.zoom);
        let view_menu = frost::Menu {
            title: "View",
            items: &[
                frost::MenuItem::new("Markers", "").checked(self.view == View::Markers),
                frost::MenuItem::new("Atlas", "").checked(self.view == View::Atlas),
                frost::MenuItem::new("Tile Map", "").checked(self.view == View::TileMap),
                frost::MenuItem::SEPARATOR,
                frost::MenuItem::new("Shapes", "").checked(self.show_shapes),
                frost::MenuItem::readout("Zoom", "Ctrl +/Ctrl -", &zoom_note),
                frost::MenuItem::slider("Light", 0.0, 1.0, self.light),
                frost::MenuItem::slider("Dark", 0.0, 1.0, self.dark),
            ],
        };
        // The brush's own menu, and it has no reason to exist off the
        // tile desk: transforms paint cells, and only there are cells.
        let transform_menu = frost::Menu {
            title: "Transform",
            items: &[
                frost::MenuItem::new("Flip X", "X").checked(self.brush & 1 != 0),
                frost::MenuItem::new("Flip Y", "Y").checked(self.brush & 2 != 0),
                frost::MenuItem::SEPARATOR,
                frost::MenuItem::new("Rotate 90 CW", "R"),
                frost::MenuItem::new("Rotate 90 CCW", "Shift-R"),
                frost::MenuItem::readout("Turn", "", turn_note(self.brush)),
            ],
        };
        // Operations is a sprite-desk menu: its one verb crops the
        // active texture, and the tile-map desk has no texture to cut
        // — only cells. There, the Transform menu wears the bar.
        let menus: Vec<frost::Menu> = if tile_view {
            vec![FILE_MENU, view_menu, transform_menu]
        } else {
            vec![FILE_MENU, view_menu, OPERATIONS_MENU]
        };
        match self.ui.menu_bar(ctx, &menus, &notes) {
            Some(frost::MenuEvent::Chose(menu, item)) => {
                log::info!("menu: {menu} / {item}");
                // Every acted-on line names itself first; the verbs that
                // write a richer line of their own (undo, crop, save)
                // simply write over it, and the note reads theirs.
                self.status = item.to_lowercase();
                match (menu, item) {
                    // Switching the desk re-syncs the work layer: the
                    // tile-map desk hides the sprite (it is the paint
                    // source, not the picture), and leaving it brings
                    // the sprite — or the animation frame — back.
                    ("View", "Markers") => {
                        self.view = View::Markers;
                        self.sync_work(ctx);
                    }
                    ("View", "Atlas") => {
                        self.view = View::Atlas;
                        self.sync_work(ctx);
                    }
                    ("View", "Tile Map") => {
                        self.view = View::TileMap;
                        self.sync_work(ctx);
                    }
                    // The overlay's own line: a display choice, no desk
                    // to re-sync — and unlike the views, it does not
                    // silence the refusals, only the drawing.
                    ("View", "Shapes") => self.show_shapes = !self.show_shapes,
                    // The brush turns and mirrors; the maps keep what
                    // they were painted with, so nothing re-syncs.
                    ("Transform", "Flip X") => self.brush = tfm_flip_x(self.brush),
                    ("Transform", "Flip Y") => self.brush = tfm_flip_y(self.brush),
                    ("Transform", "Rotate 90 CW") => self.brush = tfm_turn(self.brush, true),
                    ("Transform", "Rotate 90 CCW") => self.brush = tfm_turn(self.brush, false),
                    ("File", "Open") => self.open_dialog(ctx),
                    ("File", "Close") => self.close_active(ctx),
                    ("File", "Save") => self.save(ctx),
                    ("File", "Save as") => self.save_as(ctx),
                    ("File", "Undo") => self.undo(ctx),
                    ("File", "Redo") => self.redo(ctx),
                    ("Operations", "Crop") => self.crop(ctx),
                    ("File", "Quit") => {
                        // The same guard Escape raises: the box asks,
                        // the answer decides. Unlike the key — which
                        // first lets go of anything held — a named
                        // click means the question, so the box opens
                        // whatever the bench holds; the held state
                        // waits through the question untouched.
                        self.quit_prompt = true;
                        self.status = String::from("quit? — Enter/Y leaves, Escape/N stays");
                    }
                    _ => {}
                }
            }
            // A grey travelled its slider: the value lands quietly — the
            // checker re-greys live, every frame — and the status note
            // keeps the last real command's words.
            Some(frost::MenuEvent::Slid(_, line, v)) => match line {
                "Light" => self.light = v,
                "Dark" => self.dark = v,
                _ => {}
            },
            None => {}
        }

        // The collected button presses, now outside every panel closure.

        // The animation edits. The duration slider and loop checkbox
        // wrote their locals; commit them, then act on the buttons.
        let time_moved = self
            .anim
            .frames
            .get(self.anim.frame)
            .is_some_and(|f| f.next_time != next_time);
        if time_moved {
            // The duration slider steps into history as it moves; each
            // step is a command the user dragged through.
            self.stamp();
        }
        if let Some(f) = self.anim.frames.get_mut(self.anim.frame) {
            f.next_time = next_time;
        }
        if self.anim.looping != looping {
            self.stamp();
            self.anim.looping = looping;
        }
        if want_add_frame {
            // A new frame, inserted after the one being edited, opens
            // with the active sprite as its first layer.
            let at = (self.anim.frame + 1).min(self.anim.frames.len());
            self.stamp();
            let shot = self
                .active()
                .and_then(|sp| png_bytes(&sp.current).ok())
                .and_then(|png| {
                    frost::Shape::sprite_bytes_nearest(&png)
                        .ok()
                        .map(|s| (s, png))
                });
            self.anim.frames.insert(
                at,
                Frame {
                    layers: shot
                        .map(|(shape, png)| vec![Layer { shape, png }])
                        .unwrap_or_default(),
                    next_time: TIME_DEFAULT,
                },
            );
            self.anim.set_frame(at);
            self.sync_work(ctx);
        }
        if want_add_layer {
            let shot = self
                .active()
                .and_then(|sp| png_bytes(&sp.current).ok())
                .and_then(|png| {
                    frost::Shape::sprite_bytes_nearest(&png)
                        .ok()
                        .map(|s| (s, png))
                });
            let room = self
                .anim
                .frames
                .get(self.anim.frame)
                .is_some_and(|f| f.layers.len() < LAYER_NODES);
            if shot.is_some() && room {
                self.stamp();
                if let Some((shape, png)) = shot
                    && let Some(f) = self.anim.frames.get_mut(self.anim.frame)
                {
                    f.layers.push(Layer { shape, png });
                }
            }
            self.sync_work(ctx);
        }
        if want_del_layer {
            let some = self
                .anim
                .frames
                .get(self.anim.frame)
                .is_some_and(|f| !f.layers.is_empty());
            if some {
                self.stamp();
                if let Some(f) = self.anim.frames.get_mut(self.anim.frame) {
                    f.layers.pop();
                }
            }
            self.sync_work(ctx);
        }
        if want_del_frame {
            if self.anim.frame < self.anim.frames.len() {
                self.stamp();
                self.anim.frames.remove(self.anim.frame);
                self.anim.set_frame(
                    self.anim
                        .frame
                        .min(self.anim.frames.len().saturating_sub(1)),
                );
            }
            self.sync_work(ctx);
        }
        if want_prev_frame || want_next_frame {
            let n = self.anim.frames.len();
            if n > 0 {
                let delta = if want_next_frame { 1 } else { -1 };
                let nf = (self.anim.frame as isize + delta).rem_euclid(n as isize) as usize;
                self.anim.set_frame(nf);
                self.sync_work(ctx);
            }
        }
        if want_play {
            self.anim.playing = !self.anim.playing;
            self.anim.dir = 1;
        }

        // Ctrl, polled once: Ctrl + left is the mouse hand's
        // eraser, twin of the Delete key.
        let ctrl_held =
            ctx.key_down(frost::KeyCode::ControlLeft) || ctx.key_down(frost::KeyCode::ControlRight);
        // The keyboard eraser's state: held drags erase, a tap erases
        // the one cell under the cursor.
        let del = ctx.key_down(frost::KeyCode::Delete) || ctx.key_down(frost::KeyCode::Backspace);
        let del_edge = del && !self.was_del;
        self.was_del = del;
        // Shift selects; Escape drops whatever is held.
        let shift_held =
            ctx.key_down(frost::KeyCode::ShiftLeft) || ctx.key_down(frost::KeyCode::ShiftRight);
        let esc = ctx.key_down(frost::KeyCode::Escape);
        let esc_edge = esc && !self.was_block_esc;
        self.was_block_esc = esc;
        // The left button's work: press lands on a slot, on the work area
        // or nowhere the demo owns; release decides — click or drag.
        // The status band answers a click anywhere on it: the band
        // toggles the panel, and an open panel takes the press for
        // its own rows and thumbs. Either way the press is the bar's,
        // and the chain below never hears it.
        let mut log_hit = false;
        if pressed
            && !self.ui.hovering()
            && let Some(p) = pos
        {
            if in_rect(log_bar_rect(w, h), p) {
                self.log_open = !self.log_open;
                if self.log_open {
                    self.log_sel = 0;
                    self.log_top = 0;
                    self.log_sx = 0.0;
                }
                log_hit = true;
            } else if self.log_open && in_rect(log_panel_rect(w, h), p) {
                self.log_press(p, w, h);
                log_hit = true;
            }
        }
        if pressed
            && !self.ui.hovering()
            && !log_hit
            && let Some(p) = pos
        {
            // The strip's scrollbar claims its bottom band first; the
            // plates sit above it and never share its pixels.
            let scrub = sv.scroll_max > 0.0
                && p[1] <= floor_y(h) + (SCROLL_Y + SCROLL_H / 2.0)
                && p[0] >= sv.x0
                && p[0] <= sv.x0 + sv.track_w;
            if scrub {
                self.slot_scrub = Some(p[0] - scroll_knob(&sv, self.slot_scroll).0);
            } else if tile_view && bplate.is_some_and(|r| p_in(Some(p), r)) {
                // The panel claims a press anywhere on its plate: it
                // rides the cursor until release, and release
                // decides — a click picks the cell under it, a drag
                // leaves the panel where it landed.
                self.band_drag = Some(([p[0] - bor[0], p[1] - bor[1]], p, bor));
            } else {
                match slot_at(p, w, h, self.slot_scroll, slot_count) {
                    Some(i) => self.slot_drag = Some((i, p)),
                    None if in_work_area(p, w, h) && !tile_view => self.drag_from = Some(p),
                    // Shift + left drags a selection: release copies
                    // the rectangle into a block brush. The frame
                    // stays to show what is held.
                    None if tile_view && shift_held && in_work_area(p, w, h) => {
                        if let Some((c, r)) = desk_cell(self.active(), p, view, self.zoom) {
                            self.sel_drag = Some([c, r]);
                            self.sel = Some([c, r, c, r]);
                        }
                    }
                    // Ctrl + left is the mouse hand's eraser: the
                    // same stroke Delete opens — a click lifts the
                    // cell under it, a drag sweeps a path.
                    None if tile_view && (ctrl_held || del) && in_work_area(p, w, h) => {
                        self.paint = Some(Paint {
                            erase: true,
                            touched: false,
                            n: 0,
                            key_opened: false,
                        });
                    }
                    // A held block is the brush now: one click stamps
                    // the whole picture, one undo step for all of it.
                    // A click INSIDE the selection frame lays the
                    // block back over the frame — dress the block and
                    // click again to rotate in place, nothing left
                    // a click drops a copy cursor-anchored, gaps
                    // skipped; the held block never touches anything
                    // outside its own cells.
                    None if tile_view && self.clip.is_some() && in_work_area(p, w, h) => {
                        if let Some((col, row)) = desk_cell(self.active(), p, view, self.zoom) {
                            let before = self.capture();
                            let n = match &self.clip {
                                Some(clip) => clip_paste(&mut self.maps, clip, col, row),
                                None => 0,
                            };
                            log::info!("block: pasted at ({col},{row}) wrote {n}");
                            if n > 0 {
                                push_step(&mut self.undo_stack, &mut self.redo_stack, before);
                                self.status = format!(
                                    "pasted {n} cell{} — the block is still held",
                                    if n == 1 { "" } else { "s" }
                                );
                                self.sync_work(ctx);
                            }
                        }
                    }
                    None if tile_view && in_work_area(p, w, h) && self.picked_cell.is_some() => {
                        self.paint = Some(Paint {
                            erase: false,
                            touched: false,
                            n: 0,
                            key_opened: false,
                        });
                    }
                    None => {}
                }
            }
        }
        // Right now only dresses the held tile, and it is safe to
        // press anywhere on the desk: plain turns the brush a quarter
        // counterclockwise, Shift clockwise, Alt flips it over the
        // horizontal, Ctrl over the vertical. On the panel the pressed
        // cell becomes the brush as it dresses; on the desk the brush
        // dresses where it stands. Erasing rides Delete or Ctrl —
        // seen above and below. (Elsewhere right still spot-picks.)
        if rpressed
            && tile_view
            && let Some(p) = pos
            && !over_ron
            && !self.log_open
            && !self.ui.hovering()
            && (band_pick(p, brows, bcols, bcell, bor, bcells).is_some() || in_work_area(p, w, h))
        {
            let shift =
                ctx.key_down(frost::KeyCode::ShiftLeft) || ctx.key_down(frost::KeyCode::ShiftRight);
            let alt =
                ctx.key_down(frost::KeyCode::AltLeft) || ctx.key_down(frost::KeyCode::AltRight);
            let ctrl = ctx.key_down(frost::KeyCode::ControlLeft)
                || ctx.key_down(frost::KeyCode::ControlRight);
            let dressed = match dress_gesture(shift, ctrl, alt) {
                Dress::FlipY => "flipped y",
                Dress::FlipX => "flipped x",
                Dress::Turn(true) => "turned cw",
                Dress::Turn(false) => "turned ccw",
            };
            self.status = match band_pick(p, brows, bcols, bcell, bor, bcells) {
                Some(i) => {
                    // The rack is single-tile country: picking there
                    // drops the block and dresses one tile instead.
                    self.picked_cell = Some(i);
                    self.clip = None;
                    self.sel = None;
                    self.brush = dress_brush(self.brush, dress_gesture(shift, ctrl, alt));
                    format!("tile {} {dressed} — {}", i + 1, turn_note(self.brush))
                }
                None => {
                    if let Some(clip) = &mut self.clip {
                        // A block is held: every verb works the whole
                        // picture — the arrangement and each tile in
                        // it turn and mirror together.
                        dress_block(clip, &mut self.sel, dress_gesture(shift, ctrl, alt));
                        log::info!(
                            "block: {dressed} -> {}x{} cells={:?} tfms={:?} sel={:?}",
                            clip.cols,
                            clip.rows,
                            clip.cells,
                            clip.tfms,
                            self.sel
                        );
                        format!("block {dressed}")
                    } else {
                        self.brush = dress_brush(self.brush, dress_gesture(shift, ctrl, alt));
                        format!("brush {dressed} — {}", turn_note(self.brush))
                    }
                }
            };
        }
        // The eraser rides the keyboard: hold Delete (or Backspace)
        // and click or drag across the desk — or tap the key over
        // the map and the cell under the cursor comes off. Ctrl +
        // click, seen above, opens the very same stroke.
        if del_edge
            && tile_view
            && !over_ron
            && !self.log_open
            && !self.ui.hovering()
            && pos.is_some_and(|p| in_work_area(p, w, h))
        {
            if let Some(sel) = self.sel {
                // A selection standing on the desk: Delete lifts the
                // whole rectangle — and the held block stays, so the
                // sweep can be pasted elsewhere: a move in two beats.
                self.sel = None;
                let before = self.capture();
                if erase_rect(&mut self.maps, sel) {
                    push_step(&mut self.undo_stack, &mut self.redo_stack, before);
                    self.status = String::from("selection erased — click to paste it");
                } else {
                    self.status = String::from("the selection was already clear");
                }
                self.sync_work(ctx);
            } else {
                self.paint = Some(Paint {
                    erase: true,
                    touched: false,
                    n: 0,
                    key_opened: true,
                });
            }
        }
        if self.paint.is_some_and(|st| st.key_opened && !del) {
            self.paint = None;
        }
        // Escape drops the block and its frame; a picked tile brush,
        // if one stands, rides on untouched.
        if esc_edge && (self.clip.is_some() || self.sel.is_some() || self.sel_drag.is_some()) {
            self.clip = None;
            self.sel = None;
            self.sel_drag = None;
            self.status = String::from("selection dropped");
        }
        // The stroke, continued: every frame the painting button is
        // held, the cell under the cursor is written. The undo step is
        // taken on the first cell the stroke actually changes — a
        // click on empty space outside the map stays a no-op all the
        // way down, road unstained.
        let stroke = self
            .paint
            .map(|st| (st.erase, stroke_held(&st, down, del || ctrl_held)));
        if let Some((erase, true)) = stroke
            && let Some(p) = pos
        {
            let brush = self.active().map(|sp| {
                let (arows, acols) = sp.atlas.unwrap_or((1, 1));
                (
                    TilesetRef::of(sp),
                    sp.current.width() as f32 / acols.max(1) as f32,
                    sp.current.height() as f32 / arows.max(1) as f32,
                )
            });
            if let Some((cut, tw, th)) = brush {
                let mx = (p[0] - view[0]) / self.zoom;
                let my = (p[1] - view[1]) / self.zoom;
                let (col, row) = grid_cell(mx, my, tw, th);
                // The road keeps the world before the stroke's
                // first change — the snapshot is taken as the write
                // is about to happen, not after it happened.
                let before = self.paint.filter(|st| !st.touched).map(|_| self.capture());
                let changed = if erase {
                    erase_at(&mut self.maps, col, row)
                } else if let Some(cell) = self.picked_cell {
                    paint_at(&mut self.maps, &cut, cell, self.brush, col, row)
                } else {
                    false
                };
                if changed {
                    if let Some(before) = before {
                        push_step(&mut self.undo_stack, &mut self.redo_stack, before);
                    }
                    if let Some(st) = &mut self.paint {
                        st.touched = true;
                        st.n += 1;
                    }
                    self.sync_work(ctx);
                }
            }
        }
        // Release closes the stroke, and a stroke that painted says
        // how many cells it moved.
        for (edge, erase, word) in [(released, false, "painted"), (rreleased, true, "erased")] {
            if edge
                && self.paint.is_some_and(|st| st.erase == erase)
                && let Some(st) = self.paint.take()
                && st.touched
            {
                self.status = format!("{word} {} cell{}", st.n, if st.n == 1 { "" } else { "s" });
            }
        }
        // The selection rides the cursor while Shift + left is held;
        // its release harvests — the rectangle becomes a block brush,
        // and the frame stays to show what the desk is holding.
        if let Some([c0, r0]) = self.sel_drag {
            if let (true, Some(p)) = (down, pos) {
                if let Some((c, r)) = desk_cell(self.active(), p, view, self.zoom) {
                    self.sel = Some([c0.min(c), r0.min(r), c0.max(c), r0.max(r)]);
                }
            } else {
                self.sel_drag = None;
                let cut = self.active().map(TilesetRef::of);
                match (self.sel, cut) {
                    (Some([ca, ra, cb, rb]), Some(cut)) if tile_view => {
                        let mut clip = clip_take(
                            &self.maps,
                            &cut,
                            ca,
                            ra,
                            (cb - ca + 1) as usize,
                            (rb - ra + 1) as usize,
                        );
                        match clip_trim(&mut clip) {
                            Some((c0, r0, c1, r1)) => {
                                // The frame shrinks to the picture:
                                // turns pivot on it and drops hug it.
                                self.sel = Some([
                                    ca + c0 as i32,
                                    ra + r0 as i32,
                                    ca + c1 as i32,
                                    ra + r1 as i32,
                                ]);
                                let (fw, fh) = ((cb - ca + 1) as usize, (rb - ra + 1) as usize);
                                log::info!(
                                    "block: took sel [{ca},{ra},{cb},{rb}] trimmed to {:?} -> {}x{} cells={:?} tfms={:?}",
                                    self.sel,
                                    clip.cols,
                                    clip.rows,
                                    clip.cells,
                                    clip.tfms
                                );
                                self.status = if (clip.cols, clip.rows) == (fw, fh) {
                                    format!(
                                        "selected {}\u{d7}{} — click to paste, Delete clears the source",
                                        clip.cols, clip.rows
                                    )
                                } else {
                                    format!(
                                        "selected {}\u{d7}{} of {fw}\u{d7}{fh} — the frame trimmed to the picture",
                                        clip.cols, clip.rows
                                    )
                                };
                                self.clip = Some(clip);
                            }
                            None => {
                                self.sel = None;
                                self.status = "the selection held nothing".to_string();
                            }
                        }
                    }
                    _ => self.sel = None,
                }
            }
        }
        // The sidecar views' trees, one widget frame each: the widget
        // claims the close boxes, corner grips and thumbs in view order,
        // drives the scrolls, resizes and reorders, and reports what the
        // release hit.
        let mut bufs: Vec<Vec<frost::TreeLine>> =
            self.ron_views.iter().map(|_| Vec::new()).collect();
        let mut specs: Vec<frost::TreeSpec> = Vec::new();
        let mut slots: Vec<usize> = Vec::new();
        for (v, buf) in self.ron_views.iter().zip(bufs.iter_mut()) {
            if let Some(spec) = ron_spec(&self.sprites, self.active, v, buf) {
                specs.push(spec);
                slots.push(v.slot);
            }
        }
        let outs = self.ui.tree(&specs, &mut self.ron_state);
        let mut closed = None;
        for (slot, out) in slots.iter().zip(&outs) {
            if let Some(doc) = self.sprites.get_mut(*slot).and_then(|sp| sp.ron.as_mut()) {
                doc.scroll = out.scroll;
                doc.sx = out.sx;
                doc.vw = out.size[0];
                doc.vh = out.size[1];
                doc.clamp_scroll();
            }
            for ev in &out.events {
                match ev {
                    frost::TreeEvent::Close => closed = Some(*slot),
                    frost::TreeEvent::Row { row } => self.row_click(ctx, *slot, *row),
                    frost::TreeEvent::Add { row } => self.row_add(*slot, *row),
                    frost::TreeEvent::Del { row } => self.row_del(ctx, *slot, *row),
                    frost::TreeEvent::DragStart { row } => self.ron_drag_start(*slot, *row),
                    frost::TreeEvent::Drag { row } => self.ron_drag_move(*slot, *row),
                    frost::TreeEvent::DragEnd => self.ron_drag = None,
                }
            }
        }
        if let Some(slot) = closed {
            self.close_slot(ctx, slot);
        }
        // A work-area drag draws the crop selection in the texture's
        // pixel space.
        if down && let (Some(from), Some(p)) = (self.drag_from, pos) {
            self.selection = sel_rect_from(from, p, size, view, self.zoom);
        }
        if released {
            if let Some(from) = self.drag_from.take()
                && let Some([mx, my]) = pos
            {
                let moved = ((mx - from[0]).powi(2) + (my - from[1]).powi(2)).sqrt();
                if moved < CLICK_TOL {
                    // A click on the work area: log the sprite pixel and
                    // clear the selection. The active sprite sits at the
                    // work area's center plus `offset`, at scale `zoom`,
                    // so window coordinates map to texture pixels with
                    // `px = (x - vx) / zoom + w/2` and
                    // `py = h/2 - (y - vy) / zoom` — the y flip included,
                    // since the texture's y grows down.
                    if !self.sprites.is_empty() {
                        let [tw, th] = size;
                        let [px, py] = tex_point([mx, my], size, view, self.zoom);
                        let inside = px >= 0.0 && px <= tw && py >= 0.0 && py <= th;
                        let name = self.active().map_or("", |sp| sp.name.as_str());
                        log::info!(
                            "click at sprite pixel ({px:.1}, {py:.1}) of {tw:.0}x{th:.0} '{name}' — {where}",
                            where = if inside {
                                "inside the texture"
                            } else {
                                "outside the texture"
                            }
                        );
                        self.last_click = Some(([px, py], inside));

                        // A position picked in the sidecar panel: this
                        // click IS the edit — the tree's two numbers
                        // become this pixel, and the rows re-flatten to
                        // show them.
                        let mut say = None;
                        if inside
                            && self.active().is_some_and(|sp| {
                                sp.ron.as_ref().is_some_and(|doc| doc.edit.is_some())
                            })
                        {
                            // Moving or placing a mark: the stroke's
                            // before-picture goes down before the tree
                            // takes the new numbers.
                            self.stamp();
                        }
                        if inside
                            && let Some(sp) = self.sprites.get_mut(self.active)
                            && let Some(doc) = &mut sp.ron
                            && let Some(pi) = doc.edit
                        {
                            let mut spots = Vec::new();
                            scan_spots(&doc.root, &mut Vec::new(), "", &mut spots);
                            if let Some(spot) = spots.get(pi) {
                                let (path, label) = (spot.path.clone(), spot.label.clone());
                                if let Some(v) = ron_tree::walk(&mut doc.root, &path)
                                    && let ron_tree::Val::Struct { fields, .. } = v
                                {
                                    for (n, f) in fields.iter_mut().enumerate().take(2) {
                                        f.1.val = ron_tree::Val::Atom(
                                            format!("{:.1}", [px, py][n]),
                                            ron_tree::Kind::Num,
                                        );
                                    }
                                    doc.rows = ron_view::layout(&doc.root);
                                    say = Some(format!("moved '{label}' to ({px:.1}, {py:.1})"));
                                }
                            }
                        }
                        if let Some(t) = say {
                            self.status = t;
                        }
                    }
                    self.selection = None;
                }
            }
            // A slot press: a click activates its sprite; a drag that
            // ends on another slot swaps the two sprites' places.
            if let Some((from_slot, from)) = self.slot_drag.take()
                && let Some(p) = pos
            {
                let moved = ((p[0] - from[0]).powi(2) + (p[1] - from[1]).powi(2)).sqrt();
                if moved < CLICK_TOL {
                    if from_slot < self.sprites.len() {
                        self.active = from_slot;
                        self.selection = None;
                        self.last_click = None;
                        self.sync_work(ctx);
                    } else {
                        // The spare plate: pressing it opens the
                        // dialog, and the scroll parks at the far end
                        // so the newcomer is in view when it lands.
                        self.open_dialog(ctx);
                        // Park at the far end so the newcomer lands
                        // in view. Not f32::MAX: the thumb strip
                        // divides the scroll this very frame, and a
                        // parked-infinity walked straight off the end
                        // of `usize` — a debug-build panic.
                        self.slot_scroll = sv.scroll_max;
                    }
                } else if from_slot < self.sprites.len()
                    && let Some(to) = slot_at(p, w, h, self.slot_scroll, self.sprites.len() + 1)
                    && to != from_slot
                    && to < self.sprites.len()
                {
                    self.stamp();
                    self.sprites.swap(from_slot, to);
                    self.active = swapped_active(self.active, from_slot, to);
                    self.refresh_slots(ctx);
                }
            }
            // The panel drag ends: a click picks the cell under the
            // cursor — the panel never moved — and a drag keeps the
            // panel where it landed (already clamped by the frame's
            // own geometry).
            if let Some((_, from, _)) = self.band_drag.take()
                && let Some(p) = pos
            {
                let moved = ((p[0] - from[0]).powi(2) + (p[1] - from[1]).powi(2)).sqrt();
                if moved < CLICK_TOL {
                    if let Some(i) = band_pick(p, brows, bcols, bcell, bor, bcells) {
                        self.picked_cell = Some(i);
                        self.clip = None;
                        self.sel = None;
                        let ctrl = ctx.key_down(frost::KeyCode::ControlLeft)
                            || ctx.key_down(frost::KeyCode::ControlRight);
                        self.status = if ctrl {
                            self.brush = tfm_flip_x(self.brush);
                            format!("tile {} flipped x — {}", i + 1, turn_note(self.brush))
                        } else {
                            format!("tile {} picked", i + 1)
                        };
                    }
                } else {
                    self.band_origin = Some(bor);
                }
            }
        }

        // A held scrollbar knob rides the cursor; letting go ends it.
        if let Some(grab) = self.slot_scrub
            && let Some(p) = pos
        {
            let kw = scroll_knob(&sv, self.slot_scroll).1;
            let f = ((p[0] - grab) - sv.x0) / (sv.track_w - kw).max(1.0);
            self.slot_scroll = (f * sv.scroll_max).clamp(0.0, sv.scroll_max);
        }
        if !down {
            self.slot_scrub = None;
        }

        // A held log thumb rides the cursor: the vertical one drives
        // the shown window, the horizontal one slides long lines.
        // Letting go ends the ride; the selection keeps its place.
        if let Some((vert, grab)) = self.log_grab
            && let Some(p) = pos
        {
            let body = log_body(log_panel_rect(w, h));
            let len = self.log.len();
            if vert {
                self.log_top = log_top_at(len, body[3] - body[1], body[3] - (p[1] - grab));
            } else {
                let view_w = body[2] - body[0];
                let content = log_wide(&self.log) as f32 * RON_ADV;
                self.log_sx = log_sx_at(view_w, content, view_w, p[0] - grab - body[0]);
            }
        }
        if !down {
            self.log_grab = None;
        }

        // The status band: the bench's voice at the window's bottom
        // edge — the newest message at the left, the view's own name
        // pinned at the right — and one click away, the log: the last
        // messages as docked rows, whose thumbs answer drag, wheel
        // and arrows alike.
        let bar = log_bar_rect(w, h);
        let bcy = (bar[1] + bar[3]) / 2.0;
        ctx.rectangle(0.0, bcy, w / 2.0, STATUS_H / 2.0, LOG_BAND, LOG_Z);
        let view_name = self.view.name();
        let name_w = view_name.chars().count() as f32 * RON_ADV;
        ctx.text(
            bar[2] - LOG_INSET - name_w / 2.0,
            bcy - RON_LIFT * RON_SIZE,
            &self.ron_font,
            view_name,
            RON_SIZE,
            700.0,
            LOG_TEXT,
            LOG_Z + 0.5,
        );
        if !self.status.is_empty() {
            let room = ((bar[2] - bar[0] - 3.0 * LOG_INSET - name_w) / RON_ADV).max(0.0) as usize;
            let shown = log_head(&self.status, room);
            let wd = shown.chars().count() as f32 * RON_ADV;
            ctx.text(
                bar[0] + LOG_INSET + wd / 2.0,
                bcy - RON_LIFT * RON_SIZE,
                &self.ron_font,
                shown,
                RON_SIZE,
                400.0,
                LOG_TEXT,
                LOG_Z + 0.5,
            );
        }
        if self.log_open {
            let panel = log_panel_rect(w, h);
            let body = log_body(panel);
            ctx.rectangle(
                (panel[0] + panel[2]) / 2.0,
                (panel[1] + panel[3]) / 2.0,
                (panel[2] - panel[0]) / 2.0,
                (panel[3] - panel[1]) / 2.0,
                LOG_PLATE,
                LOG_Z,
            );
            let len = self.log.len();
            if len == 0 {
                ctx.text(
                    0.0,
                    (body[1] + body[3]) / 2.0 - RON_LIFT * RON_SIZE,
                    &self.ron_font,
                    "the bench has said nothing yet",
                    RON_SIZE,
                    400.0,
                    LOG_TEXT,
                    LOG_Z + 0.5,
                );
            }
            // Rows slice by the character, as the sidecar trees do:
            // the shared x offset decides the first visible character.
            let start = (self.log_sx / RON_ADV) as usize;
            let vis = ((body[2] - body[0] - 2.0 * LOG_INSET) / RON_ADV).max(0.0) as usize;
            for d in self.log_top..(self.log_top + LOG_ROWS).min(len) {
                let Some(msg) = log_line(&self.log, d) else {
                    continue;
                };
                let y = body[3] - (d - self.log_top) as f32 * LOG_LINE - LOG_LINE / 2.0;
                if d == self.log_sel {
                    ctx.rectangle(
                        (body[0] + body[2]) / 2.0,
                        y,
                        (body[2] - body[0]) / 2.0 - 4.0,
                        LOG_LINE / 2.0 - 1.0,
                        SELECT,
                        LOG_Z + 0.2,
                    );
                }
                let shown: String = msg.chars().skip(start).take(vis).collect();
                let wd = shown.chars().count() as f32 * RON_ADV;
                ctx.text(
                    body[0] + LOG_INSET + wd / 2.0,
                    y - RON_LIFT * RON_SIZE,
                    &self.ron_font,
                    shown,
                    RON_SIZE,
                    if d == self.log_sel { 700.0 } else { 400.0 },
                    LOG_TEXT,
                    LOG_Z + 0.5,
                );
            }
            // Thumbs: proportional, vertical on the strip at the
            // body's right, horizontal under the rows — each shown
            // only when there is somewhere to go.
            let track_h = body[3] - body[1];
            let (off, kh) = log_v_thumb(len, track_h, self.log_top);
            if len > LOG_ROWS {
                ctx.rectangle(
                    body[2] + LOG_TRACK_W / 2.0,
                    (body[1] + body[3]) / 2.0,
                    LOG_TRACK_W / 2.0 - 3.0,
                    track_h / 2.0,
                    LOG_BAND,
                    LOG_Z + 0.3,
                );
                ctx.rectangle(
                    body[2] + LOG_TRACK_W / 2.0,
                    body[3] - off - kh / 2.0,
                    LOG_THICK / 2.0,
                    kh / 2.0,
                    LOG_THUMB,
                    LOG_Z + 0.4,
                );
            }
            let view_w = body[2] - body[0];
            let content = log_wide(&self.log) as f32 * RON_ADV;
            let (hoff, hw) = log_h_thumb(view_w, content, view_w, self.log_sx);
            if content > view_w {
                ctx.rectangle(
                    body[0] + hoff + hw / 2.0,
                    panel[1] + LOG_PAD / 2.0,
                    hw / 2.0,
                    LOG_THICK / 2.0,
                    LOG_THUMB,
                    LOG_Z + 0.4,
                );
            }
        }

        // The frame's one geometry, filled once and read by every
        // painter that follows.
        let f = FrameState {
            w,
            h,
            view,
            size,
            tile_view,
            pos,
            sv,
            slot_count,
            brows,
            bcols,
            bcell,
            brects,
            bcells,
            bplate,
        };
        self.draw_layers(ctx, &f);

        let bbox = self.draw_checker(ctx, &f);

        self.draw_desk_lines(ctx, &f, bbox, authored);

        self.draw_strip(ctx, &f);

        self.draw_tileset_matrix(ctx, &f);

        self.draw_trees(ctx);

        let mut spots: Vec<Spot> = Vec::new();
        if let Some(doc) = self.ron() {
            scan_spots(&doc.root, &mut Vec::new(), "", &mut spots);
        }
        let editing = self.ron().and_then(|d| d.edit);
        self.draw_spot_markers(ctx, &f, &spots, editing);

        // The usage line, pinned near the top edge, so resizing keeps it
        // in place.
        let help = &mut ctx.scene().root.children[HELP];
        help.transform = frost::Transform::translate([0.0, h / 2.0 - 28.0]);

        // The marker dot rides the last click in window space — the
        // inverse of the conversion above — and shrinks away when there
        // is none.
        let marker = &mut ctx.scene().root.children[MARKER_NODE];
        if let Some(([px, py], _)) = self.last_click {
            let [tw, th] = size;
            marker.transform = frost::Transform::translate([
                (px - tw / 2.0) * self.zoom + view[0],
                (th / 2.0 - py) * self.zoom + view[1],
            ]);
            if let Some(frost::Shape::Circle { radius, .. }) = &mut marker.shape {
                *radius = 4.0;
            }
        } else if let Some(frost::Shape::Circle { radius, .. }) = &mut marker.shape {
            *radius = 0.0;
        }

        self.draw_exit_box(ctx, &f);

        // The cursor's position for next frame's middle-drag delta; `None`
        // (outside the window) clears it, so re-entering never jumps.
        self.last_mouse = pos;
    }
}
