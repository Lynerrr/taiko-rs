//! The drum in menus (`Config::drum_menus`): the rims move, either face picks. In song select
//! Tab takes the selector from the song list to the rows of buttons above and below it.

use winit::keyboard::KeyCode;

use crate::hit::{Canvas, Hit};
use crate::render::{VIRTUAL_HEIGHT, accent, rgba};

/// Where song select's selector is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zone {
    List,
    /// Button `at` of the row above the list, or below it.
    Row {
        bottom: bool,
        at: usize,
    },
}

/// Where a drum key was pressed.
#[derive(Clone, Copy)]
pub enum Place {
    /// A list going up and down.
    Menu,
    Select(Zone),
}

/// The key that does what drum key `drum` (0 left rim, 1 and 2 the face, 3 right rim) does.
pub fn key_of(drum: usize, place: Place) -> KeyCode {
    let along_a_row = matches!(place, Place::Select(Zone::Row { .. }));
    match drum {
        0 if along_a_row => KeyCode::ArrowLeft,
        3 if along_a_row => KeyCode::ArrowRight,
        0 => KeyCode::ArrowUp,
        3 => KeyCode::ArrowDown,
        _ => KeyCode::Enter,
    }
}

/// Song select's buttons as drawn, left to right.
#[derive(Default, Debug, PartialEq)]
pub struct Rows {
    top: Vec<(Hit, [f32; 4])>,
    bottom: Vec<(Hit, [f32; 4])>,
}

/// What the selector can land on: whichever of these song select drew, wherever it put them.
fn is_button(hit: Hit) -> bool {
    matches!(
        hit,
        Hit::ListBack
            | Hit::TextField
            | Hit::SortList
            | Hit::CollectionPick
            | Hit::ModsButton
            | Hit::BoardPage
            | Hit::Key(KeyCode::F2)
            | Hit::Tab(_)
    )
}

impl Rows {
    pub fn of(c: &Canvas) -> Rows {
        let mut rows = Rows::default();
        for b in c.hits.iter().filter(|b| is_button(b.hit)) {
            let row = if b.rect[1] < VIRTUAL_HEIGHT / 2.0 {
                &mut rows.top
            } else {
                &mut rows.bottom
            };
            if !row.iter().any(|(hit, _)| *hit == b.hit) {
                row.push((b.hit, b.rect));
            }
        }
        for row in [&mut rows.top, &mut rows.bottom] {
            row.sort_by(|a, b| a.1[0].total_cmp(&b.1[0]));
        }
        rows
    }

    fn row(&self, bottom: bool) -> &[(Hit, [f32; 4])] {
        if bottom { &self.bottom } else { &self.top }
    }

    pub fn get(&self, bottom: bool, at: usize) -> Option<Hit> {
        self.row(bottom).get(at).map(|(hit, _)| *hit)
    }
}

impl Zone {
    /// Where Tab goes: the list, the row above, the row below, the list; Shift goes back. A row
    /// with nothing on it is passed over.
    pub fn tabbed(self, rows: &Rows, back: bool) -> Zone {
        let order = [
            Zone::List,
            Zone::Row {
                bottom: false,
                at: rows
                    .top
                    .iter()
                    .position(|b| b.0 == Hit::TextField)
                    .unwrap_or(0),
            },
            Zone::Row {
                bottom: true,
                at: 0,
            },
        ];
        let here = match self {
            Zone::List => 0,
            Zone::Row { bottom: false, .. } => 1,
            Zone::Row { bottom: true, .. } => 2,
        };
        let step = if back { 2 } else { 1 };
        (1..=3)
            .map(|n| order[(here + n * step) % 3])
            .find(|z| z.within(rows) == *z)
            .unwrap_or(Zone::List)
    }

