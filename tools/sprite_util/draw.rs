//! The desk's paint: every part of a frame's drawing, in the order
//! the process pass hands it over — layers under, interface over.
//! Each method owns one banner of the frame that was one long
//! statement stream: same order, same paints, named parts.

use crate::art::*;
use crate::band::*;
use crate::desk::*;
use crate::layout::*;
use crate::map::*;
use crate::ron_panels::*;
use crate::sidecar::*;
use crate::spots::*;
use crate::strip::*;
use crate::theme::*;

impl Demo {
    /// The work area's layer pool: pan, zoom, and the
    /// tile desk's clip.
    pub(crate) fn draw_layers(&mut self, ctx: &mut frost::Context, f: &FrameState) {
        // The work area's layer pool: every node carries the pan and the
        // zoom — a uniform scale about its center, then the pan offset
        // from the work area's center. The shapes come from the sync.
        for node in &mut ctx.scene().root.children[..LAYER_NODES] {
            node.transform = frost::Transform::translate(f.view);
            node.scale = [self.zoom, self.zoom];
        }
        // On the map desk the art stops at the work area: the tile
        // maps' clip is the work rectangle seen through the desk's own
        // transform (scale then pan), so no zoom or pan ever spills a
        // tile over the slots strip or the menu bar.
        if f.tile_view {
            let clip = [
                (-f.w / 2.0 - f.view[0]) / self.zoom,
                (floor_y(f.h) + STRIP_H - f.view[1]) / self.zoom,
                (f.w / 2.0 - f.view[0]) / self.zoom,
                (f.h / 2.0 - f.view[1]) / self.zoom,
            ];
            for node in &mut ctx.scene().root.children[..LAYER_NODES] {
                if let Some(shape) = &mut node.shape {
                    shape.set_clip(Some(clip));
                }
            }
        }
    }

    /// The transparency checkerboard under the sprite.
    pub(crate) fn draw_checker(
        &mut self,
        ctx: &mut frost::Context,
        f: &FrameState,
    ) -> (f32, f32, f32, f32) {
        // The checker backdrop: the sprite's on-screen bounding box,
        // clipped to the work area, filled with cells that never scale
        // with the sprite — so the pattern reveals the sprite's
        // transparency no matter the zoom or the pan. The fill is a tiny
        // sprite, one texture pixel per cell, sampled with
        // nearest-neighbor filtering so the cell edges stay hard; the
        // texture is rebuilt only when the cell count or a grey level
        // changes, so smooth pans and zooms only reposition and rescale
        // it.
        let [tw, th] = f.size;
        let (hw, hh) = ((tw * self.zoom) / 2.0, (th * self.zoom) / 2.0);
        let (bx0, by0) = (f.view[0] - hw, f.view[1] - hh);
        let (bx1, by1) = (f.view[0] + hw, f.view[1] + hh);
        // The box's intersection with the work area — or, on the
        // tile-map desk, the work area entire: the map has no bounds
        // the checker should respect, so the pattern fills the canvas.
        let (rx0, ry0, rx1, ry1) = if f.tile_view {
            (-f.w / 2.0, floor_y(f.h) + STRIP_H, f.w / 2.0, f.h / 2.0)
        } else {
            (
                bx0.max(-f.w / 2.0),
                by0.max(floor_y(f.h) + STRIP_H),
                bx1.min(f.w / 2.0),
                by1.min(f.h / 2.0),
            )
        };
        let checker = &mut ctx.scene().root.children[CHECKER];
        if rx1 > rx0 && ry1 > ry0 && (tw > 0.0 || f.tile_view) {
            let cw = ((rx1 - rx0) / CHECK_CELL).ceil() as u32;
            let ch = ((ry1 - ry0) / CHECK_CELL).ceil() as u32;
            let key = (
                cw,
                ch,
                (self.light * 255.0).round() as u8,
                (self.dark * 255.0).round() as u8,
            );
            if key != self.checker_key {
                self.checker_key = key;
                checker.shape = checker_shape(cw, ch, self.light, self.dark);
            }
            checker.transform = frost::Transform::translate([(rx0 + rx1) / 2.0, (ry0 + ry1) / 2.0]);
            // One texture pixel per cell; the scale stretches the
            // sub-pixel last row and column so the pattern covers the
            // region exactly.
            checker.scale = [(rx1 - rx0) / cw as f32, (ry1 - ry0) / ch as f32];
        } else {
            // No sprite (or its box off the work area): hide the backdrop
            // and force a rebuild when it comes back.
            checker.shape = None;
            self.checker_key = (0, 0, 0, 0);
        }

        (bx0, by0, bx1, by1)
    }

