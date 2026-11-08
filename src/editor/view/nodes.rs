//! The Nodes page of the Gimmicks panel and the canvas that takes the timeline's place while it is
//! open: boxes for events, logic, values and actions, wires between their ports, dragged the way
//! a node editor is. Everything here edits `Graph` through the editor, one undo step each.

use super::chrome::wrap;
use super::look::{self, Look};
use super::{Btn, Drag, Mode, View, inside};
use crate::render::{Canvas, rgba};
use crate::tko::gimmick::{FieldProp, NoteProp, PropGroup};
use crate::tko::graph::{Category, Kind, Node, Port, Target};
use crate::tko::recipes::Recipe;

/// A dropdown over the node canvas.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Pick {
    Target,
    Recipe,
}

/// What the Nodes page can be told to do.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum NAct {
    Cat(usize),
    Add(Kind),
    Param(usize, i32),
    /// Type a number into parameter `i`.
    Edit(usize),
    /// Make a group of the notes selected on the timeline and point the node at it.
    FromSelection,
    Open(Pick),
    SetTarget(Target),
    PickGroup(i32),
    AddRecipe(Recipe),
    NoTarget,
    Ease,
    Clip,
    Delete,
    Seed(i32),
    /// Back to 100% at the origin.
    Home,
    /// Frame every node.
    Fit,
}

const NODE_W: f32 = 104.0;
const HEAD: f32 = 13.0;
const ROW: f32 = 11.0;
const PORT_R: f32 = 3.0;
const GRAB: f32 = 6.0;
/// How long after it fires a node stays lit (ms).
const LIT_MS: f64 = 250.0;

pub const MIN_ZOOM: f32 = 0.05;
pub const MAX_ZOOM: f32 = 3.0;
/// Below this the boxes lose their port labels and parameters; below `ZOOM_BLOCKS` they are plain blocks.
const ZOOM_TITLES: f32 = 0.5;
const ZOOM_BLOCKS: f32 = 0.3;

fn colour(c: Category) -> u32 {
    match c {
        Category::Event => 0xC98A2B,
        Category::Logic => 0x7C6BD6,
        Category::Value => 0x2F95A6,
        Category::Math => 0x4F8F5A,
        Category::Action => 0xC84F68,
    }
}

fn wire_colour(p: Port) -> u32 {
    match p {
        Port::Trigger => 0xF0B64A,
        Port::Number => 0x5CD0E0,
    }
}

/// What a wire of this kind carries.
fn port_about(p: Port, input: bool) -> &'static str {
    match (p, input) {
        (Port::Trigger, true) => {
            "Takes moments in time (orange wires): something happens at each one."
        }
        (Port::Trigger, false) => "Gives moments in time (orange wires).",
        (Port::Number, true) => "Takes a number that can be read at any time (cyan wires).",
        (Port::Number, false) => "Gives a number that can be read at any time (cyan wires).",
    }
}

fn num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e9 {
        format!("{}", v as i64)
    } else {
        format!("{v:.2}")
    }
}

/// Rows below the header: the input ports and the parameters share them, then the lines for the
/// target and the easing.
fn rows(n: &Node) -> usize {
    n.kind.inputs().len().max(n.kind.params().len()).max(1) + extra_rows(n)
}

fn extra_rows(n: &Node) -> usize {
    usize::from(n.kind.needs_target()) + usize::from(n.kind.uses_ease())
}

/// What a click on the canvas landed on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Out(u32),
    In(u32, usize),
    Body(u32),
}

impl View {
    /// Whether the node canvas is showing in place of the timeline.
    pub(super) fn node_mode(&self) -> bool {
        self.mode == Mode::Effects && self.g_page == 3 && self.ed.is_tko()
    }

    // ---- canvas geometry ----

    pub(super) fn to_screen(&self, wx: f32, wy: f32) -> (f32, f32) {
        let [x, y, _, _] = self.layout.nodes;
        (
            x + (wx - self.n_pan.0) * self.n_zoom,
            y + (wy - self.n_pan.1) * self.n_zoom,
        )
    }

    pub(super) fn to_world(&self, sx: f32, sy: f32) -> (f32, f32) {
        let [x, y, _, _] = self.layout.nodes;
        (
            (sx - x) / self.n_zoom + self.n_pan.0,
            (sy - y) / self.n_zoom + self.n_pan.1,
        )
    }

    pub(super) fn node_rect(&self, n: &Node) -> [f32; 4] {
        let (sx, sy) = self.to_screen(n.x, n.y);
        [
            sx,
            sy,
            NODE_W * self.n_zoom,
            (HEAD + ROW * rows(n) as f32) * self.n_zoom,
        ]
    }

    pub(super) fn out_pos(&self, n: &Node) -> (f32, f32) {
        let r = self.node_rect(n);
        (r[0] + r[2], r[1] + HEAD * self.n_zoom / 2.0)
    }

