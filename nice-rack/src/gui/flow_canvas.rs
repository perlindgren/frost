//! Geometry and cables derived from the structural routing tree. No connection
//! is inferred from columns or a card happening to share a row with a junction.
use super::*;
use crate::layout::Flow;
#[derive(Clone, Copy)]
struct Port {
    at: [f32; 2],
    lanes: usize,
    meter: Option<(NodeId, bool)>,
}
struct Cable {
    from: Port,
    to: Port,
    via: f32,
}
pub(super) struct PaintInput {
    pub mouse: Option<[f32; 2]>,
    pub broke: bool,
    pub clicked: bool,
    pub on_mark: bool,
    pub gate: f32,
}
pub(super) struct Canvas {
    pub frame: Geometry,
    pub forks: Vec<ForkMark>,
    cables: Vec<Cable>,
    next_scope: usize,
    scope_ids: Vec<Vec<NodeId>>,
}
impl Canvas {
    pub fn new(
        root: &Flow,
        size: [f32; 2],
        offset: [f32; 2],
        shapes: &std::collections::BTreeMap<u64, (usize, usize)>,
        channels: [usize; 2],
    ) -> Self {
        let [w, h] = size;
        let [scroll, vertical] = offset;
        let [input, output] = channels;
        let top = h / 2.0 - 90.0;
        let bottom = -h / 2.0 + 150.0;
        let available = (top - bottom).max(112.0);
        let flow = (top + bottom) / 2.0;
        let rows = root.height().max(1);
        let step = if rows < 2 {
            140.0
        } else {
            ((available - 112.0) / (rows - 1) as f32).clamp(116.0, 140.0)
        };
        let max_vertical = ((rows - 1) as f32 * step + 112.0 - available).max(0.0);
        let centre = if max_vertical > 0.0 {
            top - 56.0 - (rows - 1) as f32 * step / 2.0 + vertical.clamp(0.0, max_vertical)
        } else {
            flow
        };
        let input_x = -w / 2.0 + 273.0 - scroll;
        let mut canvas = Self {
            frame: Geometry {
                slots: Vec::new(),
                input: Slot {
                    id: INPUT_JACK,
                    x: input_x,
                    y: centre,
                },
                output: Slot {
                    id: NodeId(MASTER_WIRE_KEY),
                    x: input_x + (root.width().max(1) + 1) as f32 * COLUMN_STEP,
                    y: centre,
                },
                top,
                bottom,
                flow: centre,
                max_vertical,
            },
            forks: Vec::new(),
            cables: Vec::new(),
            next_scope: 0,
            scope_ids: Vec::new(),
        };
        let incoming = Port {
            at: [input_x + CARD_WIDTH / 2.0, centre],
            lanes: input,
            meter: Some((INPUT_JACK, true)),
        };
        let outgoing = Port {
            at: [canvas.frame.output.x - CARD_WIDTH / 2.0, centre],
            lanes: output,
            meter: Some((NodeId(MASTER_WIRE_KEY), false)),
        };
        if matches!(root, Flow::Empty) {
            canvas.connect(incoming, outgoing);
        } else {
            let (a, b) = canvas.build(root, input_x + COLUMN_STEP, centre, step, shapes, input);
            canvas.connect(incoming, a);
            canvas.connect(b, outgoing);
        }
        canvas
    }
    fn connect(&mut self, from: Port, to: Port) {
        self.cables.push(Cable {
            from,
            to,
            via: if from.meter.is_none() {
                from.at[0]
            } else if to.meter.is_none() {
                to.at[0]
            } else {
                (from.at[0] + to.at[0]) / 2.0
            },
        });
    }
    fn build(
        &mut self,
        root: &Flow,
        x: f32,
        y: f32,
        step: f32,
        shapes: &std::collections::BTreeMap<u64, (usize, usize)>,
        input: usize,
    ) -> (Port, Port) {
        match root {
            Flow::Plugin(id) => {
                let (inputs, outputs) = shapes[id];
                self.frame.slots.push(Slot {
                    id: NodeId(*id),
                    x,
                    y,
                });
                (
                    Port {
                        at: [x - CARD_WIDTH / 2.0, y],
                        lanes: inputs,
                        meter: Some((NodeId(*id), false)),
                    },
                    Port {
                        at: [x + CARD_WIDTH / 2.0, y],
                        lanes: outputs,
                        meter: Some((NodeId(*id), true)),
                    },
                )
            }
            Flow::Series(children) => {
                let mut column = x;
                let mut first = None;
                let mut previous = None;
                for child in children {
                    let (a, b) = self.build(
                        child,
                        column,
                        y,
                        step,
                        shapes,
                        previous.map_or(input, |p: Port| p.lanes),
                    );
                    if let Some(prev) = previous {
                        self.connect(prev, a);
                    } else {
                        first = Some(a);
                    }
                    previous = Some(b);
                    column += child.width() as f32 * COLUMN_STEP;
                }
                (first.unwrap(), previous.unwrap())
            }
            Flow::Parallel(children) => {
                let scope = self.next_scope;
                self.next_scope += 1;
                self.scope_ids
                    .push(root.ids().into_iter().map(NodeId).collect());
                let split = Port {
                    at: [junction_x(x, false), y],
                    lanes: input,
                    meter: None,
                };
                let mut exits = Vec::new();
                let mut row = y + (root.height() - 1) as f32 * step / 2.0;
                let anchor = NodeId(root.ids()[0]);
                self.forks.push(ForkMark {
                    scope: Some(scope),
                    anchor,
                    x: split.at[0],
                    y,
                    side: Side::Split,
                });
                for child in children {
                    let height = child.height();
                    let cy = row - (height - 1) as f32 * step / 2.0;
                    let (a, b) = self.build(child, x, cy, step, shapes, input);
                    self.connect(split, a);
                    exits.push(b);
                    row -= height as f32 * step;
                }
                let join = Port {
                    at: [
                        junction_x(x + (root.width() - 1) as f32 * COLUMN_STEP, true),
                        y,
                    ],
                    lanes: exits.iter().map(|p| p.lanes).max().unwrap_or(input),
                    meter: None,
                };
                for exit in exits {
                    self.connect(exit, join);
                }
                self.forks.push(ForkMark {
                    scope: Some(scope),
                    anchor,
                    x: join.at[0],
                    y,
                    side: Side::Join,
                });
                (split, join)
            }
            Flow::Empty => unreachable!("normalized expression"),
        }
    }
    fn segments(&self, cable: &Cable) -> Vec<([f32; 2], [f32; 2])> {
        if (cable.from.at[1] - cable.to.at[1]).abs() < 0.5 {
            return vec![(cable.from.at, cable.to.at)];
        }
        vec![
            (cable.from.at, [cable.via, cable.from.at[1]]),
            ([cable.via, cable.from.at[1]], [cable.via, cable.to.at[1]]),
            ([cable.via, cable.to.at[1]], cable.to.at),
        ]
    }
    fn owner(cable: &Cable) -> Option<(NodeId, bool)> {
        if cable.from.meter == Some((INPUT_JACK, true)) {
            cable.from.meter
        } else {
            cable.to.meter.or(cable.from.meter)
        }
    }
    pub fn run_targets(&self, moving: Option<NodeId>) -> Vec<WireRun> {
        self.cables
            .iter()
            .filter_map(|cable| {
                let (Some((from, true)), Some((to, false))) = (cable.from.meter, cable.to.meter)
                else {
                    return None;
                };
                if from == INPUT_JACK
                    || to.0 == MASTER_WIRE_KEY
                    || moving == Some(from)
                    || moving == Some(to)
                {
                    return None;
                }
                Some(WireRun {
                    card: from,
                    x0: cable.from.at[0],
                    x1: cable.to.at[0],
                    y: cable.from.at[1],
                })
            })
            .collect()
    }
    pub fn fork_targets(&self, moving: Option<NodeId>) -> Vec<ForkMark> {
        self.forks
            .iter()
            .copied()
            .filter_map(|mut mark| {
                if Some(mark.anchor) == moving {
                    // A ring remains usable while its first plugin is being dragged.
                    // Its group's IDs were captured when geometry was built.
                    mark.anchor = *self
                        .scope_ids
                        .get(mark.scope?)?
                        .iter()
                        .find(|id| Some(**id) != moving)?;
                }
                Some(mark)
            })
            .collect()
    }
    pub fn gaps(&self, open: impl Fn(u64, bool) -> bool) -> Vec<WireGap> {
        let mut gaps = Vec::new();
        for slot in self
            .frame
            .slots
            .iter()
            .chain(std::iter::once(&self.frame.input))
            .chain(std::iter::once(&self.frame.output))
        {
            for side in [false, true] {
                if (slot.id == INPUT_JACK && !side)
                    || (slot.id.0 == MASTER_WIRE_KEY && side)
                    || !open(slot.id.0, side)
                {
                    continue;
                }
                let source = if !side {
                    self.cables
                        .iter()
                        .find(|c| c.to.meter == Some((slot.id, false)))
                        .and_then(|c| c.from.meter)
                } else {
                    None
                };
                let anchor = source.and_then(|(id, _)| {
                    self.frame
                        .slots
                        .iter()
                        .chain(std::iter::once(&self.frame.input))
                        .find(|s| s.id == id)
                });
                gaps.push(if let Some(source) = anchor {
                    meter_gap(wire_box_x(source.x, true), slot.y, true)
                } else {
                    meter_gap(wire_box_x(slot.x, side), slot.y, side)
                });
            }
        }
        gaps
    }
    pub fn marks(&self, found: &[Junction]) -> Vec<WidthMark> {
        found
            .iter()
            .filter_map(|mark| {
                let cable = mark
                    .to
                    .and_then(|id| self.cables.iter().find(|c| c.to.meter == Some((id, false))))
                    .or_else(|| {
                        mark.from.and_then(|id| {
                            self.cables
                                .iter()
                                .find(|c| c.from.meter == Some((id, true)))
                        })
                    })
                    .or_else(|| {
                        self.cables
                            .iter()
                            .find(|c| c.to.meter == Some((NodeId(MASTER_WIRE_KEY), false)))
                    })?;
                let mut candidates = self.segments(cable);
                // Prefer the run nearest the receiving plugin (or output jack).
                if mark.to.is_some() {
                    candidates.reverse();
                }
                candidates.into_iter().find_map(|(mut a, mut b)| {
                    let vertical = (a[0] - b[0]).abs() < 0.5;
                    if !vertical {
                        if let Some((id, true)) = cable.from.meter {
                            let x = if id == INPUT_JACK {
                                self.frame.input.x
                            } else {
                                self.frame.slots.iter().find(|s| s.id == id)?.x
                            };
                            a[0] = a[0].max(wire_box_x(x, true) + STUB_HALF);
                        } else {
                            a[0] += MIXER_RING;
                        }
                        if let Some((id, false)) = cable.to.meter {
                            let x = if id.0 == MASTER_WIRE_KEY {
                                self.frame.output.x
                            } else {
                                self.frame.slots.iter().find(|s| s.id == id)?.x
                            };
                            b[0] = b[0].min(wire_box_x(x, false) - STUB_HALF);
                        } else {
                            b[0] -= MIXER_RING;
                        }
                    }
                    let (start, end) = if vertical { (a[1], b[1]) } else { (a[0], b[0]) };
                    let lo = start.min(end) + MARK_HIT;
                    let hi = start.max(end) - MARK_HIT;
                    (hi >= lo).then_some(WidthMark {
                        id: mark.id,
                        inputs: mark.inputs.count(),
                        outputs: mark.outputs.count(),
                        cross: if vertical { a[0] } else { a[1] },
                        lo,
                        hi,
                        vertical,
                        forward: if end >= start { 1.0 } else { -1.0 },
                    })
                })
            })
            .collect()
    }
    pub fn paint(
        &self,
        ui: &mut RackUi,
        ctx: &mut Context,
        input: PaintInput,
        marks: &[WidthMark],
    ) {
        let PaintInput {
            mouse,
            broke,
            clicked,
            on_mark,
            gate,
        } = input;
        let color = rgb(107, 114, 128);
        if broke && !on_mark {
            let owner = self.cables.iter().find_map(|cable| {
                self.segments(cable)
                    .iter()
                    .any(|(a, b)| {
                        (a[1] - b[1]).abs() < 0.5
                            && b[0] > a[0]
                            && hit(
                                mouse,
                                (a[0] + b[0]) / 2.0,
                                a[1],
                                (b[0] - a[0]) / 2.0,
                                BREAK_HALF,
                            )
                    })
                    .then(|| Self::owner(cable))
                    .flatten()
            });
            if let Some((id, side)) = owner {
                ui.toggle_wire(id.0, side);
            }
        }
        let mut gaps = self.gaps(|id, side| ui.wire_open(id, side));
        for slot in &self.frame.slots {
            if ui.runtime.bypassed(slot.id.0) {
                gaps.push(WireGap {
                    start: wire_box_x(slot.x, false),
                    end: slot.x - CARD_WIDTH / 2.0,
                    y: slot.y,
                });
                gaps.push(WireGap {
                    start: slot.x + CARD_WIDTH / 2.0,
                    end: wire_box_x(slot.x, true),
                    y: slot.y,
                });
                for segment in bypass_path(slot.x, slot.y, 10.0).windows(2) {
                    ctx.line(
                        segment[0][0],
                        segment[0][1],
                        segment[1][0],
                        segment[1][1],
                        nice_common::knob::AMBER,
                        2.0,
                        0.18,
                    );
                }
            }
        }
        for cable in &self.cables {
            for (a, b) in self.segments(cable) {
                if (a[1] - b[1]).abs() < 0.5 {
                    for (from, to, lanes) in
                        width_runs(a[0], b[0], cable.from.lanes, a[1], marks, &ui.mark_offsets)
                    {
                        wire_segment(ctx, from, to, a[1], color, lanes, &gaps);
                    }
                } else {
                    wire_riser(ctx, a[0], a[1], b[1], color, cable.from.lanes);
                }
            }
        }
        for ring in &self.forks {
            ctx.circle(ring.x, ring.y, MIXER_RING, color, 0.15);
            ctx.circle(ring.x, ring.y, MIXER_RING - 2.2, rgb(23, 18, 38), 0.16);
            ctx.line(ring.x, ring.y - 4.0, ring.x, ring.y + 4.0, color, 2.0, 0.22);
            if ring.side == Side::Join {
                ctx.line(ring.x - 4.0, ring.y, ring.x + 4.0, ring.y, color, 2.0, 0.22);
            }
        }
        for slot in &self.frame.slots {
            if ui.runtime.bypassed(slot.id.0) {
                continue;
            }
            for side in [false, true] {
                let x = wire_box_x(slot.x, side);
                let key = (slot.id.0, side);
                if clicked && ui.choosing.is_none() && hit(mouse, x, slot.y, STUB_HALF, STUB_TALL) {
                    let meter = ui
                        .wire_meters
                        .entry(key)
                        .or_insert_with(|| new_wire_meter(0.0, now()));
                    meter.open = !meter.open;
                    ui.acknowledge_clip(MeterPos::Wire(slot.id.0, side));
                }
                ctx.rectangle(
                    x,
                    slot.y,
                    STUB_HALF,
                    STUB_TALL,
                    stub_color(ui.wire_meters.get(&key), gate, color),
                    0.25,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn n(id: u64) -> Flow {
        Flow::Plugin(id)
    }
    fn s(c: Vec<Flow>) -> Flow {
        Flow::series(c)
    }
    fn p(c: Vec<Flow>) -> Flow {
        Flow::parallel(c)
    }
    fn view(root: &Flow) -> Canvas {
        let shapes = root.ids().into_iter().map(|id| (id, (1, 1))).collect();
        Canvas::new(root, [1800.0, 1000.0], [0.0, 0.0], &shapes, [1, 2])
    }
    #[test]
    fn a_nested_fork_has_distinct_ports_and_no_overlapping_cards() {
        let root = p(vec![
            s(vec![n(1), p(vec![n(2), n(3)]), n(4)]),
            s(vec![n(5), n(6), n(7)]),
        ]);
        let canvas = view(&root);
        assert_eq!(canvas.frame.slots.len(), 7);
        for (i, a) in canvas.frame.slots.iter().enumerate() {
            for b in &canvas.frame.slots[i + 1..] {
                assert!((a.x - b.x).abs() >= COLUMN_STEP || (a.y - b.y).abs() >= 116.0);
            }
        }
        assert_eq!(canvas.forks.len(), 4);
        let slot = |id| {
            canvas
                .frame
                .slots
                .iter()
                .find(|s| s.id == NodeId(id))
                .unwrap()
        };
        assert!(slot(2).y > slot(3).y && slot(3).y > slot(5).y);
        assert!(slot(1).x < slot(2).x && slot(2).x < slot(4).x);
        assert!(
            canvas
                .cables
                .iter()
                .any(|c| c.from.meter == Some((NodeId(5), true))
                    && c.to.meter == Some((NodeId(6), false)))
        );
        assert!(
            !canvas
                .cables
                .iter()
                .any(|c| c.from.meter == Some((NodeId(1), true))
                    && c.to.meter == Some((NodeId(6), false)))
        );
    }
    #[test]
    fn card_drag_zones_dispatch_to_the_structural_operation() {
        for (point, placement) in [
            ([-89.0, 0.0], crate::layout::Placement::Before),
            ([89.0, 0.0], crate::layout::Placement::After),
            ([0.0, 40.0], crate::layout::Placement::Above),
            ([0.0, -40.0], crate::layout::Placement::Below),
        ] {
            let root = p(vec![s(vec![n(1), n(2)]), n(3)]);
            let canvas = view(&root);
            let target = canvas
                .frame
                .slots
                .iter()
                .find(|s| s.id == NodeId(2))
                .unwrap();
            let at = Some([target.x + point[0], target.y + point[1]]);
            for moving in [None, Some(NodeId(3))] {
                let selected = target_at(
                    at,
                    &canvas.frame.slots,
                    moving,
                    &DropZones::default(),
                    canvas.frame.top,
                    canvas.frame.bottom,
                    1800.0,
                )
                .unwrap();
                assert_eq!(selected.card, NodeId(2));
                assert_eq!(crate::layout::Placement::from(selected.side), placement);
                let edited = root
                    .edit_scoped(
                        crate::layout::Edit::Move {
                            id: 3,
                            target: selected.card.0,
                            placement: selected.side.into(),
                        },
                        selected.scope,
                    )
                    .unwrap();
                assert_eq!(
                    edited,
                    root.edit(crate::layout::Edit::Move {
                        id: 3,
                        target: 2,
                        placement
                    })
                    .unwrap()
                );
            }
        }
    }
    #[test]
    fn ring_drag_targets_the_exact_group_not_the_innermost_group_of_its_anchor() {
        let root = p(vec![s(vec![p(vec![n(1), n(2)]), n(3)]), n(4)]);
        let canvas = view(&root);
        let forks = canvas.fork_targets(Some(NodeId(4)));
        for mark in &forks {
            let target = target_at(
                Some([mark.x, mark.y]),
                &canvas.frame.slots,
                Some(NodeId(4)),
                &DropZones {
                    joins: &forks,
                    runs: &[],
                },
                canvas.frame.top,
                canvas.frame.bottom,
                1800.0,
            )
            .unwrap();
            assert_eq!(target.scope, mark.scope);
            let edited = root
                .edit_scoped(
                    crate::layout::Edit::Move {
                        id: 4,
                        target: target.card.0,
                        placement: target.side.into(),
                    },
                    target.scope,
                )
                .unwrap();
            if mark.scope == Some(0) {
                assert_eq!(
                    edited,
                    if mark.side == Side::Split {
                        s(vec![n(4), p(vec![n(1), n(2)]), n(3)])
                    } else {
                        s(vec![p(vec![n(1), n(2)]), n(3), n(4)])
                    }
                );
            } else {
                assert_eq!(
                    edited,
                    if mark.side == Side::Split {
                        s(vec![n(4), p(vec![n(1), n(2)]), n(3)])
                    } else {
                        s(vec![p(vec![n(1), n(2)]), n(4), n(3)])
                    }
                );
            }
        }
    }
    #[test]
    fn self_drops_empty_space_and_escape_do_not_retarget_a_neighbour() {
        let canvas = view(&s(vec![n(1), n(2)]));
        let slot = canvas.frame.slots[0];
        assert!(
            target_at(
                Some([slot.x, slot.y]),
                &canvas.frame.slots,
                Some(slot.id),
                &DropZones::default(),
                canvas.frame.top,
                canvas.frame.bottom,
                1800.0
            )
            .is_none()
        );
        assert!(
            target_at(
                Some([10000.0, slot.y]),
                &canvas.frame.slots,
                None,
                &DropZones::default(),
                canvas.frame.top,
                canvas.frame.bottom,
                1800.0
            )
            .is_none()
        );
        assert!(
            target_at(
                None,
                &canvas.frame.slots,
                Some(slot.id),
                &DropZones::default(),
                canvas.frame.top,
                canvas.frame.bottom,
                1800.0
            )
            .is_none()
        );
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;
    fn n(id: u64) -> Flow {
        Flow::Plugin(id)
    }
    fn s(c: Vec<Flow>) -> Flow {
        Flow::series(c)
    }
    fn p(c: Vec<Flow>) -> Flow {
        Flow::parallel(c)
    }
    #[test]
    fn replay_the_whole_session_through_actual_drag_hit_testing() {
        let mut root = s(vec![n(1), p(vec![s(vec![n(2), n(4)]), n(3)])]);
        let steps = [
            (3, 2, Side::Right),
            (2, 4, Side::Under),
            (1, 2, Side::Left),
            (2, 4, Side::Right),
            (5, 1, Side::Left),
            (5, 2, Side::Under),
            (3, 4, Side::Over),
            (2, 3, Side::Right),
            (5, 2, Side::Right),
        ];
        for (id, target, side) in steps {
            if !root.contains(id) {
                root = root.edit(crate::layout::Edit::Append { id }).unwrap();
            }
            let shapes = root.ids().into_iter().map(|id| (id, (1, 1))).collect();
            let canvas = Canvas::new(&root, [2400.0, 1200.0], [0.0, 0.0], &shapes, [1, 2]);
            let target_slot = canvas
                .frame
                .slots
                .iter()
                .find(|s| s.id == NodeId(target))
                .unwrap();
            let (dx, dy) = match side {
                Side::Left => (-89.0, 0.0),
                Side::Right => (89.0, 0.0),
                Side::Over => (0.0, 40.0),
                Side::Under => (0.0, -40.0),
                _ => unreachable!(),
            };
            let forks = canvas.fork_targets(Some(NodeId(id)));
            let runs = canvas.run_targets(Some(NodeId(id)));
            let target = target_at(
                Some([target_slot.x + dx, target_slot.y + dy]),
                &canvas.frame.slots,
                Some(NodeId(id)),
                &DropZones {
                    joins: &forks,
                    runs: &runs,
                },
                canvas.frame.top,
                canvas.frame.bottom,
                2400.0,
            )
            .unwrap();
            assert_eq!(target.side, side);
            root = root
                .edit_scoped(
                    crate::layout::Edit::Move {
                        id,
                        target: target.card.0,
                        placement: target.side.into(),
                    },
                    target.scope,
                )
                .unwrap();
        }
        assert_eq!(root, p(vec![s(vec![n(3), n(2), n(5)]), n(4), n(1)]));
        let shapes = root.ids().into_iter().map(|id| (id, (1, 1))).collect();
        let canvas = Canvas::new(&root, [2400.0, 1200.0], [0.0, 0.0], &shapes, [1, 2]);
        assert_eq!(
            canvas.forks.len(),
            2,
            "one three-way split and one three-way join"
        );
    }
}