    pub(crate) fn draw_desk_lines(
        &mut self,
        ctx: &mut frost::Context,
        f: &FrameState,
        bbox: (f32, f32, f32, f32),
        authored: Vec<frost::Authored>,
    ) {
        let [tw, th] = f.size;

        // The desk's own lines stop at the work area's edge: no
        // bounding box, atlas grid or crop selection escapes onto the
        // slots strip or past the top of the window.
        let (wx0, wy0, wx1, wy1) = (-f.w / 2.0, floor_y(f.h) + STRIP_H, f.w / 2.0, f.h / 2.0);
        let clip_v = |x: f32, y0: f32, y1: f32| -> Option<(f32, f32)> {
            let (a, b) = (y0.max(wy0), y1.min(wy1));
            (x >= wx0 && x <= wx1 && b > a).then_some((a, b))
        };
        let clip_h = |y: f32, x0: f32, x1: f32| -> Option<(f32, f32)> {
            let (a, b) = (x0.max(wx0), x1.min(wx1));
            (y >= wy0 && y <= wy1 && b > a).then_some((a, b))
        };
        // The bounding box: the sprite's full on-screen rectangle,
        // cut to the desk.
        if tw > 0.0 && th > 0.0 && !f.tile_view {
            if let Some((a, b)) = clip_h(bbox.1, bbox.0, bbox.2) {
                ctx.line(a, bbox.1, b, bbox.1, BBOX, BBOX_WIDTH, BBOX_Z);
            }
            if let Some((a, b)) = clip_v(bbox.2, bbox.1, bbox.3) {
                ctx.line(bbox.2, a, bbox.2, b, BBOX, BBOX_WIDTH, BBOX_Z);
            }
            if let Some((a, b)) = clip_h(bbox.3, bbox.0, bbox.2) {
                ctx.line(a, bbox.3, b, bbox.3, BBOX, BBOX_WIDTH, BBOX_Z);
            }
            if let Some((a, b)) = clip_v(bbox.0, bbox.1, bbox.3) {
                ctx.line(bbox.0, a, bbox.0, b, BBOX, BBOX_WIDTH, BBOX_Z);
            }
        }

        // The crop selection: the texture-space rectangle mapped back to
        // the window — the same transform the marker dot uses.
        if let Some([x0, y0, x1, y1]) = self.selection.filter(|_| !f.tile_view) {
            let wx = |px: f32| (px - tw / 2.0) * self.zoom + f.view[0];
            let wy = |py: f32| (th / 2.0 - py) * self.zoom + f.view[1];
            let (sx0, sx1) = (wx(x0), wx(x1));
            let (sy0, sy1) = (wy(y0), wy(y1));
            if let Some((a, b)) = clip_h(sy0, sx0, sx1) {
                ctx.line(a, sy0, b, sy0, SELECT, BBOX_WIDTH, SELECT_Z);
            }
            if let Some((a, b)) = clip_v(sx1, sy0, sy1) {
                ctx.line(sx1, a, sx1, b, SELECT, BBOX_WIDTH, SELECT_Z);
            }
            if let Some((a, b)) = clip_h(sy1, sx0, sx1) {
                ctx.line(a, sy1, b, sy1, SELECT, BBOX_WIDTH, SELECT_Z);
            }
            if let Some((a, b)) = clip_v(sx0, sy0, sy1) {
                ctx.line(sx0, a, sx0, b, SELECT, BBOX_WIDTH, SELECT_Z);
            }
        }

        // The tile grid: the atlas' rows and columns mapped onto the
        // sprite, the split drawn over the texture. The inner lines
        // only — the outer ones are the bounding box.
        if let Some((rows, cols)) = self.active().and_then(|sp| sp.atlas)
            && !f.tile_view
            && tw > 0.0
            && th > 0.0
            && (rows > 1 || cols > 1)
        {
            let wx = |px: f32| (px - tw / 2.0) * self.zoom + f.view[0];
            let wy = |py: f32| (th / 2.0 - py) * self.zoom + f.view[1];
            for i in 1..cols {
                let x = wx(tw * i as f32 / cols as f32);
                if let Some((a, b)) = clip_v(x, bbox.1, bbox.3) {
                    ctx.line(x, a, x, b, TILE, TILE_WIDTH, TILE_Z);
                }
            }
            for j in 1..rows {
                let y = wy(th * j as f32 / rows as f32);
                if let Some((a, b)) = clip_h(y, bbox.0, bbox.2) {
                    ctx.line(a, y, b, y, TILE, TILE_WIDTH, TILE_Z);
                }
            }
        }

        // The authored shapes: the sidecar's `shapes:` entries drawn
        // over the sprite in the texture's pixels — the author's own
        // view of the geometry a bake will one day take. Rectangles and
        // polygons get their outline stroked; a circle gets stroked as
        // the 16-gon the light field actually shadows with —
        // `Convex::disc`'s own vertex ring — so what sits on the desk
        // is the edge the shadow will have. Every entry whose geometry
        // read draws, whatever else the reading refused beside it; the
        // refusals speak on the bar's notes. Like the position markers,
        // the overlay is the sprite's own furniture: panned off the
        // desk it rides the art over the strip, the clipping belonging
        // to the desk's lines, not to it.
        if self.show_shapes && !f.tile_view && tw > 0.0 && th > 0.0 {
            for sh in &authored {
                let corners = shape_corners(sh, f.size, f.view, self.zoom);
                for i in 0..corners.len() {
                    let [ax, ay] = corners[i];
                    let [bx, by] = corners[(i + 1) % corners.len()];
                    ctx.line(ax, ay, bx, by, SHAPE, SHAPE_WIDTH, SHAPE_Z);
                }
            }
        }

        // The tile-map grid: the active tileset's cell pitch extended
        // across the whole work area, the map axes drawn louder. The
        // drawn step doubles while a cell shrinks below GRID_MIN_PX on
        // screen, so the grid thins by powers of two — always on whole
        // cells, always a few dozen lines, and the tile coordinates
        // stay legible off any surviving line. With no tileset loaded
        // the pitch falls back to the demo tile, so the desk is never
        // a blank void.
        if f.tile_view {
            let (tx, ty) = self
                .active()
                .map(|sp| {
                    let (rows, cols) = sp.atlas.unwrap_or((1, 1));
                    (
                        sp.current.width() as f32 / cols.max(1) as f32,
                        sp.current.height() as f32 / rows.max(1) as f32,
                    )
                })
                .unwrap_or((32.0, 32.0));
            let (sx, sy) = (grid_step(tx, self.zoom), grid_step(ty, self.zoom));
            let (x0w, y0w, x1w, y1w) = (-f.w / 2.0, floor_y(f.h) + STRIP_H, f.w / 2.0, f.h / 2.0);
            // The work area's corners in map space; the window's y-up
            // and the map's y-up agree, so one sign serves both axes.
            let (mx0, mx1) = ((x0w - f.view[0]) / self.zoom, (x1w - f.view[0]) / self.zoom);
            let (my0, my1) = ((y0w - f.view[1]) / self.zoom, (y1w - f.view[1]) / self.zoom);
            let wx = |x: f32| x * self.zoom + f.view[0];
            let wy = |y: f32| y * self.zoom + f.view[1];
            let cols_at = |n: i64| if n == 0 { AXIS } else { TILE };
            let wid_at = |n: i64| if n == 0 { BBOX_WIDTH } else { TILE_WIDTH };
            let z_at = |n: i64| if n == 0 { AXIS_Z } else { GRID_Z };
            let nx = |v: f32, s: f32| (v / s).floor() as i64;
            let nxe = |v: f32, s: f32| (v / s).ceil() as i64;
            let mut n = nx(mx0, sx);
            while n <= nxe(mx1, sx) {
                let x = wx(n as f32 * sx);
                ctx.line(x, y0w, x, y1w, cols_at(n), wid_at(n), z_at(n));
                n += 1;
            }
            let mut n = nx(my0, sy);
            while n <= nxe(my1, sy) {
                let y = wy(n as f32 * sy);
                ctx.line(x0w, y, x1w, y, cols_at(n), wid_at(n), z_at(n));
                n += 1;
            }
        }
        // The selection's frame: the held rectangle outlined in the
        // picker's blue, cut to the work area like every desk line.
        if f.tile_view
            && let Some([sc0, sr0, sc1, sr1]) = self.sel
        {
            let (sx, sy) = self
                .active()
                .map(|sp| {
                    let (arows, acols) = sp.atlas.unwrap_or((1, 1));
                    (
                        sp.current.width() as f32 / acols.max(1) as f32,
                        sp.current.height() as f32 / arows.max(1) as f32,
                    )
                })
                .unwrap_or((32.0, 32.0));
            let (fx0, fx1) = (
                sc0 as f32 * sx * self.zoom + f.view[0],
                (sc1 + 1) as f32 * sx * self.zoom + f.view[0],
            );
            let (fy0, fy1) = (
                -(sr1 + 1) as f32 * sy * self.zoom + f.view[1],
                -sr0 as f32 * sy * self.zoom + f.view[1],
            );
            if let Some((a, b)) = clip_h(fy0, fx0, fx1) {
                ctx.line(a, fy0, b, fy0, SELECT, BBOX_WIDTH, SELECT_Z);
            }
            if let Some((a, b)) = clip_h(fy1, fx0, fx1) {
                ctx.line(a, fy1, b, fy1, SELECT, BBOX_WIDTH, SELECT_Z);
            }
            if let Some((a, b)) = clip_v(fx0, fy0, fy1) {
                ctx.line(fx0, a, fx0, b, SELECT, BBOX_WIDTH, SELECT_Z);
            }
            if let Some((a, b)) = clip_v(fx1, fy0, fy1) {
                ctx.line(fx1, a, fx1, b, SELECT, BBOX_WIDTH, SELECT_Z);
            }
        }
    }