    pub(super) fn in_pos(&self, n: &Node, port: usize) -> (f32, f32) {
        let r = self.node_rect(n);
        (
            r[0],
            r[1] + (HEAD + ROW * (port as f32 + 0.5)) * self.n_zoom,
        )
    }

    /// What is under the pointer, the topmost box first.
    pub(super) fn node_hit(&self, x: f32, y: f32) -> Option<Hit> {
        let g = &self.ed.gimmick()?.graph;
        for n in g.nodes.iter().rev() {
            if n.kind.output().is_some() {
                let (px, py) = self.out_pos(n);
                if (px - x).abs() <= GRAB && (py - y).abs() <= GRAB {
                    return Some(Hit::Out(n.id));
                }
            }
            for port in 0..n.kind.inputs().len() {
                let (px, py) = self.in_pos(n, port);
                if (px - x).abs() <= GRAB && (py - y).abs() <= GRAB {
                    return Some(Hit::In(n.id, port));
                }
            }
            if inside(self.node_rect(n), x, y) {
                return Some(Hit::Body(n.id));
            }
        }
        None
    }

    pub(super) fn lit(&self, id: u32) -> bool {
        self.game
            .as_ref()
            .and_then(|g| g.tko_fired_ago(id))
            .is_some_and(|ago| (0.0..LIT_MS).contains(&ago))
    }

    // ---- drawing ----

    /// A line of 1.5 units made of rectangles.
    fn segment(c: &mut Canvas, a: (f32, f32), b: (f32, f32), col: u32, alpha: f32) {
        let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
        let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
        c.rect(
            x0 - 0.75,
            y0 - 0.75,
            (x1 - x0) + 1.5,
            (y1 - y0) + 1.5,
            rgba(col, alpha),
        );
    }

    /// An elbow from an output to an input.
    fn wire(c: &mut Canvas, a: (f32, f32), b: (f32, f32), col: u32, alpha: f32) {
        let run = 12.0;
        let mid = if b.0 > a.0 + 2.0 * run {
            (a.0 + b.0) / 2.0
        } else {
            a.0 + run
        };
        View::segment(c, a, (mid, a.1), col, alpha);
        View::segment(c, (mid, a.1), (mid, b.1), col, alpha);
        if mid > b.0 {
            // Backwards: come round under the target's left edge.
            View::segment(c, (mid, b.1), (b.0 - run, b.1), col, alpha);
        } else {
            View::segment(c, (mid, b.1), b, col, alpha);
        }
    }

    /// What parameter `i` of `n` reads as: a name where the number is a choice (a group, a type,
    /// a mark), the number otherwise. `short` is for the cramped box on the canvas.
    pub(super) fn node_param_text(&self, n: &Node, i: usize, short: bool) -> String {
        let Some(d) = n.kind.params().get(i) else {
            return String::new();
        };
        let v = n.param(i);
        let k = v as usize;
        let marked = matches!(n.kind, Kind::OnMark | Kind::Between | Kind::Span);
        match d.name {
            "group" => {
                let g = self
                    .ed
                    .gimmick()
                    .and_then(|g| g.groups.get(k.wrapping_sub(1)));
                match (k, g) {
                    (0, _) => "any".into(),
                    (_, Some(g)) if !short && !g.name.is_empty() => g.name.clone(),
                    _ => format!("G{k}"),
                }
            }
            "type" => ["any", "don", "kat", "finisher", "roll"]
                .get(k)
                .copied()
                .unwrap_or("any")
                .into(),
            "judge" => ["any", "great", "ok"]
                .get(k)
                .copied()
                .unwrap_or("any")
                .into(),
            "shape" => ["sine", "triangle", "saw", "square"]
                .get(k)
                .copied()
                .unwrap_or("sine")
                .into(),
            "mark" | "from" | "to" if marked => {
                let at = self
                    .ed
                    .gimmick()
                    .and_then(|g| g.marks.get(k.wrapping_sub(1)))
                    .map(|m| m.time);
                match (k, at) {
                    (0, _) if d.name == "mark" => "every".into(),
                    (0, _) if d.name == "from" => "start".into(),
                    (0, _) => "end".into(),
                    (_, Some(t)) if !short => format!("#{k} {}", super::fmt_time(t)),
                    (_, Some(_)) => format!("#{k}"),
                    (_, None) => format!("#{k} (none)"),
                }
            }
            _ => num(v),
        }
    }