    /// `by` buttons along the row, round its ends.
    pub fn moved(self, rows: &Rows, by: i32) -> Zone {
        let Zone::Row { bottom, at } = self else {
            return self;
        };
        let n = rows.row(bottom).len() as i32;
        if n == 0 {
            return Zone::List;
        }
        Zone::Row {
            bottom,
            at: (at as i32 + by).rem_euclid(n) as usize,
        }
    }

    /// Itself, or the list if its button is gone (the buttons change with the window and mods).
    pub fn within(self, rows: &Rows) -> Zone {
        match self {
            Zone::Row { bottom, at } if rows.get(bottom, at).is_none() => Zone::List,
            zone => zone,
        }
    }
}

/// A ring round the button the selector is on.
pub fn draw(c: &mut Canvas, zone: Zone, rows: &Rows) {
    let Zone::Row { bottom, at } = zone else {
        return;
    };
    if let Some((_, [x, y, w, h])) = rows.row(bottom).get(at) {
        c.round_rect(
            [x - 2.0, y - 2.0],
            [w + 4.0, h + 4.0],
            4.0,
            rgba(0x000000, 0.0),
            rgba(accent(), 1.0),
            1.5,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Rows {
        let b = |hit, x| (hit, [x, 0.0, 10.0, 10.0]);
        Rows {
            top: vec![
                b(Hit::ListBack, 0.0),
                b(Hit::TextField, 20.0),
                b(Hit::SortList, 40.0),
            ],
            bottom: vec![b(Hit::ModsButton, 0.0), b(Hit::Tab(1), 20.0)],
        }
    }

    #[test]
    fn the_rims_move_and_the_face_picks() {
        let list = Place::Select(Zone::List);
        assert_eq!(key_of(0, list), KeyCode::ArrowUp);
        assert_eq!(key_of(3, list), KeyCode::ArrowDown);
        assert_eq!(key_of(1, list), KeyCode::Enter);
        assert_eq!(key_of(2, list), KeyCode::Enter);
        let row = Place::Select(Zone::Row {
            bottom: true,
            at: 0,
        });
        assert_eq!(key_of(0, row), KeyCode::ArrowLeft);
        assert_eq!(key_of(3, row), KeyCode::ArrowRight);
        assert_eq!(key_of(1, row), KeyCode::Enter);
        assert_eq!(key_of(3, Place::Menu), KeyCode::ArrowDown);
    }

    #[test]
    fn tab_goes_list_top_bottom_and_round_and_shift_goes_back() {
        let r = rows();
        let top = Zone::List.tabbed(&r, false);
        assert_eq!(
            top,
            Zone::Row {
                bottom: false,
                at: 1
            },
            "the search box first"
        );
        let bottom = top.tabbed(&r, false);
        assert_eq!(
            bottom,
            Zone::Row {
                bottom: true,
                at: 0
            }
        );
        assert_eq!(bottom.tabbed(&r, false), Zone::List);
        assert_eq!(Zone::List.tabbed(&r, true), bottom);
        assert_eq!(bottom.tabbed(&r, true).tabbed(&r, true), Zone::List);
    }

    #[test]
    fn an_empty_row_is_passed_over() {
        let r = Rows {
            bottom: Vec::new(),
            ..rows()
        };
        let top = Zone::List.tabbed(&r, false);
        assert_eq!(top.tabbed(&r, false), Zone::List);
        assert_eq!(Zone::List.tabbed(&Rows::default(), false), Zone::List);
    }

    #[test]
    fn moving_goes_round_the_ends_of_a_row() {
        let r = rows();
        let first = Zone::Row {
            bottom: false,
            at: 0,
        };
        assert_eq!(
            first.moved(&r, -1),
            Zone::Row {
                bottom: false,
                at: 2
            }
        );
        assert_eq!(first.moved(&r, -1).moved(&r, 1), first);
    }

    #[test]
    fn a_button_that_went_puts_the_selector_back_on_the_list() {
        let gone = Zone::Row {
            bottom: true,
            at: 9,
        };
        assert_eq!(gone.within(&rows()), Zone::List);
    }
}