    /// The slot strip: its floor, its plates, the thumbnails
    /// of the loaded sprites.
    pub(crate) fn draw_strip(&mut self, ctx: &mut frost::Context, f: &FrameState) {
        // The slots strip: its floor, one plate per sprite plus the
        // spare loader plate, the active slot's frame — and, when the
        // plates outrun the pool, a scrollbar along the strip's bottom
        // edge.
        ctx.rectangle(
            0.0,
            floor_y(f.h) + STRIP_H / 2.0,
            f.w / 2.0,
            STRIP_H / 2.0,
            STRIP,
            STRIP_Z,
        );
        for i in 0..f.slot_count {
            let [cx, cy] = slot_center(i, f.w, f.h, self.slot_scroll);
            if cx + SLOT / 2.0 < f.sv.x0 || cx - SLOT / 2.0 > f.sv.x0 + f.sv.visible_px {
                continue;
            }
            let plate = if i < self.sprites.len() {
                PLATE
            } else {
                PLATE_EMPTY
            };
            ctx.rectangle(cx, cy, SLOT / 2.0, SLOT / 2.0, plate, PLATE_Z);
            if i == self.sprites.len() {
                // The spare plate wears a plus: press it for the open
                // dialog.
                let g = SLOT / 5.0;
                ctx.line(cx - g, cy, cx + g, cy, AXIS, 2.0, PLATE_Z + 0.1);
                ctx.line(cx, cy - g, cx, cy + g, AXIS, 2.0, PLATE_Z + 0.1);
            }
            if i == self.active && i < self.sprites.len() {
                let r = SLOT / 2.0;
                ctx.line(cx - r, cy - r, cx + r, cy - r, SELECT, BBOX_WIDTH, FRAME_Z);
                ctx.line(cx + r, cy - r, cx + r, cy + r, SELECT, BBOX_WIDTH, FRAME_Z);
                ctx.line(cx + r, cy + r, cx - r, cy + r, SELECT, BBOX_WIDTH, FRAME_Z);
                ctx.line(cx - r, cy + r, cx - r, cy - r, SELECT, BBOX_WIDTH, FRAME_Z);
            }
        }
        if f.sv.scroll_max > 0.0 {
            let ty = floor_y(f.h) + SCROLL_Y;
            let (kx, kw) = scroll_knob(&f.sv, self.slot_scroll);
            ctx.rectangle(
                f.sv.x0 + f.sv.track_w / 2.0,
                ty,
                f.sv.track_w / 2.0,
                SCROLL_H / 2.0,
                PLATE_EMPTY,
                PLATE_Z + 0.05,
            );
            ctx.rectangle(
                kx + kw / 2.0,
                ty,
                kw / 2.0,
                SCROLL_H / 2.0,
                AXIS,
                PLATE_Z + 0.1,
            );
        }

        // The slot thumbnails: minimized originals, centered on their
        // plates — the pool maps the scrolled window, so scrolling
        // moves which sprite rides which node. A dragged thumbnail
        // rides the cursor, above everything until it lands.
        {
            let first = (self.slot_scroll / f.sv.pitch).floor().max(0.0) as usize;
            for j in 0..SLOTS {
                let node = &mut ctx.scene().root.children[THUMBS + j];
                // Saturating by stubbornness: the scroll is clamped
                // upstream, and an index that runs past the strip
                // should read as nothing, never as a panic.
                let i = first.saturating_add(j);
                if let Some(sp) = self.sprites.get(i) {
                    node.shape = Some(sp.thumb.clone());
                    let dragging = self.slot_drag.is_some_and(|(slot, _)| slot == i);
                    node.order = if dragging { DRAG_ORDER } else { THUMB_ORDER };
                    node.transform = match (dragging, f.pos) {
                        (true, Some(p)) => frost::Transform::translate(p),
                        _ => {
                            frost::Transform::translate(slot_center(i, f.w, f.h, self.slot_scroll))
                        }
                    };
                } else {
                    node.shape = None;
                }
            }
        }
    }