    pub(super) fn draw_node_canvas(&mut self, c: &mut Canvas) {
        let [x, y, w, h] = self.layout.nodes;
        c.round_rect(
            [x, y],
            [w, h],
            3.0,
            rgba(0x101016, 0.95),
            rgba(0x2a2a38, 1.0),
            1.0,
        );
        let start = c.shapes.len();
        // A faint grid that moves with the pan, so panning can be seen.
        let mut step = 40.0 * self.n_zoom;
        while step < 12.0 {
            step *= 2.0;
        }
        let (ox, oy) = self.to_screen(0.0, 0.0);
        let mut gx = x + (ox - x).rem_euclid(step);
        while gx < x + w {
            c.rect(gx, y, 1.0, h, rgba(0xffffff, 0.035));
            gx += step;
        }
        let mut gy = y + (oy - y).rem_euclid(step);
        while gy < y + h {
            c.rect(x, gy, w, 1.0, rgba(0xffffff, 0.035));
            gy += step;
        }
        let Some(graph) = self.ed.gimmick().map(|g| g.graph.clone()) else {
            return;
        };
        for n in &graph.nodes {
            for (port, from) in n.inputs.iter().enumerate() {
                let Some(src) = from.and_then(|id| graph.node(id)) else {
                    continue;
                };
                let kind = n.kind.inputs()[port].1;
                let on = self.lit(src.id);
                let (a, b) = (self.out_pos(src), self.in_pos(n, port));
                // A wire wholly off the canvas costs nothing to skip (the elbow stays within a run of its ends).
                if a.0.max(b.0) + 12.0 < x
                    || a.0.min(b.0) - 12.0 > x + w
                    || a.1.max(b.1) < y
                    || a.1.min(b.1) > y + h
                {
                    continue;
                }
                View::wire(c, a, b, wire_colour(kind), if on { 1.0 } else { 0.55 });
            }
        }
        if let Drag::Wire { from } = self.drag
            && let Some(src) = graph.node(from)
            && let Some(p) = src.kind.output()
        {
            View::wire(c, self.out_pos(src), self.mouse, wire_colour(p), 0.9);
        }
        let z = self.n_zoom;
        for n in &graph.nodes {
            let [nx, ny, nw, nh] = self.node_rect(n);
            if nx > x + w || nx + nw < x || ny > y + h || ny + nh < y {
                continue;
            }
            let selected = self.n_sel == Some(n.id);
            let lit = self.lit(n.id);
            let col = colour(n.kind.category());
            c.round_rect(
                [nx, ny],
                [nw, nh],
                3.0,
                rgba(0x1b1b26, 0.97),
                rgba(
                    if selected {
                        0xFFFFFF
                    } else if lit {
                        0xFFE27A
                    } else {
                        col
                    },
                    1.0,
                ),
                if selected || lit { 2.0 } else { 1.0 },
            );
            c.rect(
                nx + 1.0,
                ny + 1.0,
                nw - 2.0,
                HEAD * z - 1.0,
                rgba(col, 0.85),
            );
            self.tips.push((
                [nx, ny, nw, HEAD * z],
                format!("{}\n{}", n.kind.label(), n.kind.about()),
            ));
            // A node that can't do anything yet wears a red "!" with the reason as its tip.
            if let Some(why) = graph.problem(n.id) {
                let s = (HEAD * z - 3.0).max(6.0);
                let (bx, by) = (nx + nw - s - 2.0, ny + 1.5);
                c.rect(bx, by, s, s, rgba(0xE2463C, 1.0));
                c.text_centered(
                    bx + s / 2.0,
                    by + (s - 6.0) / 2.0,
                    6.0,
                    rgba(0xffffff, 1.0),
                    "!",
                );
                self.tips
                    .push(([bx, by, s, s], format!("Does nothing yet\n{why}")));
            }
            if z < ZOOM_BLOCKS {
                continue;
            }
            let size = (6.5 * z).max(4.0);
            c.text(
                nx + 4.0 * z,
                ny + (HEAD * z - size) / 2.0,
                size,
                rgba(0xffffff, 1.0),
                &Canvas::fit(size, nw - 8.0 * z, n.kind.label()),
            );
            if z < ZOOM_TITLES {
                // Zoomed far out: the box and its title are all that can be read.
                continue;
            }
            for (port, (name, kind)) in n.kind.inputs().iter().enumerate() {
                let (px, py) = self.in_pos(n, port);
                let wired = n.inputs.get(port).copied().flatten().is_some();
                c.circle(
                    px,
                    py,
                    PORT_R * z,
                    rgba(wire_colour(*kind), if wired { 1.0 } else { 0.35 }),
                    rgba(0x000000, 0.8),
                    1.0,
                );
                c.text(
                    px + 5.0 * z,
                    py - 2.5 * z,
                    5.0 * z,
                    rgba(0x9a9ab0, 1.0),
                    name,
                );
                let r = PORT_R * z + 3.0;
                self.tips.push((
                    [px - r, py - r, r * 2.0, r * 2.0],
                    format!("Input: {name}\n{}", port_about(*kind, true)),
                ));
            }
            if let Some(p) = n.kind.output() {
                let (px, py) = self.out_pos(n);
                c.circle(
                    px,
                    py,
                    PORT_R * z,
                    rgba(wire_colour(p), 1.0),
                    rgba(0x000000, 0.8),
                    1.0,
                );
                let r = PORT_R * z + 3.0;
                self.tips.push((
                    [px - r, py - r, r * 2.0, r * 2.0],
                    format!(
                        "Output\n{}\nDrag from here to an input of the same colour.",
                        port_about(p, false)
                    ),
                ));
            }
            // Parameters sit on the right of the input rows.
            for (i, d) in n.kind.params().iter().enumerate() {
                let line = format!("{} {}", d.name, self.node_param_text(n, i, true));
                let ty = ny + (HEAD + ROW * (i as f32 + 0.5)) * z - 2.5 * z;
                let tw = Canvas::text_width(5.0 * z, &line);
                c.text(
                    nx + nw - tw - 4.0 * z,
                    ty,
                    5.0 * z,
                    rgba(0xe6e6f0, 1.0),
                    &line,
                );
            }
            let mut extra = n.kind.inputs().len().max(n.kind.params().len()).max(1);
            let mut line = |c: &mut Canvas, text: String, col: u32| {
                let ty = ny + (HEAD + ROW * (extra as f32 + 0.5)) * z - 2.5 * z;
                c.text(
                    nx + 4.0 * z,
                    ty,
                    5.0 * z,
                    rgba(col, 1.0),
                    &Canvas::fit(5.0 * z, nw - 8.0 * z, &text),
                );
                extra += 1;
            };
            if n.kind.needs_target() {
                match n.target {
                    Some(t) => line(c, t.label(), 0xF0B64A),
                    None => line(c, "no target".into(), 0xE2463C),
                }
            }
            if n.kind.uses_ease() {
                line(c, format!("ease {}", n.ease.label()), 0x9a9ab0);
            }
        }
        c.clip_since(start, x, y, w, h);
        let hint = if graph.is_empty() {
            "Pick a node on the right to add it. Drag from a dot to a dot to wire them."
        } else {
            "drag: move   dot to dot: wire   right-click: delete   wheel: zoom   drag empty: pan"
        };
        c.text(x + 4.0, y + h - 9.0, 6.0, rgba(0x6a6a80, 1.0), hint);
        self.draw_first_steps(c, &graph);
    }

