//! Bind the focused reusable text component to the screen's model.
use crate::text_input::{Field, Modifiers, Rules, TextInput};
use crate::*;

impl State {
    fn input_value(&self) -> Option<(Field, String)> {
        if self.shot_view.is_some()
            || self.quit_asked
            || self.crash_shown()
            || self.profile.is_some()
            || self.terms.is_some()
            || self.splash.is_some()
            || self.guide.page.is_some()
            || self.changelog.is_some()
            || self.medals.is_some()
            || self.unlock.showing()
        {
            return None;
        }
        if self.social_keys() {
            return Some((Field::Social, self.social.input.clone()));
        }
        if self.in_settings() {
            if self.settings_ui.binding.is_some() {
                return None;
            }
            if let Some(e) = &self.settings_ui.editing {
                return Some((Field::Setting(e.item), e.text.clone()));
            }
            if let Some(d) = &self.settings_ui.dropdown {
                return d
                    .editing
                    .as_ref()
                    .map(|(_, s)| (Field::Resolution, s.clone()));
            }
            if let Some(c) = &self.settings_ui.choices {
                return c
                    .searchable()
                    .then(|| (Field::FontSearch, c.search.clone()));
            }
            return Some((Field::SettingsSearch, self.settings_ui.search.clone()));
        }
        if let Some(s) = &self.preset_naming {
            return Some((Field::Preset, s.clone()));
        }
        if let Some(n) = &self.select.naming {
            return Some((Field::Collection, n.text.clone()));
        }
        if let Some(s) = &self.random_setup {
            return Some((Field::Random, s.clone()));
        }
        if let Some(t) = self.universe_setup.as_ref().and_then(|s| s.typing.as_ref()) {
            let (multiverse::Typing::Name(s) | multiverse::Typing::Colours(s)) = t;
            return Some((Field::Universe, s.clone()));
        }
        if let Some(p) = self.player.as_ref().filter(|p| p.searching) {
            return Some((Field::PlayerSearch, p.list.filter.clone()));
        }
        if self.scene.screen_is::<MpMenu>() {
            use multiplayer::Row;
            let (id, s) = match self.mp_menu.row() {
                Row::User => (0, &self.mp_menu.user),
                Row::Password => (1, &self.mp_menu.password),
                Row::Email => (2, &self.mp_menu.email),
                Row::Code => (3, &self.mp_menu.code),
                Row::NewPassword => (4, &self.mp_menu.new_password),
                Row::JoinPassword => (5, &self.mp_menu.join_password),
                _ => return None,
            };
            return Some((Field::Account(id), s.clone()));
        }
        if self.in_lobby()
            && let Some(s) = &self.mp
        {
            if let Some(t) = &s.chat_input {
                return Some((Field::LobbyChat, t.clone()));
            }
            if let Some(t) = &s.edit_password {
                return Some((Field::LobbyPassword, t.clone()));
            }
        }
        if self.scene.screen().is_some_and(|s| s.previews_remote()) && !self.on_menu() {
            return Some((Field::BrowseSearch, self.browse.query.clone()));
        }
        if self.scene.is_select()
            && self.select.searching
            && !self.on_menu()
            && self.mod_select.is_none()
        {
            return Some((Field::SongSearch, self.select.filter.clone()));
        }
        None
    }

    pub(super) fn sync_text_input(&mut self) -> Option<Field> {
        let Some((field, text)) = self.input_value() else {
            self.input_focus = None;
            self.input_drag = false;
            return None;
        };
        if self.input_focus != Some(field) {
            self.text_input = TextInput::new(text);
            self.input_focus = Some(field);
            self.input_engaged = false;
            if matches!(field, Field::Setting(_))
                && self.settings_ui.editing.as_ref().is_some_and(|e| e.fresh)
            {
                self.text_input.select_all();
            }
        } else {
            self.text_input.sync(&text);
        }
        Some(field)
    }

    fn store_text_input(&mut self, field: Field) {
        let text = self.text_input.text.clone();
        let caret = self.text_input.text[..self.text_input.caret]
            .chars()
            .count();
        match field {
            Field::Social => {
                if matches!(
                    self.social.row(self.mp.is_some()),
                    social::Row::Friend(_) | social::Row::Lobby
                ) && !text.is_empty()
                {
                    self.social.open_conversation();
                }
                self.social.input = text;
            }
            Field::SettingsSearch => {
                if self.settings_ui.search != text {
                    self.search_settings(text);
                }
            }
            Field::FontSearch => {
                if let Some(p) = &mut self.settings_ui.choices
                    && p.search != text
                {
                    p.set_search(text);
                    self.mob.reset_choices_scroll();
                }
            }
            Field::Setting(_) => {
                if let Some(e) = &mut self.settings_ui.editing {
                    e.text = text;
                    e.caret = caret;
                    e.fresh = false;
                }
            }
            Field::Resolution => {
                if let Some(d) = &mut self.settings_ui.dropdown {
                    if let Some((_, t)) = &mut d.editing {
                        *t = text;
                    }
                    d.caret = caret;
                }
            }
            Field::SongSearch => {
                if self.select.filter != text {
                    self.select.filter = text;
                    self.select.refilter(&self.library);
                }
            }
            Field::BrowseSearch => self.browse.set_query(text),
            Field::PlayerSearch => {
                if let Some(p) = &mut self.player
                    && p.list.filter != text
                {
                    p.search(text);
                }
            }
            Field::LobbyChat => {
                if let Some(s) = &mut self.mp {
                    s.chat_input = Some(text);
                    s.chat_caret = caret;
                }
            }
            Field::LobbyPassword => {
                if let Some(s) = &mut self.mp {
                    s.edit_password = Some(text);
                }
            }
            Field::Account(id) => {
                *match id {
                    0 => &mut self.mp_menu.user,
                    1 => &mut self.mp_menu.password,
                    2 => &mut self.mp_menu.email,
                    3 => &mut self.mp_menu.code,
                    4 => &mut self.mp_menu.new_password,
                    _ => &mut self.mp_menu.join_password,
                } = text;
            }
            Field::Preset => self.preset_naming = Some(text),
            Field::Collection => {
                if let Some(n) = &mut self.select.naming {
                    n.text = text;
                }
            }
            Field::Random => self.random_setup = Some(text),
            Field::Universe => {
                if let Some(t) = self.universe_setup.as_mut().and_then(|s| s.typing.as_mut()) {
                    let (multiverse::Typing::Name(s) | multiverse::Typing::Colours(s)) = t;
                    *s = text;
                }
            }
        }
    }