    /// The active tileset's grid as its matrix, and the brush
    /// and block riding it (tile desk only).
    pub(crate) fn draw_tileset_matrix(&mut self, ctx: &mut frost::Context, f: &FrameState) {
        // --- The tileset panel (tile desk only) -------------------------
        // The active tileset's grid, drawn as its matrix: the brush
        // rack, and a panel — drag it anywhere on the desk, and the
        // wheel over it scales its cells alone. The picked cell wears
        // the selection frame, and the cursor answers back — the cell
        // under it shows a translucent ghost of the picked tile, the
        // same one-tile `TileMap` shapes the rack draws, scaled into
        // the map and placed where a click would land.
        {
            let rects = &f.brects;
            let cells = f.bcells;
            let cell = f.bcell;
            if f.tile_view && !self.cell_shapes.is_empty() {
                // The plate: one quiet board behind the whole rack,
                // hugging the matrix as drawn.
                if let Some([x0, y0, x1, y1]) = f.bplate {
                    ctx.rectangle(
                        (x0 + x1) / 2.0,
                        (y0 + y1) / 2.0,
                        (x1 - x0) / 2.0,
                        (y1 - y0) / 2.0,
                        PLATE,
                        PLATE_Z,
                    );
                }
                // The pool keeps its full size; the rack's cell
                // decides what shows. Shapes are built for the
                // standard box, so a shrunk rack scales them.
                for node in &mut ctx.scene().root.children[BAND0..BAND0 + BAND_CELLS] {
                    node.shape = None;
                }
                // The shapes are built for the standard box; the
                // panel's zoom scales them with it, up or down.
                let k2 = cell / BAND_CELL;
                for (i, rect) in rects.iter().take(cells).enumerate() {
                    let node = &mut ctx.scene().root.children[BAND0 + i];
                    node.shape = self.cell_shapes.get(i).cloned();
                    node.scale = [k2, k2];
                    node.transform = frost::Transform::translate([
                        (rect[0] + rect[2]) / 2.0,
                        (rect[1] + rect[3]) / 2.0,
                    ]);
                    node.order = BAND_Z;
                }
                // The held chip: the picked cell's art wearing the
                // brush's transform — the right-click's verdict,
                // visible without leaving the panel.
                let chip_data = match (
                    self.band_base.clone(),
                    self.picked_cell.filter(|c| *c < cells),
                    f.bplate,
                ) {
                    (Some(base), Some(c), Some([_, _, x1, y1])) if self.clip.is_none() => {
                        let [tw, th] = self.active().map_or([1.0, 1.0], |sp| {
                            [
                                sp.current.width() as f32 / f.bcols.max(1) as f32,
                                sp.current.height() as f32 / f.brows.max(1) as f32,
                            ]
                        });
                        let k = 0.8 * k2 * ((BAND_CELL / tw).min(BAND_CELL / th));
                        let (dw, dh) = (tw * k, th * k);
                        let b = self.brush;
                        let mut shape = base;
                        shape.set_tiles(vec![
                            frost::Tile::new([0.0, 0.0], [dw, dh], cell_uv(c, f.brows, f.bcols))
                                .transformed((b >> 2) & 3, b & 1 != 0, b & 2 != 0),
                        ]);
                        Some((shape, [x1 + 8.0 + dw / 2.0, y1 - 8.0 - dh / 2.0], dw, dh))
                    }
                    _ => None,
                };
                // Beside the plate's upper-right, on its own quiet
                // board — never over a cell's art.
                if let Some((shape, at, dw, dh)) = chip_data {
                    ctx.rectangle(at[0], at[1], dw / 2.0 + 5.0, dh / 2.0 + 5.0, PLATE, PLATE_Z);
                    let chip = &mut ctx.scene().root.children[HELD];
                    chip.shape = Some(shape);
                    chip.transform = frost::Transform::translate(at);
                    chip.order = BAND_Z + 0.2;
                } else {
                    ctx.scene().root.children[HELD].shape = None;
                }
                if let Some(picked) = self.picked_cell.filter(|c| *c < cells) {
                    let [x0, y0, x1, y1] = rects[picked];
                    ctx.line(x0, y0, x1, y0, SELECT, BBOX_WIDTH, BAND_Z + 0.1);
                    ctx.line(x1, y0, x1, y1, SELECT, BBOX_WIDTH, BAND_Z + 0.1);
                    ctx.line(x1, y1, x0, y1, SELECT, BBOX_WIDTH, BAND_Z + 0.1);
                    ctx.line(x0, y1, x0, y0, SELECT, BBOX_WIDTH, BAND_Z + 0.1);
                }
                // The ghost: the picked tile where the next click
                // would place it. Off the canvas, in a panel or over
                // the rack itself, it withdraws.
                let over_band = rects
                    .iter()
                    .take(cells)
                    .any(|[x0, y0, x1, y1]| p_in(f.pos, [*x0, *y0, *x1, *y1]));
                // A held block ghosts as a block: the whole picture
                // gathers on the cursor's cell, dressed as copied.
                // Only the active tileset ghosts — one shape, one
                // texture; a block of another tileset still pastes.
                let block_ghost =
                    match (&self.clip, &self.band_base, f.pos) {
                        (Some(clip), Some(base), Some(p))
                            if f.tile_view
                                && !over_band
                                && !self.ui.hovering()
                                && in_work_area(p, f.w, f.h)
                                && self.active().is_some_and(|sp| sp.path == clip.tileset) =>
                        {
                            let (rows, cols) =
                                self.active().and_then(|sp| sp.atlas).unwrap_or((1, 1));
                            let (tw, th) = (
                                self.active().map_or(1.0, |sp| sp.current.width() as f32)
                                    / cols.max(1) as f32,
                                self.active().map_or(1.0, |sp| sp.current.height() as f32)
                                    / rows.max(1) as f32,
                            );
                            let (col, row) = grid_cell(
                                (p[0] - f.view[0]) / self.zoom,
                                (p[1] - f.view[1]) / self.zoom,
                                tw,
                                th,
                            );
                            let (bc, br) = paint_bounds(&self.maps, &clip.tileset);
                            {
                                // Preview the drop the desk will
                                // perform: exactly the cells the drop
                                // writes — the paste's own list,
                                // intersected with the map. Where the
                                // block hangs past an edge, the
                                // preview shows the part that lands,
                                // no more and no less.
                                let (oc, orr) = (col, row);
                                let mut shape = base.clone();
                                let tiles: Vec<frost::Tile> =
                                    ghost_cells(clip, col, row, bc, br)
                                        .into_iter()
                                        .map(|(c, r, cell, tf)| {
                                            frost::Tile::new(
                                                [(c - oc) as f32 * tw, -((r - orr) as f32) * th],
                                                [tw, th],
                                                cell_uv(cell as usize, rows, cols),
                                            )
                                            .transformed((tf >> 2) & 3, tf & 1 != 0, tf & 2 != 0)
                                        })
                                        .collect();
                                shape.set_tiles(tiles);
                                if let frost::Shape::TileMap { color, .. } = &mut shape {
                                    color.a = GHOST_A;
                                }
                                let center = [
                                    (oc as f32 + 0.5) * tw * self.zoom + f.view[0],
                                    -(orr as f32 + 0.5) * th * self.zoom + f.view[1],
                                ];
                                let node = &mut ctx.scene().root.children[GHOST];
                                node.transform = ghost_transform(self.zoom, center);
                                node.order = GHOST_Z;
                                Some(shape)
                            }
                        }
                        _ => None,
                    };
                let ghost = block_ghost.or_else(|| {
                    match (
                        &self.band_base,
                        self.picked_cell.filter(|c| *c < self.cell_shapes.len()),
                        f.pos,
                        f.tile_view
                            && in_work_area(f.pos.unwrap_or([0.0, -1e9]), f.w, f.h)
                            && !self.ui.hovering()
                            && !over_band,
                    ) {
                        (Some(base), Some(cell), Some(p), true) => {
                            let (rows, cols) =
                                self.active().and_then(|sp| sp.atlas).unwrap_or((1, 1));
                            let (tw, th) = (
                                self.active().map_or(1.0, |sp| sp.current.width() as f32)
                                    / cols.max(1) as f32,
                                self.active().map_or(1.0, |sp| sp.current.height() as f32)
                                    / rows.max(1) as f32,
                            );
                            // The cell the cursor stands on, in map space:
                            // columns to the right, rows downward (the
                            // grid's y runs down, the window's runs up).
                            let mx = (p[0] - f.view[0]) / self.zoom;
                            let my = (p[1] - f.view[1]) / self.zoom;
                            let (col, row) = grid_cell(mx, my, tw, th);
                            // The brush only reaches the map — the layer
                            // this tileset owns, or the map a first click
                            // would create. Past its edge, no ghost.
                            let (bc, br) = self
                                .active()
                                .map_or((0, 0), |sp| paint_bounds(&self.maps, &sp.path));
                            if cell_in(col, row, bc, br) {
                                let center = [
                                    (col as f32 + 0.5) * tw * self.zoom + f.view[0],
                                    -(row as f32 + 0.5) * th * self.zoom + f.view[1],
                                ];
                                let mut shape = base.clone();
                                let b = self.brush;
                                shape.set_tiles(vec![
                                    frost::Tile::new(
                                        [0.0, 0.0],
                                        [tw, th],
                                        cell_uv(cell, rows, cols),
                                    )
                                    .transformed(
                                        (b >> 2) & 3,
                                        b & 1 != 0,
                                        b & 2 != 0,
                                    ),
                                ]);
                                if let frost::Shape::TileMap { color, .. } = &mut shape {
                                    color.a = GHOST_A;
                                }
                                let node = &mut ctx.scene().root.children[GHOST];
                                node.transform = ghost_transform(self.zoom, center);
                                node.order = GHOST_Z;
                                Some(shape)
                            } else {
                                None
                            }
                        }
                        _ => None,
                    }
                });
                ctx.scene().root.children[GHOST].shape = ghost;
            } else {
                for node in &mut ctx.scene().root.children[BAND0..=HELD] {
                    node.shape = None;
                }
            }
        }
    }