    /// A card over the canvas with the first steps of a first effect, each ticked off as it is
    /// done. It goes away once all four are, or when the graph has grown past a first try.
    fn draw_first_steps(&mut self, c: &mut Canvas, graph: &crate::tko::graph::Graph) {
        let steps = graph.first_steps();
        if steps.iter().all(|s| s.0) || graph.nodes.len() > 6 {
            return;
        }
        let [x, y, w, _] = self.layout.nodes;
        let cw = (w - 16.0).min(330.0);
        let ch = 14.0 + steps.len() as f32 * 11.0 + 14.0;
        let (cx, cy) = (x + w - cw - 8.0, y + 8.0);
        c.round_rect(
            [cx, cy],
            [cw, ch],
            3.0,
            rgba(0x0c0c12, 0.92),
            rgba(0x4a4a68, 1.0),
            1.0,
        );
        c.text(
            cx + 8.0,
            cy + 5.0,
            6.5,
            rgba(0xffffff, 1.0),
            "Your first effect",
        );
        for (i, (done, text)) in steps.iter().enumerate() {
            let ty = cy + 17.0 + i as f32 * 11.0;
            c.text(
                cx + 8.0,
                ty,
                6.0,
                rgba(if *done { 0x6fdc8c } else { 0x9a9ab0 }, 1.0),
                &if *done {
                    "OK".to_string()
                } else {
                    (i + 1).to_string()
                },
            );
            c.text(
                cx + 24.0,
                ty,
                6.0,
                rgba(if *done { 0x6fdc8c } else { 0xd0d0e0 }, 1.0),
                &Canvas::fit(6.0, cw - 32.0, text),
            );
        }
        c.text(
            cx + 8.0,
            cy + ch - 11.0,
            5.5,
            rgba(0x7a7a90, 1.0),
            "Then press Space: the playfield breathes. Help > Manual explains every node.",
        );
    }

    /// The thin strip above the canvas: where the playhead is, and a click moves it.
    pub(super) fn draw_node_strip(&mut self, c: &mut Canvas) {
        let [x, y, w, h] = self.layout.strip;
        c.round_rect(
            [x, y],
            [w, h],
            3.0,
            rgba(0x15151d, 0.95),
            rgba(0x2a2a38, 1.0),
            1.0,
        );
        let start = c.shapes.len();
        for o in &self.ed.chart.objects {
            let ox = self.note_x(o.time);
            if (x..x + w).contains(&ox) {
                c.rect(ox - 1.0, y + h / 2.0 - 3.0, 2.0, 6.0, rgba(0x9a9ab0, 0.8));
            }
        }
        if let Some(g) = self.ed.gimmick() {
            let marks: Vec<f64> = g.marks.iter().map(|m| m.time).collect();
            for (i, m) in marks.iter().enumerate() {
                let mx = self.x_of(*m);
                if (x..x + w).contains(&mx) {
                    c.rect(mx - 0.5, y + 1.0, 1.0, h - 2.0, rgba(super::MARK_COL, 0.9));
                    c.text(
                        mx + 2.0,
                        y + 1.5,
                        5.5,
                        rgba(super::MARK_COL, 1.0),
                        &format!("{}", i + 1),
                    );
                }
            }
        }
        let px = self.x_of(self.ed.time);
        c.rect(px - 0.75, y + 1.0, 1.5, h - 2.0, rgba(0xffffff, 0.9));
        c.clip_since(start, x, y, w, h);
    }

