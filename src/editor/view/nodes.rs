//! The Nodes page of the Gimmicks panel and the canvas that takes the timeline's place while it is
//! open: boxes for events, logic, values and actions, wires between their ports, dragged the way
//! a node editor is. Everything here edits `Graph` through the editor, one undo step each.

use super::{Btn, Drag, View, inside};
use crate::render::{Canvas, rgba};
use crate::tko::graph::{Category, Kind, Node, Port};

/// What the Nodes page can be told to do.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum NAct {
    Cat(usize),
    Add(Kind),
    Param(usize, i32),
    Target(i32),
    NoTarget,
    Ease,
    Clip,
    Delete,
    Seed(i32),
    Home,
}

const NODE_W: f32 = 104.0;
const HEAD: f32 = 13.0;
const ROW: f32 = 11.0;
const PORT_R: f32 = 3.0;
const GRAB: f32 = 6.0;
/// How long after it fires a node stays lit (ms).
const LIT_MS: f64 = 250.0;

pub const MIN_ZOOM: f32 = 0.6;
pub const MAX_ZOOM: f32 = 1.6;

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
        self.gimmicks && self.g_page == 3 && self.ed.is_tko()
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
        let step = 40.0 * self.n_zoom;
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
                View::wire(
                    c,
                    self.out_pos(src),
                    self.in_pos(n, port),
                    wire_colour(kind),
                    if on { 1.0 } else { 0.55 },
                );
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
            let size = 6.5 * z;
            c.text(
                nx + 4.0 * z,
                ny + (HEAD * z - size) / 2.0,
                size,
                rgba(0xffffff, 1.0),
                &Canvas::fit(size, nw - 8.0 * z, n.kind.label()),
            );
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
            }
            // Parameters sit on the right of the input rows.
            for (i, d) in n.kind.params().iter().enumerate() {
                let line = format!("{} {}", d.name, num(n.param(i)));
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
            let ox = self.x_of(o.time);
            if (x..x + w).contains(&ox) {
                c.rect(ox - 1.0, y + h / 2.0 - 3.0, 2.0, 6.0, rgba(0x9a9ab0, 0.8));
            }
        }
        let px = self.x_of(self.ed.time);
        c.rect(px - 0.75, y + 1.0, 1.5, h - 2.0, rgba(0xffffff, 0.9));
        c.clip_since(start, x, y, w, h);
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
        for (label, b, on) in items {
            let w = Canvas::text_width(7.0, label) + 12.0;
            if cx + w > x + inner && cx > x {
                cx = x;
                y += 18.0;
            }
            self.button(c, [cx, y, w, 16.0], label, *b, *on);
            cx += w + 4.0;
        }
        y + 18.0
    }

    pub(super) fn draw_nodes_page(&mut self, c: &mut Canvas, x: f32, mut y: f32, inner: f32) {
        let dim = rgba(0x9a9ab0, 1.0);
        let white = rgba(0xffffff, 1.0);
        if !self.ed.is_tko() {
            c.text(
                x,
                y,
                7.0,
                white,
                &Canvas::fit(7.0, inner, "Nodes need a .tko map."),
            );
            c.text(
                x,
                y + 12.0,
                6.0,
                dim,
                &Canvas::fit(6.0, inner, "Open the Keyframes page and press Make .tko."),
            );
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
        y = self.wrapped(c, x, y, inner, &cats);
        let cat = Category::ALL[self.n_cat.min(Category::ALL.len() - 1)];
        let kinds: Vec<(String, Btn, bool)> = Kind::ALL
            .iter()
            .filter(|k| k.category() == cat)
            .map(|k| (format!("+ {}", k.label()), Btn::N(NAct::Add(*k)), false))
            .collect();
        y = self.wrapped(c, x, y, inner, &kinds) + 2.0;
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
                    ("Reset view".into(), Btn::N(NAct::Home), false),
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
        for (i, d) in n.kind.params().iter().enumerate() {
            c.text(
                x,
                y + 4.0,
                6.5,
                white,
                &format!("{}: {}", d.name, num(n.param(i))),
            );
            self.row(
                c,
                x + 150.0,
                y,
                &[
                    ("-".into(), Btn::N(NAct::Param(i, -1)), false),
                    ("+".into(), Btn::N(NAct::Param(i, 1)), false),
                ],
            );
            y += 17.0;
        }
        let mut row: Vec<(String, Btn, bool)> = Vec::new();
        if n.kind.needs_target() {
            row.push(("< target".into(), Btn::N(NAct::Target(-1)), false));
            row.push(("target >".into(), Btn::N(NAct::Target(1)), false));
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
            NAct::Cat(i) => self.n_cat = i,
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
            NAct::Target(dir) => {
                if let Some(id) = self.n_sel
                    && self.ed.cycle_node_target(id, dir)
                {
                    self.mark_stale();
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
    pub(super) fn node_zoom(&mut self, notches: i32) {
        let (mx, my) = self.mouse;
        let before = self.to_world(mx, my);
        let f = if notches > 0 { 0.85 } else { 1.0 / 0.85 };
        self.n_zoom = (self.n_zoom * f).clamp(MIN_ZOOM, MAX_ZOOM);
        let after = self.to_world(mx, my);
        self.n_pan.0 += before.0 - after.0;
        self.n_pan.1 += before.1 - after.1;
    }
}