    /// The open sidecar files' trees.
    pub(crate) fn draw_trees(&mut self, ctx: &mut frost::Context) {
        // --- The sidecar views ------------------------------------------
        // Every open file's tree is one widget frame's paint: rows,
        // bands, list buttons, scroll thumbs, close boxes and corner
        // grips, in the shared UI font, at RON_ORDER over the UI's
        // plates. A folded view's tree paints nothing but its close
        // box.
        {
            let mut bufs: Vec<Vec<frost::TreeLine>> =
                self.ron_views.iter().map(|_| Vec::new()).collect();
            let specs: Vec<frost::TreeSpec> = self
                .ron_views
                .iter()
                .zip(bufs.iter_mut())
                .filter_map(|(v, buf)| ron_spec(&self.sprites, self.active, v, buf))
                .collect();
            for spec in &specs {
                self.ui.tree_paint(ctx, spec);
            }
        }
    }

    /// The position markers the active sidecar names.
    pub(crate) fn draw_spot_markers(
        &mut self,
        ctx: &mut frost::Context,
        f: &FrameState,
        spots: &[Spot],
        editing: Option<usize>,
    ) {
        let [_tw, _th] = f.size;

        // --- The position markers ---------------------------------------
        // Every spot the ACTIVE sprite's sidecar names, drawn at its
        // pixel coordinate in the row's palette colour; the picked one
        // swells. Chrome: fixed size in window pixels, so the markers
        // stay readable at any zoom.
        let mut marks = 0usize;
        let mut numbered = 0usize;
        if !f.tile_view
            && self
                .sprites
                .get(self.active)
                .is_some_and(|sp| sp.ron.is_some())
        {
            let [tw, th] = f.size;
            for (pi, s) in spots.iter().enumerate() {
                let [wx, wy] = [
                    (s.x - tw / 2.0) * self.zoom + f.view[0],
                    (th / 2.0 - s.y) * self.zoom + f.view[1],
                ];
                let (r0, r1, a) = if editing == Some(pi) {
                    (11.0, 4.2, 1.0)
                } else {
                    (6.5, 2.4, 0.85)
                };
                let (pr, pg, pb) = PALETTE[pi % PALETTE.len()];
                let nodes = &mut ctx.scene().root.children;
                nodes[SPOTS + marks].shape = Some(frost::Shape::Circle {
                    center: [0.0, 0.0],
                    radius: r0,
                    color: frost::Color {
                        r: pr,
                        g: pg,
                        b: pb,
                        a,
                    },
                });
                nodes[SPOTS + marks].transform = frost::Transform::translate([wx, wy]);
                marks += 1;
                nodes[SPOTS + marks].shape = Some(frost::Shape::Circle {
                    center: [0.0, 0.0],
                    radius: r1,
                    color: frost::Color {
                        r: 0.95,
                        g: 0.95,
                        b: 0.95,
                        a,
                    },
                });
                nodes[SPOTS + marks].transform = frost::Transform::translate([wx, wy]);
                marks += 1;
                // An entry of a list wears its seat number beside the
                // marker — the same index its `[n]:` row shows, and it
                // follows the entry through every reorder.
                if let Some(i) = s.idx {
                    nodes[SPOTT + numbered].shape = Some(self.ron_text(
                        format!("{i}"),
                        SEAT_SIZE,
                        // 700 is this FiraCode's weight axis ceiling.
                        700.0,
                        frost::Color {
                            r: 0.93,
                            g: 0.94,
                            b: 0.97,
                            a: 0.95,
                        },
                    ));
                    nodes[SPOTT + numbered].transform =
                        frost::Transform::translate([wx + 13.0, wy + 13.0 - RON_LIFT * SEAT_SIZE]);
                    numbered += 1;
                }
            }
        }
        let nodes = &mut ctx.scene().root.children;
        for node in &mut nodes[SPOTS + marks..SPOTS + 2 * SPOTS_MAX] {
            node.shape = None;
        }
        for node in &mut nodes[SPOTT + numbered..SPOTT + SPOTS_MAX] {
            node.shape = None;
        }
    }

    /// The exit question, drawn over everything.
    pub(crate) fn draw_exit_box(&mut self, ctx: &mut frost::Context, f: &FrameState) {
        // The exit box, drawn last of all: it stands over the whole
        // frame — the veil dims the world, the question waits at the
        // centre, two answers beneath.
        if self.quit_prompt {
            match self.ui.confirm(
                ctx,
                [f.w, f.h],
                "Leave the editor?",
                "Yes, quit",
                "No, stay",
            ) {
                Some(frost::Answer::Yes) => {
                    self.quit_prompt = false;
                    ctx.exit();
                }
                Some(frost::Answer::No) => {
                    self.quit_prompt = false;
                    self.status = String::from("staying");
                }
                None => {}
            }
        }
    }
}