    // ---- dropdowns ----

    /// The target or recipe list over the canvas.
    pub(super) fn draw_node_pick(&mut self, c: &mut Canvas) {
        let Some(pick) = self.n_pick else { return };
        let [cx, cy, cw, _] = self.layout.nodes;
        let w = cw.min(540.0) - 16.0;
        let (x, y) = (cx + 8.0, cy + 8.0);
        let h = match pick {
            Pick::Target => 190.0,
            Pick::Recipe => {
                let cols = ((w - 16.0) / 158.0).floor().max(1.0) as usize;
                16.0 + Recipe::ALL.len().div_ceil(cols) as f32 * 22.0 + 30.0
            }
        };
        c.round_rect(
            [x, y],
            [w, h],
            4.0,
            rgba(0x1b1b26, 0.99),
            rgba(0x6a6a88, 1.0),
            1.0,
        );
        match pick {
            Pick::Recipe => {
                // A grid of buttons; the one under the pointer says what it does below.
                let cols = ((w - 16.0) / 158.0).floor().max(1.0) as usize;
                let rows = Recipe::ALL.len().div_ceil(cols);
                let mut said = None;
                for (i, r) in Recipe::ALL.iter().enumerate() {
                    let (col, row) = (i % cols, i / cols);
                    let rect = [
                        x + 8.0 + col as f32 * 158.0,
                        y + 8.0 + row as f32 * 22.0,
                        150.0,
                        18.0,
                    ];
                    self.button(c, rect, r.label(), Btn::N(NAct::AddRecipe(*r)), false);
                    if c.hovered(rect[0], rect[1], rect[2], rect[3]) {
                        said = Some(*r);
                    }
                }
                let line = said.map_or(
                    "Pick one: its nodes are added, wired, and can be edited after.".to_string(),
                    |r| r.about().to_string(),
                );
                let ty = y + 14.0 + rows as f32 * 22.0;
                for (i, l) in wrap(6.0, w - 16.0, &line).iter().take(2).enumerate() {
                    c.text(x + 8.0, ty + i as f32 * 8.0, 6.0, rgba(0x9a9ab0, 1.0), l);
                }
            }
            Pick::Target => {
                let current = self
                    .n_sel
                    .and_then(|id| self.ed.node(id))
                    .and_then(|n| n.target);
                let col_w = (w - 16.0) / 4.0;
                let groups: Vec<FieldProp> = Vec::new();
                drop(groups);
                for (k, grp) in PropGroup::ALL.iter().enumerate() {
                    let gx = x + 8.0 + k as f32 * col_w;
                    c.text(gx, y + 8.0, 6.5, rgba(0xF0B64A, 1.0), grp.label());
                    let mut ry = y + 20.0;
                    for p in FieldProp::ALL.iter().filter(|p| p.group() == *grp) {
                        let rect = [gx, ry, col_w - 6.0, 15.0];
                        let t = Target::Field(*p);
                        self.button(
                            c,
                            rect,
                            p.name(),
                            Btn::N(NAct::SetTarget(t)),
                            current == Some(t),
                        );
                        self.tips
                            .push((rect, format!("{}\n{}", p.name(), p.about())));
                        ry += 17.0;
                    }
                }
                let gx = x + 8.0 + 3.0 * col_w;
                let count = self.ed.gimmick().map_or(0, |g| g.groups.len());
                if count == 0 {
                    c.text(gx, y + 8.0, 6.5, rgba(0xF0B64A, 1.0), "Notes");
                    self.paragraph(
                        c,
                        gx,
                        y + 20.0,
                        col_w - 6.0,
                        6.0,
                        0x9a9ab0,
                        "Make a group of notes first (Keyframes page, or Group from selection).",
                    );
                } else {
                    let g = self.n_pick_group.min(count - 1);
                    let name = self
                        .ed
                        .gimmick()
                        .map(|gm| gm.groups[g].name.clone())
                        .filter(|n| !n.is_empty())
                        .unwrap_or_else(|| format!("Group {}", g + 1));
                    c.text(
                        gx,
                        y + 8.0,
                        6.5,
                        rgba(0xF0B64A, 1.0),
                        &Canvas::fit(6.5, col_w - 40.0, &name),
                    );
                    self.button(
                        c,
                        [gx + col_w - 36.0, y + 5.0, 14.0, 13.0],
                        "<",
                        Btn::N(NAct::PickGroup(-1)),
                        false,
                    );
                    self.button(
                        c,
                        [gx + col_w - 20.0, y + 5.0, 14.0, 13.0],
                        ">",
                        Btn::N(NAct::PickGroup(1)),
                        false,
                    );
                    let mut ry = y + 20.0;
                    for p in NoteProp::ALL {
                        let rect = [gx, ry, col_w - 6.0, 15.0];
                        let t = Target::Group(g, p);
                        self.button(
                            c,
                            rect,
                            p.name(),
                            Btn::N(NAct::SetTarget(t)),
                            current == Some(t),
                        );
                        self.tips
                            .push((rect, format!("{}\n{}", p.name(), p.about())));
                        ry += 17.0;
                    }
                }
            }
        }
    }