    pub(super) fn text_input_key(
        &mut self,
        code: KeyCode,
        typed: Option<&str>,
        repeat: bool,
    ) -> bool {
        let Some((field, value)) = self.input_value() else {
            self.input_focus = None;
            return false;
        };
        if matches!(
            code,
            KeyCode::Escape | KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Tab
        ) {
            self.input_engaged = false;
            return false;
        }
        // Empty implicit searches still leave navigation and row activation to the settings.
        if field == Field::SettingsSearch
            && value.is_empty()
            && !self.input_engaged
            && !keys::starts_text(code)
            && !keys::is_sign(code)
            && !(self.modifiers_ctrl || self.modifiers_super)
        {
            return false;
        }
        // Friend-list actions stay available until a message is actually being edited.
        if field == Field::Social
            && value.is_empty()
            && !self.social.in_conversation(self.mp.is_some())
            && !matches!(
                self.social.row(self.mp.is_some()),
                social::Row::AddFriend
                    | social::Row::Search
                    | social::Row::Global
                    | social::Row::AdminInput
            )
            && matches!(
                code,
                KeyCode::Delete
                    | KeyCode::ArrowLeft
                    | KeyCode::ArrowRight
                    | KeyCode::Home
                    | KeyCode::End
            )
        {
            return false;
        }
        self.sync_text_input();
        let name = field == Field::Social
            && (self.social.tab == social::Tab::Players
                || self.social.row(self.mp.is_some()) == social::Row::AddFriend);
        let max = match field {
            Field::Social if name => 16,
            Field::Social | Field::LobbyChat => 200,
            Field::FontSearch => 128,
            Field::Setting(_) => {
                if self.settings_ui.editing.as_ref().is_some_and(|e| {
                    matches!(
                        e.item,
                        Item::SongsFolder | Item::TjaFolder | Item::DataFolder
                    )
                }) {
                    400
                } else {
                    12
                }
            }
            Field::Resolution | Field::Random => 20,
            Field::Account(0 | 2) => 254,
            Field::Account(3) => 6,
            Field::Account(4 | 5) | Field::LobbyPassword => 32,
            Field::Account(_) => 128,
            Field::Preset => 40,
            Field::Collection => collections::MAX_NAME,
            _ => 4096,
        };
        let secret = matches!(field, Field::Account(1 | 4 | 5) | Field::LobbyPassword);
        let modifiers = Modifiers {
            ctrl: self.modifiers_ctrl,
            shift: self.modifiers_shift,
            alt: self.modifiers_alt,
            super_key: self.modifiers_super,
        };
        let used =
            self.text_input
                .key(code, typed, modifiers, repeat, Rules { max, secret }, |c| {
                    if name {
                        c.is_ascii_alphanumeric() || c == '_' || c == '-'
                    } else {
                        match field {
                            Field::Account(0 | 2) => c.is_ascii_graphic(),
                            Field::Account(3) | Field::Random => c.is_ascii_digit(),
                            Field::Account(4 | 5) | Field::LobbyPassword => c != ' ',
                            Field::Social | Field::LobbyChat => net::drawable_char(c),
                            _ => true,
                        }
                    }
                });
        if used {
            self.input_engaged = true;
            self.store_text_input(field);
        }
        used
    }

    pub(super) fn text_input_mouse(&mut self, pressed: bool) -> bool {
        if !pressed && self.input_drag {
            self.input_drag = false;
            return true;
        }
        if !pressed {
            return false;
        }
        let (x, y) = self.mouse_virtual();
        let Some(region) = self
            .input_regions
            .iter()
            .rev()
            .find(|r| r.contains(x, y))
            .cloned()
        else {
            return false;
        };
        if self.hit_at(x, y).is_some_and(|h| !self.is_text_field(h)) {
            return false;
        }
        if let Field::Account(id) = region.field {
            use multiplayer::Row;
            self.mp_menu.select(match id {
                0 => Row::User,
                1 => Row::Password,
                2 => Row::Email,
                3 => Row::Code,
                4 => Row::NewPassword,
                _ => Row::JoinPassword,
            });
        }
        // Clicking a search field focuses it before placement; other fields are already active.
        if region.field == Field::SongSearch {
            self.select.searching = true;
        }
        let Some(field) = self.sync_text_input() else {
            return false;
        };
        if field != region.field {
            return false;
        }
        self.text_input
            .click(region.offset(x), self.modifiers_shift, Instant::now());
        self.input_drag = true;
        self.input_engaged = true;
        self.store_text_input(field);
        true
    }

    pub(super) fn text_input_drag(&mut self) -> bool {
        if !self.input_drag {
            return false;
        }
        let (x, _) = self.mouse_virtual();
        if let Some(region) = self
            .input_regions
            .iter()
            .rev()
            .find(|r| Some(r.field) == self.input_focus)
        {
            self.text_input.place(region.offset(x), true);
        }
        true
    }
}