    // ---- the page ----

    /// Buttons from `x`, wrapping to the next line at `x + inner`. Returns where the next line
    /// starts.
    fn wrapped(
        &mut self,
        c: &mut Canvas,
        x: f32,
        mut y: f32,
        inner: f32,
        items: &[(String, Btn, bool)],
    ) -> f32 {
        let mut cx = x;
        for (k, (label, b, on)) in items.iter().enumerate() {
            let l = look::look_of(*b);
            let w = look::width(l, label);
            if cx + w > x + inner && cx > x {
                cx = x;
                y += 18.0;
            }
            self.styled(c, [cx, y, w, 16.0], label, *b, *on, l);
            let next = items.get(k + 1).map_or(Look::Plain, |n| look::look_of(n.1));
            cx += w + look::gap(l, next);
        }
        y + 18.0
    }

    pub(super) fn draw_nodes_page(&mut self, c: &mut Canvas, x: f32, mut y: f32, inner: f32) {
        let dim = rgba(0x9a9ab0, 1.0);
        let white = rgba(0xffffff, 1.0);
        if !self.ed.is_tko() {
            self.draw_tko_explainer(c, x, y, inner);
            return;
        }
        let cats: Vec<(String, Btn, bool)> = Category::ALL
            .iter()
            .enumerate()
            .map(|(i, cat)| {
                (
                    cat.label().to_string(),
                    Btn::N(NAct::Cat(i)),
                    self.n_cat == i,
                )
            })
            .collect();
        let mut cats = cats;
        cats.push((
            "+ Recipe".into(),
            Btn::N(NAct::Open(Pick::Recipe)),
            self.n_pick == Some(Pick::Recipe),
        ));
        y = self.wrapped(c, x, y, inner, &cats);
        let cat = Category::ALL[self.n_cat.min(Category::ALL.len() - 1)];
        let kinds: Vec<(String, Btn, bool)> = Kind::ALL
            .iter()
            .filter(|k| k.category() == cat)
            .map(|k| (format!("+ {}", k.label()), Btn::N(NAct::Add(*k)), false))
            .collect();
        let before = self.buttons.len();
        y = self.wrapped(c, x, y, inner, &kinds) + 2.0;
        // Each kind says what it does when the pointer rests on it.
        for i in before..self.buttons.len() {
            if let (r, Btn::N(NAct::Add(k))) = self.buttons[i] {
                self.tips.push((r, format!("{}\n{}", k.label(), k.about())));
            }
        }
        let Some(n) = self.n_sel.and_then(|id| self.ed.node(id)).cloned() else {
            c.text(
                x,
                y,
                6.5,
                dim,
                &Canvas::fit(6.5, inner, "Click a node to set its numbers."),
            );
            let seed = self.ed.gimmick().map_or(0, |g| g.graph.seed);
            c.text(x, y + 20.0, 6.5, white, &format!("Random seed: {seed}"));
            self.row(
                c,
                x + 150.0,
                y + 16.0,
                &[
                    ("-".into(), Btn::N(NAct::Seed(-1)), false),
                    ("+".into(), Btn::N(NAct::Seed(1)), false),
                    ("Fit".into(), Btn::N(NAct::Fit), false),
                    ("100%".into(), Btn::N(NAct::Home), false),
                ],
            );
            return;
        };
        c.text(
            x,
            y,
            7.0,
            white,
            &Canvas::fit(7.0, inner, &format!("#{}  {}", n.id, n.kind.label())),
        );
        y += 10.0;
        y = self.paragraph(c, x, y, inner, 6.0, 0x9a9ab0, n.kind.about());
        if let Some(why) = self.ed.gimmick().and_then(|g| g.graph.problem(n.id)) {
            y = self.paragraph(
                c,
                x,
                y,
                inner,
                6.0,
                0xE2463C,
                &format!("Does nothing yet: {why}"),
            );
        }
        let wide = inner.min(300.0);
        for (i, d) in n.kind.params().iter().enumerate() {
            let typing = self.n_typing.as_ref().filter(|(k, _)| *k == i);
            let label = match typing {
                Some((_, text)) => format!("{}: {text}_", d.name),
                None => format!("{}: {}", d.name, self.node_param_text(&n, i, false)),
            };
            self.button(
                c,
                [x, y, wide - 44.0, 15.0],
                &label,
                Btn::N(NAct::Edit(i)),
                typing.is_some(),
            );
            self.tips.push((
                [x, y, wide - 44.0, 15.0],
                format!(
                    "{}\nClick to type a number, or use - and +. Range {} to {}.",
                    d.name,
                    num(d.min),
                    num(d.max)
                ),
            ));
            self.row(
                c,
                x + wide - 40.0,
                y,
                &[
                    ("-".into(), Btn::N(NAct::Param(i, -1)), false),
                    ("+".into(), Btn::N(NAct::Param(i, 1)), false),
                ],
            );
            y += 17.0;
        }
        let mut row: Vec<(String, Btn, bool)> = Vec::new();
        if n.kind.params().first().is_some_and(|d| d.name == "group") {
            row.push((
                "Group from selection".into(),
                Btn::N(NAct::FromSelection),
                false,
            ));
        }
        if n.kind.needs_target() {
            let label = n.target.map_or("none".to_string(), |t| t.label());
            row.push((
                format!("Target: {label}"),
                Btn::N(NAct::Open(Pick::Target)),
                self.n_pick == Some(Pick::Target),
            ));
            row.push(("No target".into(), Btn::N(NAct::NoTarget), false));
        }
        if n.kind.uses_ease() {
            row.push((
                format!("Ease: {}", n.ease.label()),
                Btn::N(NAct::Ease),
                false,
            ));
        }
        if n.kind == Kind::Clip {
            row.push(("Next clip".into(), Btn::N(NAct::Clip), false));
        }
        row.push(("Delete".into(), Btn::N(NAct::Delete), false));
        self.wrapped(c, x, y, inner, &row);
    }

    pub(super) fn press_node_action(&mut self, a: NAct, shift: bool) {
        let k = if shift { 10 } else { 1 };
        match a {
            NAct::Cat(i) => {
                self.n_cat = i;
                self.n_pick = None;
            }
            NAct::Add(kind) => {
                let [_, _, w, h] = self.layout.nodes;
                let (cx, cy) = self.to_world(
                    self.layout.nodes[0] + w / 2.0,
                    self.layout.nodes[1] + h / 2.0,
                );
                let count = self.ed.gimmick().map_or(0, |g| g.graph.nodes.len());
                let jitter = (count % 6) as f32 * 14.0;
                match self.ed.add_node(
                    kind,
                    (cx - NODE_W / 2.0 + jitter).round(),
                    (cy - 20.0 + jitter).round(),
                ) {
                    Ok(id) => {
                        self.n_sel = Some(id);
                        self.mark_stale();
                    }
                    Err(e) => self.status = e,
                }
            }
            NAct::Param(i, dir) => {
                if let Some(id) = self.n_sel
                    && self.ed.step_node_param(id, i, f64::from(dir * k))
                {
                    self.mark_stale();
                }
            }
            NAct::Edit(i) => {
                if self.n_sel.is_some() {
                    self.n_typing = Some((i, String::new()));
                }
            }
            NAct::FromSelection => {
                let Some(id) = self.n_sel else { return };
                match self.ed.add_group_from_selection() {
                    Ok(g) => {
                        self.ed.set_node_param(id, 0, (g + 1) as f64);
                        self.status =
                            format!("the node now follows Group {} (the selected notes)", g + 1);
                        self.mark_stale();
                    }
                    Err(e) => self.status = e,
                }
            }
            NAct::Open(p) => {
                self.n_pick = if self.n_pick == Some(p) {
                    None
                } else {
                    Some(p)
                };
            }
            NAct::SetTarget(t) => {
                self.n_pick = None;
                if let Some(id) = self.n_sel
                    && self.ed.set_node_target(id, Some(t))
                {
                    self.mark_stale();
                }
            }
            NAct::PickGroup(d) => {
                let groups = self.ed.gimmick().map_or(0, |g| g.groups.len()).max(1);
                self.n_pick_group =
                    (self.n_pick_group as i32 + d).rem_euclid(groups as i32) as usize;
            }
            NAct::AddRecipe(r) => {
                self.n_pick = None;
                let [lx, ly, w, h] = self.layout.nodes;
                let (cx, cy) = self.to_world(lx + 30.0, ly + h / 2.0 - 40.0 + 0.0 * w);
                match self.ed.add_recipe(r, cx.round(), cy.round()) {
                    Ok(ids) => {
                        self.n_sel = ids.first().copied();
                        self.status = format!("added {}: {}", r.label(), r.about());
                        self.mark_stale();
                    }
                    Err(e) => self.status = e,
                }
            }
            NAct::NoTarget => {
                if let Some(id) = self.n_sel
                    && self.ed.set_node_target(id, None)
                {
                    self.mark_stale();
                }
            }
            NAct::Ease => {
                if let Some(id) = self.n_sel
                    && self.ed.next_node_ease(id)
                {
                    self.mark_stale();
                }
            }
            NAct::Clip => {
                if let Some(id) = self.n_sel {
                    self.n_clip = (self.n_clip + 1) % 3;
                    if self.ed.set_node_clip(id, self.n_clip) {
                        self.mark_stale();
                    }
                }
            }
            NAct::Delete => self.delete_selected_node(),
            NAct::Seed(dir) => {
                if self.ed.step_seed(i64::from(dir * k)) {
                    self.mark_stale();
                }
            }
            NAct::Home => {
                self.n_pan = (0.0, 0.0);
                self.n_zoom = 1.0;
            }
            NAct::Fit => self.node_fit(),
        }
    }

    pub(super) fn delete_selected_node(&mut self) {
        if let Some(id) = self.n_sel.take()
            && self.ed.delete_node(id)
        {
            self.mark_stale();
        }
    }

    // ---- the mouse ----

    pub(super) fn node_down(&mut self, x: f32, y: f32) {
        match self.node_hit(x, y) {
            Some(Hit::Out(id)) => {
                self.n_sel = Some(id);
                self.drag = Drag::Wire { from: id };
            }
            Some(Hit::In(id, port)) => {
                self.n_sel = Some(id);
                // Picking up a wire by its end moves it.
                let from = self
                    .ed
                    .node(id)
                    .and_then(|n| n.inputs.get(port).copied().flatten());
                if let Some(from) = from
                    && self.ed.disconnect_node(id, port)
                {
                    self.drag = Drag::Wire { from };
                    self.mark_stale();
                }
            }
            Some(Hit::Body(id)) => {
                self.n_sel = Some(id);
                if let Some(n) = self.ed.node(id) {
                    let (wx, wy) = self.to_world(x, y);
                    self.drag = Drag::Node {
                        id,
                        dx: wx - n.x,
                        dy: wy - n.y,
                    };
                }
            }
            None => {
                self.n_sel = None;
                self.drag = Drag::Pan { last: (x, y) };
            }
        }
    }

    pub(super) fn node_drag(&mut self, x: f32, y: f32) {
        match self.drag {
            Drag::Node { id, dx, dy } => {
                let (wx, wy) = self.to_world(x, y);
                self.ed
                    .move_node(id, (wx - dx).round(), (wy - dy).round(), true);
            }
            Drag::Pan { last } => {
                self.n_pan.0 -= (x - last.0) / self.n_zoom;
                self.n_pan.1 -= (y - last.1) / self.n_zoom;
                self.drag = Drag::Pan { last: (x, y) };
            }
            _ => {}
        }
    }

    /// A wire was let go at (`x`, `y`).
    pub(super) fn wire_dropped(&mut self, from: u32, x: f32, y: f32) {
        if let Some(Hit::In(to, port)) = self.node_hit(x, y) {
            match self.ed.connect_nodes(from, to, port) {
                Ok(()) => self.mark_stale(),
                Err(e) => self.status = e,
            }
        }
    }

    pub(super) fn node_right(&mut self, x: f32, y: f32) {
        match self.node_hit(x, y) {
            Some(Hit::Body(id) | Hit::Out(id)) => {
                self.n_sel = Some(id);
                self.delete_selected_node();
            }
            Some(Hit::In(id, port)) if self.ed.disconnect_node(id, port) => self.mark_stale(),
            Some(Hit::In(..)) | None => {}
        }
    }

    /// Zoom about the pointer.
    /// Pans and zooms so every node is in view.
    pub(super) fn node_fit(&mut self) {
        let [_, _, w, h] = self.layout.nodes;
        let Some(g) = self.ed.gimmick() else { return };
        if g.graph.nodes.is_empty() {
            self.n_pan = (0.0, 0.0);
            self.n_zoom = 1.0;
            return;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for n in &g.graph.nodes {
            x0 = x0.min(n.x);
            y0 = y0.min(n.y);
            x1 = x1.max(n.x + NODE_W);
            y1 = y1.max(n.y + HEAD + ROW * rows(n) as f32);
        }
        let pad = 12.0;
        let z = ((w - 2.0 * pad) / (x1 - x0).max(1.0)).min((h - 2.0 * pad) / (y1 - y0).max(1.0));
        self.n_zoom = z.clamp(MIN_ZOOM, 1.0);
        self.n_pan = (
            (x0 + x1) / 2.0 - w / 2.0 / self.n_zoom,
            (y0 + y1) / 2.0 - h / 2.0 / self.n_zoom,
        );
    }

    pub(super) fn node_zoom(&mut self, notches: i32) {
        let (mx, my) = self.mouse;
        let before = self.to_world(mx, my);
        // Wheel up (away from you) zooms in, like every canvas.
        let f = 1.18f32.powi(notches.clamp(-8, 8));
        self.n_zoom = (self.n_zoom * f).clamp(MIN_ZOOM, MAX_ZOOM);
        let after = self.to_world(mx, my);
        self.n_pan.0 += before.0 - after.0;
        self.n_pan.1 += before.1 - after.1;
    }
}
