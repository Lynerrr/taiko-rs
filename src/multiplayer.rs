//! Multiplayer session state and screens: host/join menu, lobby, in-match scoreboard, results.

use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use crate::game::{Game, Stats};
use crate::net::{self, Client, Final, Live, MapInfo, PlayerInfo, Server, Status};
use crate::online;
use crate::render::{Canvas, Hit, accent, rgba};
use winit::keyboard::KeyCode;

pub struct Session {
    pub client: Client,
    /// Present when we host the room.
    pub server: Option<Server>,
    pub players: Vec<PlayerInfo>,
    pub map: Option<MapInfo>,
    pub in_match: bool,
    pub results: Option<Vec<PlayerInfo>>,
    pub ready: bool,
    pub last_live: Instant,
    pub label: String,
    /// Last map hash we reported HasMap for, and whether we have it.
    pub reported_map: Option<u64>,
    pub have_map: bool,
    /// Our library entry for the lobby map.
    pub map_entry: Option<usize>,
    /// Presses of the current match already sent to the room.
    pub inputs_sent: usize,
    /// Lobby chat: (name, line), oldest first; and lines not seen yet.
    pub chat: Vec<(String, String)>,
    pub chat_unread: usize,
    /// A chat line being typed in the lobby (T or a click on its line), and its caret.
    pub chat_input: Option<String>,
    pub chat_caret: usize,
    /// The lobby's id on the online server (friends can join it from their list).
    pub online_room: Option<u32>,
    /// The room plays mapguessr; how many maps every player has (None: a list is missing).
    pub mapguessr: bool,
    pub common: Option<u32>,
    /// Defense: the team's lives, gold, shield and towers, as the room keeps them.
    pub team: Option<net::TeamState>,
    /// The lobby needs a password, and how many players it takes.
    pub locked: bool,
    pub slots: u32,
    /// Host: a new password being typed in the lobby (K).
    pub edit_password: Option<String>,
    /// In a match: we asked to skip the intro, and how many players have (of how many).
    pub skip_asked: bool,
    pub skip_votes: Option<(u32, u32)>,
}

impl Session {
    fn new(client: Client, server: Option<Server>, label: String) -> Session {
        Session {
            client,
            server,
            players: Vec::new(),
            map: None,
            in_match: false,
            results: None,
            ready: false,
            last_live: Instant::now(),
            label,
            reported_map: None,
            have_map: false,
            map_entry: None,
            inputs_sent: 0,
            chat: Vec::new(),
            chat_unread: 0,
            chat_input: None,
            chat_caret: 0,
            online_room: None,
            mapguessr: false,
            common: None,
            team: None,
            locked: false,
            slots: net::RoomSettings::MAX_SLOTS,
            edit_password: None,
            skip_asked: false,
            skip_votes: None,
        }
    }

    pub fn me(&self) -> Option<&PlayerInfo> {
        let id = self.client.id?;
        self.players.iter().find(|p| p.id == id)
    }

    /// Has the host role: picks maps and starts matches (it can be handed over).
    pub fn is_host(&self) -> bool {
        self.me().is_some_and(|p| p.host)
    }

    /// This machine runs the room (closing it ends it for everyone).
    pub fn owns_room(&self) -> bool {
        self.server.is_some()
    }
}

pub type Connecting = Receiver<Result<Session, String>>;

/// Opens a room on a port of this machine and joins it ourselves (`mapguessr`: one playing
/// mapguessr): the layout tests' lobbies (the menu only opens them on the server now).
#[cfg(test)]
pub fn start_host(port: u16, name: String, mapguessr: bool) -> Connecting {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let server = net::host(port, mapguessr)?;
            let client = Client::connect(&format!("127.0.0.1:{port}"), &name, Some(server.token))?;
            Ok(Session::new(
                client,
                Some(server),
                format!("hosting on port {}", port),
            ))
        })();
        let _ = tx.send(result);
    });
    rx
}

pub fn live_from(game: &Game) -> Live {
    let s = &game.stats;
    Live {
        score: s.score,
        combo: s.combo,
        accuracy: s.accuracy(),
        miss: s.miss,
        progress: game.progress(),
        great: s.great,
    }
}

/// Defense: the team for the match starting (everyone playing it, by id so every player has
/// it in the same order) with their names, and which of them we are.
pub fn defense_team(s: &Session) -> (Vec<(u32, String)>, usize) {
    let mut team: Vec<(u32, String)> = s
        .players
        .iter()
        .filter(|p| p.status != Status::Lobby && !p.spectator)
        .map(|p| (p.id, p.name.clone()))
        .collect();
    if let Some(id) = s.client.id
        && !team.iter().any(|p| p.0 == id)
    {
        team.push((id, s.me().map_or_else(String::new, |p| p.name.clone())));
    }
    team.sort_by_key(|p| p.0);
    let me = s
        .client
        .id
        .and_then(|id| team.iter().position(|p| p.0 == id))
        .unwrap_or(0);
    (team, me)
}

/// Defense, after the match: how the team did (the room's lives, else from everyone's misses).
fn defense_verdict(
    defense: net::Defense,
    team: Option<net::TeamState>,
    rows: &[PlayerInfo],
) -> (String, u32) {
    let leaked: u32 = rows
        .iter()
        .map(|p| p.result.as_ref().map_or(p.live.miss, |r| r.miss))
        .sum();
    let lives = team.map_or(defense.lives, |t| t.max);
    let left = team.map_or(lives.saturating_sub(leaked), |t| t.lives);
    if left == 0 {
        (
            tr!("THE TEAM FELL  ({leaked} notes got through)", "チーム敗北  ({leaked}ノーツ突破)", "КОМАНДА ПАЛА  (прорвалось {leaked})";),
            0xEB452C,
        )
    } else {
        (
            tr!("DEFENDED  ({left}/{lives} lives left)", "防衛成功  (ライフ {left}/{lives})", "ОБОРОНА УСТОЯЛА  ({left}/{lives} жизней)";),
            0x86E05A,
        )
    }
}

pub fn final_from(stats: &Stats, quit: bool) -> Final {
    Final {
        score: stats.score,
        accuracy: stats.accuracy(),
        max_combo: stats.max_combo,
        great: stats.great,
        ok: stats.ok,
        miss: stats.miss,
        ur: stats.unstable_rate(),
        quit,
    }
}

// ---------------------------------------------------------------------------------------------
// Host / join menu

/// The multiplayer menu's rows, top to bottom: an account (its forms), then once logged in the
/// server's lobbies and a new lobby's settings, or the password of a locked lobby being joined.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Row {
    User,
    Password,
    /// For a new account (where codes go), or an old one's to link.
    Email,
    /// The code mailed (`Menu::code_for`).
    Code,
    LogIn,
    /// On the login form it opens `Form::Register`; there it sends the code, then makes the
    /// account.
    Register,
    /// Forgot the password: on the login form it opens `Form::Reset`; there it sends a code to
    /// the account's email, then sets the new password.
    Forgot,
    /// Logged in: a new password, by a code to the account's email (`Form::Reset`).
    ChangePassword,
    /// Logged in without an email: link one (opens `Form::Link`, then sends the code and
    /// checks it).
    LinkEmail,
    /// Out of a form, back to logging in (or the lobbies).
    Back,
    /// Opens a lobby with the settings above it (mode, password, slots).
    HostOnline,
    /// The new lobby: a match or mapguessr, its password (empty: open to all), how many it takes.
    NewMode,
    NewPassword,
    NewSlots,
    Lobby(usize),
    LogOut,
    /// Joining a locked lobby: its password, then join or not.
    JoinPassword,
    JoinLocked,
    CancelJoin,
}

impl Row {
    /// Enter (or a click) does something; other rows are text fields.
    pub fn is_action(self) -> bool {
        !matches!(
            self,
            Row::User
                | Row::Password
                | Row::Email
                | Row::Code
                | Row::NewPassword
                | Row::JoinPassword
        )
    }
}

/// What the account side of the menu shows: the login form (or the lobbies, logged in), or one
/// of the steps around it, each on its own so no field means two things.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Form {
    #[default]
    Main,
    Register,
    /// A forgotten password, or (logged in) a changed one: both by a code to the email.
    Reset,
    Link,
}

/// What a mailed code is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeFor {
    Register,
    Reset,
    Link,
}

#[derive(Default)]
pub struct Menu {
    pub cursor: usize,
    /// The login form.
    pub user: String,
    pub password: String,
    pub email: String,
    pub code: String,
    /// A code was mailed for this (the Code row shows), and the one asked for, until answered.
    pub code_for: Option<CodeFor>,
    pub asking: Option<CodeFor>,
    /// Logged in as (a saved session).
    pub account: Option<String>,
    /// Whether the account has an email (None: the server doesn't say).
    pub has_email: Option<bool>,
    /// The form shown in place of the login (or the lobbies).
    pub form: Form,
    pub rooms: Vec<online::RoomInfo>,
    /// Whether the server answered the last request (None: not asked yet).
    pub online: Option<Result<(), String>>,
    pub account_request: Option<Receiver<Result<online::Reply, String>>>,
    pub rooms_request: Option<Receiver<Result<Vec<online::RoomInfo>, String>>>,
    pub last_refresh: Option<Instant>,
    /// The saved session was loaded from the config (and is being checked).
    pub session_loaded: bool,
    /// The lobby to open: mapguessr (or a match), its password, and how many players it takes
    /// (0: the most).
    pub new_guessr: bool,
    pub new_password: String,
    pub new_slots: u32,
    /// A locked lobby being joined, and the password typed for it.
    pub joining: Option<online::RoomInfo>,
    pub join_password: String,
}

impl Menu {
    /// The new lobby's player limit.
    pub fn slots(&self) -> u32 {
        if self.new_slots == 0 {
            net::RoomSettings::MAX_SLOTS
        } else {
            self.new_slots
        }
    }

    /// The new lobby's player limit stepped `dir` along the choices.
    pub fn step_slots(&mut self, dir: i32) {
        let list = net::RoomSettings::SLOTS;
        let at = list
            .iter()
            .position(|&n| n == self.slots())
            .unwrap_or(list.len() - 1) as i32;
        self.new_slots = list[(at + dir).clamp(0, list.len() as i32 - 1) as usize];
    }

    /// The new lobby's settings, to open it with.
    pub fn new_settings(&self) -> net::RoomSettings {
        let password = Some(self.new_password.clone()).filter(|p| !p.is_empty());
        net::RoomSettings {
            password,
            slots: self.slots(),
        }
        .clean()
    }
}

impl Menu {
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        let sent = |f| self.code_for == Some(f);
        match (self.account.is_some(), self.form) {
            (true, Form::Link) => {
                rows.push(Row::Email);
                if sent(CodeFor::Link) {
                    rows.push(Row::Code);
                }
                rows.extend([Row::LinkEmail, Row::Back]);
            }
            (logged_in, Form::Reset) => {
                if !logged_in {
                    rows.push(Row::User);
                }
                if sent(CodeFor::Reset) {
                    rows.extend([Row::Code, Row::Password]);
                }
                rows.extend([Row::Forgot, Row::Back]);
            }
            (false, Form::Register) => {
                rows.extend([Row::User, Row::Password, Row::Email]);
                if sent(CodeFor::Register) {
                    rows.push(Row::Code);
                }
                rows.extend([Row::Register, Row::Back]);
            }
            (true, _) if self.joining.is_some() => {
                rows.extend([Row::JoinPassword, Row::JoinLocked, Row::CancelJoin])
            }
            (true, _) => {
                rows.extend((0..self.rooms.len()).map(Row::Lobby));
                rows.extend([
                    Row::NewMode,
                    Row::NewPassword,
                    Row::NewSlots,
                    Row::HostOnline,
                ]);
                rows.extend([Row::LogOut, Row::ChangePassword]);
                if self.has_email == Some(false) {
                    rows.push(Row::LinkEmail);
                }
            }
            (false, _) => rows.extend([
                Row::User,
                Row::Password,
                Row::LogIn,
                Row::Register,
                Row::Forgot,
            ]),
        }
        rows
    }

    /// Shows `form`, from its start (what was typed for another one stays, but codes don't).
    pub fn open(&mut self, form: Form) {
        self.form = form;
        self.code_for = None;
        self.asking = None;
        self.code.clear();
        self.password.clear();
        self.cursor = 0;
    }

    /// A text field's label, as this form means it.
    pub fn field_label(&self, row: Row) -> Option<&'static str> {
        Some(match row {
            Row::User if self.form == Form::Register => tr!("Name", "名前", "Ник"),
            Row::User => tr!("Name or email", "名前かメール", "Ник или почта"),
            Row::Password if self.form == Form::Reset => {
                tr!("New password", "新しいパスワード", "Новый пароль")
            }
            Row::Password => tr!("Password", "パスワード", "Пароль"),
            Row::Email => tr!("Email", "メール", "Почта"),
            Row::Code => tr!("Code from the email", "メールのコード", "Код из письма"),
            _ => return None,
        })
    }

    /// Above the form: what it's for.
    fn title(&self) -> Option<&'static str> {
        Some(match self.form {
            Form::Register => tr!("NEW ACCOUNT", "新規アカウント", "НОВЫЙ АККАУНТ"),
            Form::Reset if self.account.is_some() => {
                tr!("CHANGE PASSWORD", "パスワード変更", "СМЕНА ПАРОЛЯ")
            }
            Form::Reset => tr!(
                "FORGOT PASSWORD",
                "パスワードを忘れた",
                "ВОССТАНОВЛЕНИЕ ПАРОЛЯ"
            ),
            Form::Link => tr!("LINK AN EMAIL", "メールを登録", "ПРИВЯЗКА ПОЧТЫ"),
            Form::Main => return None,
        })
    }

    /// Under the form: what to do, step by step.
    fn steps(&self) -> [&'static str; 2] {
        let sent = self.code_for.is_some();
        match self.form {
            Form::Register if sent => [
                tr!(
                    "The code is in your email (look in spam too).",
                    "コードはメールにあります(迷惑メールも確認)。",
                    "Код пришёл на почту (загляни и в спам)."
                ),
                tr!(
                    "Type it above, then CREATE ACCOUNT.",
                    "上に入力して「アカウント作成」。",
                    "Введи его выше и нажми СОЗДАТЬ АККАУНТ."
                ),
            ],
            Form::Register => [
                tr!(
                    "Pick a name and a password, type your email,",
                    "名前とパスワードを決め、メールを入力して",
                    "Придумай ник и пароль, впиши почту"
                ),
                tr!(
                    "then SEND CODE: it comes to that email.",
                    "「コードを送信」。そのメールに届きます。",
                    "и нажми ОТПРАВИТЬ КОД: он придёт туда."
                ),
            ],
            Form::Reset if sent => [
                tr!(
                    "The code is in your email (look in spam too).",
                    "コードはメールにあります(迷惑メールも確認)。",
                    "Код пришёл на почту (загляни и в спам)."
                ),
                tr!(
                    "Type it and a new password, then SET PASSWORD.",
                    "コードと新しいパスワードを入力して「設定」。",
                    "Введи его и новый пароль, затем СОХРАНИТЬ."
                ),
            ],
            Form::Reset if self.account.is_some() => [
                tr!(
                    "SEND CODE: it goes to your account's email.",
                    "「コードを送信」: アカウントのメールに届きます。",
                    "ОТПРАВИТЬ КОД: он придёт на почту аккаунта."
                ),
                tr!(
                    "Then you type it with the new password.",
                    "そのコードと新しいパスワードを入力します。",
                    "Потом вводишь его и новый пароль."
                ),
            ],
            Form::Reset => [
                tr!(
                    "Type your name or email, then SEND CODE:",
                    "名前かメールを入力して「コードを送信」:",
                    "Впиши ник или почту и нажми ОТПРАВИТЬ КОД:"
                ),
                tr!(
                    "it goes to the account's email.",
                    "アカウントのメールに届きます。",
                    "он придёт на почту аккаунта."
                ),
            ],
            Form::Link => [
                tr!(
                    "A code goes to the email: type it, then LINK.",
                    "メールにコードが届きます: 入力して「登録」。",
                    "На почту придёт код: введи его и нажми ПРИВЯЗАТЬ."
                ),
                tr!(
                    "With it, a lost password can be reset.",
                    "これでパスワードを忘れても再設定できます。",
                    "С ней можно восстановить забытый пароль."
                ),
            ],
            Form::Main => [
                tr!(
                    "An account plays online: lobbies, friends, leaderboards.",
                    "アカウントでオンライン: ロビー・フレンド・ランキング。",
                    "С аккаунтом играешь онлайн: лобби, друзья, рейтинги."
                ),
                tr!(
                    "No account yet? CREATE ACCOUNT takes a minute.",
                    "まだない? 「アカウント作成」はすぐです。",
                    "Нет аккаунта? РЕГИСТРАЦИЯ займёт минуту."
                ),
            ],
        }
    }

    pub fn row(&self) -> Row {
        let rows = self.rows();
        rows[self.cursor.min(rows.len() - 1)]
    }

    pub fn select(&mut self, row: Row) {
        if let Some(i) = self.rows().iter().position(|r| *r == row) {
            self.cursor = i;
        }
    }

    /// Asks the server for its lobbies (in the background).
    pub fn refresh(&mut self, server: &str) {
        if self.rooms_request.is_some() {
            return;
        }
        self.last_refresh = Some(Instant::now());
        let (tx, rx) = mpsc::channel();
        let server = server.to_string();
        std::thread::spawn(move || {
            let result = online::call(&server, online::Request::ListRooms).and_then(|r| match r {
                online::Reply::Rooms(rooms) => Ok(rooms),
                _ => Err(crate::lang::odd_reply()),
            });
            let _ = tx.send(result);
        });
        self.rooms_request = Some(rx);
    }

    /// Sends an account request (log in, register, log out, check a session) in the background.
    pub fn account_call(&mut self, server: &str, request: online::Request) {
        let (tx, rx) = mpsc::channel();
        let server = server.to_string();
        std::thread::spawn(move || {
            let _ = tx.send(online::call(&server, request));
        });
        self.account_request = Some(rx);
    }
}

/// Creates or joins a lobby on the online server; the connection then becomes the room's.
/// Opens or joins a lobby on the online server; `password` is the lobby's, when it has one.
pub fn start_online(
    server: String,
    request: online::Request,
    name: String,
    password: Option<String>,
) -> Connecting {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| match online::exchange(&server, request)? {
            (
                online::Reply::Joined {
                    room,
                    title,
                    host_token,
                },
                reader,
            ) => {
                let client = Client::over(reader, &name, host_token, password)?;
                let mut s = Session::new(client, None, format!("online: {title}"));
                s.online_room = Some(room);
                Ok(s)
            }
            (online::Reply::Error(e), _) => Err(e),
            _ => Err(crate::lang::odd_reply()),
        })();
        let _ = tx.send(result);
    });
    rx
}

/// Text shown for the server in the menu, without the default port.
fn server_label(server: &str) -> &str {
    server
        .strip_suffix(&format!(":{}", online::DEFAULT_PORT))
        .unwrap_or(server)
}

/// What a menu row shows: a text field (label left, its value or a placeholder right, a caret
/// when selected), a stepper (label left, `< value >` right) or an action (its label centred).
enum RowLook<'a> {
    Field(&'a str, bool),
    Stepper(&'a str),
    Action,
}

/// One row of the multiplayer menu, `h` tall.
#[allow(clippy::too_many_arguments)]
fn menu_row(
    c: &mut Canvas,
    sel: bool,
    i: usize,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    label: &str,
    look: RowLook,
) {
    let bg = if sel {
        0x34344a
    } else if c.hovered(x, y, w, h) {
        0x262634
    } else {
        0x1d1d27
    };
    c.round_rect([x, y], [w, h], 7.0, rgba(bg, 1.0), rgba(0x2a2a38, 1.0), 1.0);
    if sel {
        crate::style::selector(c, "multiplayer", [x - 3.0, y - 3.0, w + 6.0, h + 6.0]);
    }
    c.hit(x, y, w, h, Hit::MpRow(i));
    let ty = y + h / 2.0 - 4.5;
    match look {
        RowLook::Field(value, placeholder) => {
            c.text(x + 12.0, ty, 9.0, rgba(0x9a9ab0, 1.0), label);
            // The value keeps clear of the label: a long one shows its end (where you type).
            let room = w - 34.0 - Canvas::text_width(9.0, label);
            let shown = if sel {
                format!("{value}_")
            } else {
                value.to_string()
            };
            let shown = if placeholder {
                Canvas::fit(9.0, room, &shown)
            } else {
                Canvas::fit_end(9.0, room, &shown)
            };
            let colour = if placeholder && !sel {
                0x55556a
            } else {
                0xffffff
            };
            c.text_right(x + w - 12.0, ty, 9.0, rgba(colour, 1.0), &shown);
        }
        RowLook::Stepper(value) => {
            c.text(x + 12.0, ty, 9.0, rgba(0x9a9ab0, 1.0), label);
            let arrows = if sel { 0xffffff } else { 0x55556a };
            let vw = Canvas::text_width(9.0, value);
            c.text_right(x + w - 12.0, ty, 9.0, rgba(arrows, 1.0), ">");
            c.text_right(x + w - 26.0, ty, 9.0, rgba(0xffffff, 1.0), value);
            c.text_right(x + w - 34.0 - vw, ty, 9.0, rgba(arrows, 1.0), "<");
        }
        RowLook::Action => {
            let size = Canvas::fit_size(11.0, 7.0, w - 16.0, label);
            c.text_centered(
                x + w / 2.0,
                y + h / 2.0 - size / 2.0,
                size,
                rgba(if sel { 0xffffff } else { 0xc8c8d8 }, 1.0),
                label,
            );
        }
    }
}

/// A card of the menu, the song select's look: dark, a thin border.
fn card(c: &mut Canvas, x: f32, y: f32, w: f32, h: f32) {
    c.round_rect(
        [x, y],
        [w, h],
        8.0,
        rgba(0x15151d, 0.86),
        rgba(0x2a2a38, 1.0),
        1.0,
    );
}

/// A small padlock (a locked lobby), `size` tall at (x, y).
fn padlock(c: &mut Canvas, x: f32, y: f32, size: f32, colour: u32) {
    let k = size / 10.0;
    c.ring(
        x + 4.5 * k,
        y + 4.0 * k,
        2.8 * k,
        rgba(colour, 1.0),
        1.4 * k,
    );
    c.round_rect(
        [x, y + 4.0 * k],
        [9.0 * k, 6.0 * k],
        1.5 * k,
        rgba(colour, 1.0),
        rgba(0, 0.0),
        0.0,
    );
}

/// The multiplayer menu. Logged out, the account's card (log in, or a form to make one, get a
/// lost password back...). Logged in, the server's lobbies on the left (a padlock on the ones
/// with a password, how full they are) and the account and a new lobby's settings on the
/// right; joining a locked lobby asks for its password first.
pub fn draw_menu(c: &mut Canvas, menu: &Menu, server: &str, status: &str, busy: bool) {
    let w = c.width;
    let state = match &menu.online {
        Some(Ok(())) => tr!("online", "オンライン", "онлайн"),
        Some(Err(_)) => tr!("no connection", "接続なし", "нет связи"),
        None => tr!("connecting...", "接続中...", "подключение..."),
    };
    crate::settings::tab_bar(c, 2, &format!("{}   {state}", server_label(server)));
    let rows = menu.rows();
    let cursor = menu.cursor.min(rows.len() - 1);
    let in_lobbies = menu.account.is_some() && menu.form == Form::Main;
    match (in_lobbies, &menu.joining) {
        (true, Some(room)) => draw_join_prompt(c, menu, room, &rows, cursor),
        (true, None) => draw_lobbies(c, menu, &rows, cursor),
        (false, _) => draw_account(c, menu, &rows, cursor),
    }

    let room = w - 48.0;
    let msg = if busy { "connecting..." } else { status };
    let size = Canvas::fit_size(9.0, 6.5, room, msg);
    c.text_centered(
        w / 2.0,
        440.0 + (9.0 - size) / 2.0,
        size,
        rgba(0xFFD34D, 1.0),
        &Canvas::fit(size, room, msg),
    );
    let hints = match (in_lobbies, menu.joining.is_some()) {
        (true, true) => "type the password   Enter join   Esc cancel",
        (true, false) => {
            "Up/Down select   Left/Right lobbies or new lobby   Enter join / change   F5 refresh   Tab or Esc back"
        }
        _ => "Up/Down select   type to edit   Enter   Tab or Esc back",
    };
    c.text_centered(
        w / 2.0,
        462.0,
        8.0,
        rgba(0x55556a, 1.0),
        &Canvas::fit_hints(8.0, room, hints),
    );
}

/// Logged out (or in one of the account's forms): its card, in the middle.
fn draw_account(c: &mut Canvas, menu: &Menu, rows: &[Row], cursor: usize) {
    let w = c.width;
    let col = (w - 48.0).min(420.0);
    let x = (w - col) / 2.0;
    let (top, bottom) = (52.0, 428.0);
    card(c, x - 14.0, top, col + 28.0, bottom - top);
    let heading = menu.title().unwrap_or(tr!(
        "ONLINE ACCOUNT",
        "オンラインアカウント",
        "ОНЛАЙН-АККАУНТ"
    ));
    c.text(x, top + 12.0, 9.0, rgba(accent(), 1.0), heading);
    let mut y = top + 32.0;
    let steps = |c: &mut Canvas, y: f32| {
        for (k, line) in menu.steps().iter().enumerate() {
            let size = Canvas::fit_size(7.0, 6.0, col, line);
            c.text_centered(
                x + col / 2.0,
                y + 4.0 + 14.0 * k as f32,
                size,
                rgba(0x8a8aa0, 1.0),
                &Canvas::fit(size, col, line),
            );
        }
    };
    for (i, r) in rows.iter().enumerate() {
        let sel = i == cursor;
        let label = menu.field_label(*r).unwrap_or("");
        let field = |value: &str, hint: &'static str| -> (String, bool) {
            if value.is_empty() {
                (hint.to_string(), true)
            } else {
                (value.to_string(), false)
            }
        };
        match *r {
            Row::User => {
                let hint = if menu.form == Form::Register {
                    online::NAME_RULES
                } else {
                    ""
                };
                let (v, ph) = field(&menu.user, hint);
                menu_row(c, sel, i, x, y, col, 30.0, label, RowLook::Field(&v, ph));
                y += 38.0;
            }
            Row::Password => {
                let stars = "*".repeat(menu.password.chars().count());
                let hint = if menu.form == Form::Main {
                    ""
                } else {
                    tr!("6 characters or more", "6文字以上", "от 6 символов")
                };
                let (v, ph) = field(&stars, hint);
                menu_row(c, sel, i, x, y, col, 30.0, label, RowLook::Field(&v, ph));
                y += 38.0;
            }
            Row::Email => {
                let hint = if menu.form == Form::Register {
                    tr!("the code comes here", "コードの送り先", "сюда придёт код")
                } else {
                    tr!("your email", "あなたのメール", "твоя почта")
                };
                let (v, ph) = field(&menu.email, hint);
                menu_row(c, sel, i, x, y, col, 30.0, label, RowLook::Field(&v, ph));
                y += 38.0;
            }
            Row::Code => {
                let (v, ph) = field(&menu.code, tr!("6 digits", "6桁", "6 цифр"));
                menu_row(c, sel, i, x, y, col, 30.0, label, RowLook::Field(&v, ph));
                y += 38.0;
            }
            Row::LogIn => {
                menu_row(
                    c,
                    sel,
                    i,
                    x,
                    y,
                    col,
                    30.0,
                    tr!("LOG IN", "ログイン", "ВОЙТИ"),
                    RowLook::Action,
                );
                y += 38.0;
            }
            // On the login form, the ways to its other forms, side by side.
            Row::Register if menu.form == Form::Main => {
                let half = (col - 8.0) / 2.0;
                half_row(
                    c,
                    sel,
                    i,
                    x,
                    y,
                    half,
                    tr!("CREATE ACCOUNT", "アカウント作成", "РЕГИСТРАЦИЯ"),
                );
                half_row(
                    c,
                    i + 1 == cursor,
                    i + 1,
                    x + half + 8.0,
                    y,
                    half,
                    tr!("FORGOT PASSWORD?", "パスワードを忘れた?", "ЗАБЫЛ ПАРОЛЬ?"),
                );
                y += 34.0;
                steps(c, y);
                y += 34.0;
            }
            Row::Forgot if menu.form == Form::Main => {}
            Row::Register => {
                let label = if menu.code_for == Some(CodeFor::Register) {
                    tr!("CREATE ACCOUNT", "アカウント作成", "СОЗДАТЬ АККАУНТ")
                } else {
                    tr!("SEND CODE", "コードを送信", "ОТПРАВИТЬ КОД")
                };
                menu_row(c, sel, i, x, y, col, 30.0, label, RowLook::Action);
                y += 38.0;
            }
            Row::Forgot => {
                let label = if menu.code_for == Some(CodeFor::Reset) {
                    tr!("SET PASSWORD", "設定", "СОХРАНИТЬ")
                } else {
                    tr!("SEND CODE", "コードを送信", "ОТПРАВИТЬ КОД")
                };
                menu_row(c, sel, i, x, y, col, 30.0, label, RowLook::Action);
                y += 38.0;
            }
            Row::LinkEmail => {
                let label = if menu.code_for == Some(CodeFor::Link) {
                    tr!("LINK", "登録", "ПРИВЯЗАТЬ")
                } else {
                    tr!("SEND CODE", "コードを送信", "ОТПРАВИТЬ КОД")
                };
                menu_row(c, sel, i, x, y, col, 30.0, label, RowLook::Action);
                y += 38.0;
            }
            Row::Back => {
                menu_row(
                    c,
                    sel,
                    i,
                    x,
                    y,
                    col,
                    24.0,
                    tr!("BACK", "戻る", "НАЗАД"),
                    RowLook::Action,
                );
                y += 32.0;
                steps(c, y);
            }
            _ => {}
        }
    }
}

/// Logged in: the lobbies (left), the account and a new lobby (right).
fn draw_lobbies(c: &mut Canvas, menu: &Menu, rows: &[Row], cursor: usize) {
    let w = c.width;
    let (top, bottom) = (52.0, 428.0);
    let lx = 24.0;
    let lw = ((w - 64.0) * 0.6).max(280.0);
    let rx = lx + lw + 16.0;
    let rw = w - 24.0 - rx;

    // The lobbies.
    card(c, lx, top, lw, bottom - top);
    c.text(
        lx + 14.0,
        top + 12.0,
        9.0,
        rgba(accent(), 1.0),
        tr!("LOBBIES", "ロビー", "ЛОББИ"),
    );
    let count = menu.rooms.len();
    c.text_right(
        lx + lw - 14.0,
        top + 13.0,
        7.0,
        rgba(0x6f6f86, 1.0),
        &tr!("{count} open   F5 refresh", "{count}件   F5 更新", "открыто: {count}   F5 обновить";),
    );
    let lobby_rows: Vec<(usize, usize)> = rows
        .iter()
        .enumerate()
        .filter_map(|(i, r)| {
            if let Row::Lobby(k) = r {
                Some((i, *k))
            } else {
                None
            }
        })
        .collect();
    const ROW_H: f32 = 40.0;
    let list_top = top + 32.0;
    let shown = ((bottom - list_top - 6.0) / ROW_H).floor().max(1.0) as usize;
    let first = lobby_rows
        .iter()
        .position(|(i, _)| *i == cursor)
        .map(|p| p.saturating_sub(shown - 1))
        .unwrap_or(0);
    if lobby_rows.is_empty() {
        let msg = match &menu.online {
            Some(Err(e)) => e.clone(),
            _ => tr!(
                "Nobody is hosting right now: open a lobby on the right.",
                "今はロビーがありません: 右で作ろう。",
                "Сейчас никто не хостит: создай лобби справа."
            )
            .to_string(),
        };
        c.text_centered(
            lx + lw / 2.0,
            list_top + 60.0,
            8.0,
            rgba(0x6f6f86, 1.0),
            &Canvas::fit(8.0, lw - 28.0, &msg),
        );
    }
    for (n, &(i, k)) in lobby_rows.iter().enumerate().skip(first).take(shown) {
        let Some(r) = menu.rooms.get(k) else { continue };
        let (x, y, rw_) = (lx + 8.0, list_top + (n - first) as f32 * ROW_H, lw - 16.0);
        let sel = i == cursor;
        let bg = if sel {
            0x34344a
        } else if c.hovered(x, y, rw_, ROW_H - 4.0) {
            0x262634
        } else {
            0x1d1d27
        };
        c.round_rect(
            [x, y],
            [rw_, ROW_H - 4.0],
            7.0,
            rgba(bg, 1.0),
            rgba(0x2a2a38, 1.0),
            1.0,
        );
        if sel {
            crate::style::selector(c, "multiplayer", [x - 3.0, y - 3.0, rw_ + 6.0, ROW_H + 2.0]);
        }
        c.hit(x, y, rw_, ROW_H - 4.0, Hit::MpRow(i));
        // Right: how full it is, and whether a match is on.
        let full = r.players >= r.slots;
        let players = format!("{}/{}", r.players, r.slots);
        c.text_right(
            x + rw_ - 12.0,
            y + 7.0,
            9.0,
            rgba(if full { 0xFF4B4B } else { 0xffffff }, 1.0),
            &players,
        );
        let (state, colour) = if r.in_match {
            (tr!("in a match", "対戦中", "идёт матч"), 0x438EAD)
        } else {
            (tr!("open", "待機中", "ждут"), 0x86E05A)
        };
        c.text_right(x + rw_ - 12.0, y + 21.0, 7.0, rgba(colour, 1.0), state);
        let right = Canvas::text_width(9.0, &players).max(Canvas::text_width(7.0, state)) + 24.0;
        // Left: the padlock, the title, then the host, the mode and the map.
        let mut tx = x + 12.0;
        if r.locked {
            padlock(c, tx, y + 6.0, 10.0, 0xFFD34D);
            tx += 14.0;
        }
        c.text(
            tx,
            y + 7.0,
            9.0,
            rgba(0xffffff, 1.0),
            &Canvas::fit(9.0, x + rw_ - right - tx, &r.title),
        );
        let mut info = format!("{} {}", tr!("host", "ホスト", "хост"), r.host);
        if r.mapguessr {
            info.push_str("   MAPGUESSR");
        }
        if r.defense {
            info.push_str(tr!(
                "   DEFENSE co-op",
                "   ディフェンス協力",
                "   ОБОРОНА кооп"
            ));
        }
        if let Some(m) = &r.map {
            info.push_str("   ");
            info.push_str(m);
        }
        c.text(
            x + 12.0,
            y + 22.0,
            6.5,
            rgba(0x8a8aa0, 1.0),
            &Canvas::fit(6.5, rw_ - right - 12.0, &info),
        );
    }
    if lobby_rows.len() > shown {
        let more = lobby_rows.len() - shown;
        c.text_centered(
            lx + lw / 2.0,
            bottom - 12.0,
            6.5,
            rgba(0x6f6f86, 1.0),
            &tr!("{more} more (Up/Down)", "あと{more}件 (上下)", "ещё {more} (вверх/вниз)";),
        );
    }

    // The account.
    let has_link = rows.contains(&Row::LinkEmail);
    let account_h = if has_link { 108.0 } else { 80.0 };
    card(c, rx, top, rw, account_h);
    c.text(
        rx + 14.0,
        top + 12.0,
        9.0,
        rgba(accent(), 1.0),
        tr!("ACCOUNT", "アカウント", "АККАУНТ"),
    );
    let who = menu.account.as_deref().unwrap_or("");
    c.text(
        rx + 14.0,
        top + 27.0,
        8.0,
        rgba(0xc8c8d8, 1.0),
        &Canvas::fit(
            8.0,
            rw - 28.0,
            &tr!("logged in as {who}", "{who} でログイン中", "ты вошёл как {who}";),
        ),
    );
    let half = (rw - 28.0 - 8.0) / 2.0;
    let mut y = top + 44.0;
    for (i, r) in rows.iter().enumerate() {
        match *r {
            Row::LogOut => {
                half_row(
                    c,
                    i == cursor,
                    i,
                    rx + 14.0,
                    y,
                    half,
                    tr!("LOG OUT", "ログアウト", "ВЫЙТИ"),
                );
                half_row(
                    c,
                    i + 1 == cursor,
                    i + 1,
                    rx + 22.0 + half,
                    y,
                    half,
                    tr!("CHANGE PASSWORD", "パスワード変更", "СМЕНИТЬ ПАРОЛЬ"),
                );
                y += 30.0;
            }
            Row::LinkEmail => {
                half_row(
                    c,
                    i == cursor,
                    i,
                    rx + 14.0,
                    y,
                    rw - 28.0,
                    tr!("LINK AN EMAIL", "メールを登録", "ПРИВЯЗАТЬ ПОЧТУ"),
                );
            }
            _ => {}
        }
    }

    // A new lobby.
    let ny = top + account_h + 12.0;
    card(c, rx, ny, rw, bottom - ny);
    c.text(
        rx + 14.0,
        ny + 12.0,
        9.0,
        rgba(accent(), 1.0),
        tr!("NEW LOBBY", "ロビーを作る", "НОВОЕ ЛОББИ"),
    );
    let (fx, fw) = (rx + 14.0, rw - 28.0);
    let mut y = ny + 32.0;
    for (i, r) in rows.iter().enumerate() {
        let sel = i == cursor;
        match *r {
            Row::NewMode => {
                let mode = if menu.new_guessr {
                    "Mapguessr"
                } else {
                    tr!("Match", "対戦", "Матч")
                };
                menu_row(
                    c,
                    sel,
                    i,
                    fx,
                    y,
                    fw,
                    28.0,
                    tr!("Mode", "モード", "Режим"),
                    RowLook::Stepper(mode),
                );
                y += 34.0;
            }
            Row::NewPassword => {
                let stars = "*".repeat(menu.new_password.chars().count());
                let (v, ph) = if stars.is_empty() {
                    (
                        tr!("none: open to all", "なし: 誰でも", "нет: для всех").to_string(),
                        true,
                    )
                } else {
                    (stars, false)
                };
                menu_row(
                    c,
                    sel,
                    i,
                    fx,
                    y,
                    fw,
                    28.0,
                    tr!("Password", "パスワード", "Пароль"),
                    RowLook::Field(&v, ph),
                );
                y += 34.0;
            }
            Row::NewSlots => {
                let slots = menu.slots().to_string();
                menu_row(
                    c,
                    sel,
                    i,
                    fx,
                    y,
                    fw,
                    28.0,
                    tr!("Players", "人数", "Игроков"),
                    RowLook::Stepper(&slots),
                );
                y += 38.0;
            }
            Row::HostOnline => {
                menu_row(
                    c,
                    sel,
                    i,
                    fx,
                    y,
                    fw,
                    32.0,
                    tr!("OPEN LOBBY", "ロビーを開く", "ОТКРЫТЬ ЛОББИ"),
                    RowLook::Action,
                );
                y += 40.0;
            }
            _ => {}
        }
    }
    let note = if menu.new_guessr {
        tr!(
            "Mapguessr: guess the song from a few seconds of it.",
            "マップ当て: 数秒で曲を当てよう。",
            "Угадайка: узнай песню по паре секунд."
        )
    } else {
        tr!(
            "The host picks the maps; the rest can download them.",
            "曲はホストが選び、他の人はダウンロードできます。",
            "Карты выбирает хост, остальные могут их скачать."
        )
    };
    let size = Canvas::fit_size(7.0, 6.0, fw, note);
    c.text_centered(
        rx + rw / 2.0,
        y,
        size,
        rgba(0x6f6f86, 1.0),
        &Canvas::fit(size, fw, note),
    );
}

/// Joining a locked lobby: its password, over the darkened menu.
fn draw_join_prompt(
    c: &mut Canvas,
    menu: &Menu,
    room: &online::RoomInfo,
    rows: &[Row],
    cursor: usize,
) {
    let w = c.width;
    let cw = (w - 48.0).min(380.0);
    let (x, y) = ((w - cw) / 2.0, 120.0);
    card(c, x, y, cw, 176.0);
    padlock(c, x + 16.0, y + 13.0, 11.0, 0xFFD34D);
    c.text(
        x + 34.0,
        y + 14.0,
        9.0,
        rgba(accent(), 1.0),
        &Canvas::fit(9.0, cw - 50.0, &room.title),
    );
    let info = format!(
        "{} {}   {}/{}",
        tr!("host", "ホスト", "хост"),
        room.host,
        room.players,
        room.slots
    );
    c.text(
        x + 16.0,
        y + 32.0,
        7.0,
        rgba(0x8a8aa0, 1.0),
        &Canvas::fit(7.0, cw - 32.0, &info),
    );
    c.text(
        x + 16.0,
        y + 48.0,
        8.0,
        rgba(0xc8c8d8, 1.0),
        tr!(
            "This lobby needs a password.",
            "このロビーはパスワードが必要です。",
            "В это лобби нужен пароль."
        ),
    );
    let (fx, fw) = (x + 16.0, cw - 32.0);
    let half = (fw - 8.0) / 2.0;
    for (i, r) in rows.iter().enumerate() {
        let sel = i == cursor;
        match *r {
            Row::JoinPassword => {
                let stars = "*".repeat(menu.join_password.chars().count());
                let ph = stars.is_empty();
                menu_row(
                    c,
                    sel,
                    i,
                    fx,
                    y + 66.0,
                    fw,
                    30.0,
                    tr!("Password", "パスワード", "Пароль"),
                    RowLook::Field(&stars, ph),
                );
            }
            Row::JoinLocked => {
                half_row(c, sel, i, fx, y + 108.0, half, tr!("JOIN", "参加", "ВОЙТИ"))
            }
            Row::CancelJoin => half_row(
                c,
                sel,
                i,
                fx + half + 8.0,
                y + 108.0,
                half,
                tr!("CANCEL", "キャンセル", "ОТМЕНА"),
            ),
            _ => {}
        }
    }
}

/// An action row narrower than a column (two side by side), 24 tall.
fn half_row(c: &mut Canvas, sel: bool, i: usize, x: f32, y: f32, w: f32, label: &str) {
    let bg = if sel {
        0x34344a
    } else if c.hovered(x, y, w, 24.0) {
        0x262634
    } else {
        0x1d1d27
    };
    c.round_rect(
        [x, y],
        [w, 24.0],
        7.0,
        rgba(bg, 1.0),
        rgba(0x2a2a38, 1.0),
        1.0,
    );
    if sel {
        crate::style::selector(c, "multiplayer", [x - 3.0, y - 3.0, w + 6.0, 30.0]);
    }
    c.hit(x, y, w, 24.0, Hit::MpRow(i));
    let size = Canvas::fit_size(11.0, 7.0, w - 12.0, label);
    c.text_centered(
        x + w / 2.0,
        y + 12.0 - size / 2.0,
        size,
        rgba(if sel { 0xffffff } else { 0xc8c8d8 }, 1.0),
        &Canvas::fit(size, w - 12.0, label),
    );
}

// ---------------------------------------------------------------------------------------------
// Lobby

/// `map_chosen`: "no map" only means something once the host has picked one.
fn status_text(p: &PlayerInfo, map_chosen: bool) -> (&'static str, u32) {
    match p.status {
        Status::Lobby if p.spectator => ("spectator", 0x4fb4ff),
        Status::Loading => ("loading", 0x9a9ab0),
        Status::Playing => ("playing", 0x438EAD),
        Status::Finished => ("finished", 0x9a9ab0),
        Status::Lobby if map_chosen && !p.has_map => ("no map", 0xFF4B4B),
        Status::Lobby if p.ready => ("ready", 0x86E05A),
        Status::Lobby => ("not ready", 0x8a8aa0),
    }
}

/// The lobby. Left, the map the host picked (song select's look: title, artist, mapper, the
/// difficulty with its stars, length, BPM, OD and HP at the match rate) and the players (two
/// columns, up to 16; the host can hand the role over or take a player out). Right, the room's
/// settings (password, players, rate, Defense and its rules: the host's to change, a click or
/// a key each) and the chat. Under them, what can be done.
/// `download`: progress of fetching the lobby map, when it's being downloaded.
pub fn draw_lobby(c: &mut Canvas, s: &Session, have_map: bool, download: Option<String>) {
    let w = c.width;
    let right = format!("{}   ping {:.0} ms", s.label, s.client.rtt_ms);
    crate::settings::tab_bar(c, 2, &right);
    let (top, bottom) = (52.0, 418.0);
    let lx = 24.0;
    let lw = ((w - 64.0) * 0.6).max(300.0);
    let rx = lx + lw + 16.0;
    let rw = w - 24.0 - rx;
    let host = s.is_host() && !s.in_match;

    // The map.
    let map_h = 102.0;
    let can_preview = s.map.is_some() && have_map;
    let hover = can_preview && c.hovered(lx, top, lw, map_h);
    c.round_rect(
        [lx, top],
        [lw, map_h],
        8.0,
        rgba(if hover { 0x1c1c27 } else { 0x15151d }, 0.86),
        rgba(0x2a2a38, 1.0),
        1.0,
    );
    if can_preview {
        c.hit(lx, top, lw, map_h, Hit::Key(KeyCode::KeyP));
    }
    let (ix, iw) = (lx + 14.0, lw - 28.0);
    match &s.map {
        Some(m) => {
            let stats = m.stats.as_ref();
            let stars = stats.and_then(|st| (st.stars >= 0.0).then_some(st.stars));
            let pill = crate::stars::draw_pill(c, lx + lw - 14.0, top + 12.0, 9.0, stars);
            c.text(
                ix,
                top + 11.0,
                11.0,
                rgba(0xffffff, 1.0),
                &Canvas::fit(11.0, iw - pill - 10.0, &m.title),
            );
            c.text(
                ix,
                top + 28.0,
                8.0,
                rgba(0xc8c8d8, 1.0),
                &Canvas::fit(8.0, iw, &m.artist),
            );
            let mapper = stats.map_or(String::new(), |st| format!("   mapped by {}", st.creator));
            c.text(
                ix,
                top + 42.0,
                8.0,
                rgba(accent(), 1.0),
                &Canvas::fit(8.0, iw, &format!("[{}]{mapper}", m.version)),
            );
            // At the match rate.
            let rate = m.rate.max(0.01) as f32;
            let mut line = Vec::new();
            if let Some(st) = stats {
                if st.length_ms >= 0.0 {
                    let secs = (st.length_ms / rate / 1000.0).round() as u32;
                    line.push(format!("Length {}:{:02}", secs / 60, secs % 60));
                }
                if st.bpm > 0.0 {
                    line.push(format!("BPM {:.0}", st.bpm * rate));
                }
                line.push(format!("OD {:.1}", st.od));
                line.push(format!("HP {:.1}", st.hp));
            }
            if (m.rate - 1.0).abs() > 1e-6 {
                line.push(format!("{:.2}x", m.rate));
            }
            c.text(
                ix,
                top + 58.0,
                8.0,
                rgba(0xffffff, 1.0),
                &Canvas::fit(8.0, iw, &line.join("    ")),
            );
            // Mods and where it starts.
            let mut extra = Vec::new();
            if !m.mods.is_empty() {
                extra.push(format!("+{}", m.mods.concat()));
            }
            if let Some(t) = m.from {
                let (min, sec) = ((t / 1000.0) as u64 / 60, (t / 1000.0) as u64 % 60);
                extra.push(tr!("from {min}:{sec:02}", "{min}:{sec:02}から", "с {min}:{sec:02}";));
            }
            if m.defense.is_some() {
                extra.push(tr!("DEFENSE co-op", "ディフェンス協力", "ОБОРОНА кооп").to_string());
            }
            c.text(
                ix,
                top + 73.0,
                7.0,
                rgba(0x9a9ab0, 1.0),
                &Canvas::fit(7.0, iw, &extra.join("    ")),
            );
            // How it is for us: being fetched, missing, or there to preview.
            let (note, colour) = match (&download, have_map) {
                (Some(d), _) => (d.clone(), 0xFFD34D),
                (None, false) => (
                    tr!(
                        "not in your Songs folder - D: from the host   M: the mirrors",
                        "Songsにありません - D: ホストから   M: ミラーから",
                        "нет в папке Songs - D: от хоста   M: с зеркал"
                    )
                    .to_string(),
                    0xFF4B4B,
                ),
                (None, true) => (
                    tr!(
                        "click to preview (P)",
                        "クリックでプレビュー (P)",
                        "клик: превью (P)"
                    )
                    .to_string(),
                    0x6f6f86,
                ),
            };
            c.text(
                ix,
                top + 87.0,
                7.0,
                rgba(colour, 1.0),
                &Canvas::fit(7.0, iw, &note),
            );
        }
        None => {
            let t = if s.is_host() {
                tr!(
                    "Press M to pick a map",
                    "Mで曲を選ぶ",
                    "Нажми M, чтобы выбрать карту"
                )
            } else {
                tr!(
                    "Waiting for the host to pick a map",
                    "ホストの選曲待ち",
                    "Ждём, пока хост выберет карту"
                )
            };
            c.text_centered(
                lx + lw / 2.0,
                top + map_h / 2.0 - 5.0,
                10.0,
                rgba(0x8a8aa0, 1.0),
                t,
            );
        }
    }

    // The players, in two columns.
    let py = top + map_h + 8.0;
    let ph = bottom - py;
    c.round_rect(
        [lx, py],
        [lw, ph],
        8.0,
        rgba(0x15151d, 0.86),
        rgba(0x2a2a38, 1.0),
        1.0,
    );
    let (count, slots) = (s.players.len(), s.slots);
    c.text(
        ix,
        py + 10.0,
        9.0,
        rgba(accent(), 1.0),
        &tr!("PLAYERS  {count}/{slots}", "プレイヤー  {count}/{slots}", "ИГРОКИ  {count}/{slots}";),
    );
    let my_id = s.client.id;
    let cell_w = (iw - 8.0) / 2.0;
    let cell_h = 26.0;
    let per_col = (((ph - 32.0) / (cell_h + 4.0)).floor() as usize).max(1);
    for (i, p) in s.players.iter().enumerate().take(per_col * 2) {
        let (x, y) = (
            ix + (i / per_col) as f32 * (cell_w + 8.0),
            py + 28.0 + (i % per_col) as f32 * (cell_h + 4.0),
        );
        let me = Some(p.id) == my_id;
        c.round_rect(
            [x, y],
            [cell_w, cell_h],
            6.0,
            rgba(if me { 0x2b2b3d } else { 0x1d1d27 }, 1.0),
            rgba(0x2a2a38, 1.0),
            1.0,
        );
        // Right: the host's buttons (hand the role over, take them out), or the status.
        let mut name_end = x + cell_w - 8.0;
        if host && !me {
            for (label, hit) in [("kick", Hit::Kick(p.id)), ("host", Hit::GiveHost(p.id))] {
                let bw = Canvas::text_width(6.5, label) + 10.0;
                name_end -= bw;
                c.button(name_end, y + 6.0, bw, 14.0, label, 6.5, hit, false);
                name_end -= 6.0;
            }
        } else {
            let (st, colour) = status_text(p, s.map.is_some());
            c.text_right(name_end, y + 5.0, 7.0, rgba(colour, 1.0), st);
            name_end -= Canvas::text_width(7.0, st) + 8.0;
        }
        let mut name = p.name.clone();
        if p.host {
            name.push_str(tr!("  (host)", "  (ホスト)", "  (хост)"));
        }
        c.text(
            x + 8.0,
            y + 5.0,
            8.0,
            rgba(0xffffff, 1.0),
            &Canvas::fit(8.0, name_end - x - 12.0, &name),
        );
        let mods = if p.mods.is_empty() {
            String::new()
        } else {
            format!("+{}", p.mods.concat())
        };
        let under = if host && !me {
            let (st, _) = status_text(p, s.map.is_some());
            format!("{st}   {mods}")
        } else {
            mods
        };
        c.text(
            x + 8.0,
            y + 16.0,
            6.0,
            rgba(0x8a8aa0, 1.0),
            &Canvas::fit(6.0, name_end - x - 12.0, &under),
        );
    }

    // The room's settings.
    let defense = s.map.as_ref().and_then(|m| m.defense);
    let mut settings: Vec<(String, String, Option<KeyCode>)> = Vec::new();
    let password = match &s.edit_password {
        Some(typed) => format!("{typed}_"),
        None if s.locked => tr!("set", "あり", "есть").to_string(),
        None => tr!("none: open to all", "なし: 誰でも", "нет: для всех").to_string(),
    };
    settings.push((
        tr!("Password", "パスワード", "Пароль").into(),
        password,
        Some(KeyCode::KeyK),
    ));
    settings.push((
        tr!("Players", "人数", "Игроков").into(),
        s.slots.to_string(),
        Some(KeyCode::KeyL),
    ));
    if let Some(m) = &s.map {
        settings.push((
            tr!("Match rate", "速度", "Скорость").into(),
            format!("{:.2}x", m.rate),
            Some(KeyCode::BracketRight),
        ));
    }
    let mode = if defense.is_some() {
        tr!("Defense co-op", "ディフェンス協力", "Оборона (кооп)")
    } else {
        tr!("Match", "対戦", "Матч")
    };
    settings.push((
        tr!("Mode", "モード", "Режим").into(),
        mode.into(),
        Some(KeyCode::KeyC),
    ));
    if let Some(d) = defense {
        let keys = [
            KeyCode::Digit1,
            KeyCode::Digit2,
            KeyCode::Digit3,
            KeyCode::Digit4,
            KeyCode::Digit5,
        ];
        for (o, key) in net::DefenseOption::ALL.into_iter().zip(keys) {
            settings.push((format!("  {}", o.name()), o.value(&d), Some(key)));
        }
    }
    let row_h = 18.0;
    let room_h = 30.0 + settings.len() as f32 * row_h + 8.0;
    c.round_rect(
        [rx, top],
        [rw, room_h],
        8.0,
        rgba(0x15151d, 0.86),
        rgba(0x2a2a38, 1.0),
        1.0,
    );
    c.text(
        rx + 14.0,
        top + 10.0,
        9.0,
        rgba(accent(), 1.0),
        tr!("ROOM", "ルーム", "КОМНАТА"),
    );
    if s.locked {
        padlock(c, rx + rw - 24.0, top + 9.0, 10.0, 0xFFD34D);
    }
    let key_name = |k: KeyCode| match k {
        KeyCode::KeyK => "K",
        KeyCode::KeyL => "L",
        KeyCode::KeyC => "C",
        KeyCode::BracketRight => "[ ]",
        KeyCode::Digit1 => "1",
        KeyCode::Digit2 => "2",
        KeyCode::Digit3 => "3",
        KeyCode::Digit4 => "4",
        _ => "5",
    };
    for (k, (label, value, key)) in settings.iter().enumerate() {
        let y = top + 28.0 + k as f32 * row_h;
        let (x, w2) = (rx + 8.0, rw - 16.0);
        let editing = k == 0 && s.edit_password.is_some();
        if host {
            let hovered = c.hovered(x, y, w2, row_h - 2.0);
            if hovered || editing {
                c.round_rect(
                    [x, y],
                    [w2, row_h - 2.0],
                    5.0,
                    rgba(0x262634, 1.0),
                    rgba(0, 0.0),
                    0.0,
                );
            }
            if let Some(key) = key {
                c.hit(x, y, w2, row_h - 2.0, Hit::Key(*key));
                c.text(x + 6.0, y + 4.0, 6.5, rgba(0x55556a, 1.0), key_name(*key));
            }
        }
        c.text(x + 30.0, y + 4.0, 7.5, rgba(0x9a9ab0, 1.0), label);
        let label_w = Canvas::text_width(7.5, label);
        c.text_right(
            x + w2 - 6.0,
            y + 4.0,
            7.5,
            rgba(if editing { 0xFFD34D } else { 0xffffff }, 1.0),
            &Canvas::fit(7.5, w2 - 46.0 - label_w, value),
        );
    }

    // The chat.
    let cy = top + room_h + 8.0;
    let ch = bottom - cy;
    if ch > 44.0 {
        c.round_rect(
            [rx, cy],
            [rw, ch],
            8.0,
            rgba(0x15151d, 0.86),
            rgba(0x2a2a38, 1.0),
            1.0,
        );
        c.text(
            rx + 14.0,
            cy + 10.0,
            8.0,
            rgba(0x4fb4ff, 1.0),
            tr!("CHAT", "チャット", "ЧАТ"),
        );
        // The line to write on, along the bottom of the box.
        let (ix, iw, ih) = (rx + 8.0, rw - 16.0, 16.0);
        let iy = cy + ch - ih - 6.0;
        let focused = s.chat_input.is_some();
        let hover = c.hovered(ix, iy, iw, ih);
        c.round_rect(
            [ix, iy],
            [iw, ih],
            4.0,
            rgba(0x0e0e14, 0.9),
            rgba(
                if focused {
                    crate::render::accent()
                } else if hover {
                    0x6a6a88
                } else {
                    0x2a2a38
                },
                1.0,
            ),
            1.0,
        );
        match &s.chat_input {
            Some(typed) => {
                let room = iw - 16.0;
                let shown = Canvas::fit_end(7.0, room, typed);
                c.text(ix + 8.0, iy + 4.5, 7.0, rgba(0xffffff, 1.0), &shown);
                // The caret sits in the text when all of it fits, else at the end.
                let at = if shown == *typed {
                    crate::settings::caret_x(7.0, typed, s.chat_caret)
                } else {
                    Canvas::text_width(7.0, &shown)
                };
                c.rect(ix + 8.0 + at, iy + 3.0, 1.0, ih - 6.0, rgba(0xffffff, 1.0));
            }
            None => {
                c.text(
                    ix + 8.0,
                    iy + 4.5,
                    7.0,
                    rgba(0x6f6f86, 1.0),
                    tr!("T - write", "T - 書く", "T - написать"),
                );
            }
        }
        c.hit(ix, iy, iw, ih, Hit::LobbyChat);
        let lines = ((ch - 30.0 - ih - 6.0) / 11.0).max(0.0) as usize;
        for (k, (from, text)) in s
            .chat
            .iter()
            .skip(s.chat.len().saturating_sub(lines))
            .enumerate()
        {
            c.text(
                rx + 14.0,
                cy + 26.0 + k as f32 * 11.0,
                6.5,
                rgba(0xdedeea, 1.0),
                &Canvas::fit(6.5, rw - 28.0, &format!("{from}: {text}")),
            );
        }
    }

    // What can be done, under it all.
    let ready = s.players.iter().filter(|p| p.ready && p.has_map).count();
    let me_ready = s.me().is_some_and(|p| p.ready);
    let me_spectator = s.me().is_some_and(|p| p.spectator);
    let ready_label = if me_ready {
        "Unready  (Space)"
    } else {
        "Ready  (Space)"
    };
    let spectate_label = if me_spectator {
        "Play instead  (S)"
    } else {
        "Spectate  (S)"
    };
    let start_label = format!("Start  (Enter, {ready} ready)");
    if s.in_match {
        c.text_centered(
            w / 2.0,
            426.0,
            8.0,
            rgba(0x9a9ab0, 1.0),
            &Canvas::fit(
                8.0,
                w - 48.0,
                "A match is in progress; you'll join the next one.",
            ),
        );
        let me_playing = s.me().is_some_and(|p| p.status != Status::Lobby);
        if !me_playing && have_map {
            c.button_row(
                440.0,
                &[
                    ("Spectate  (S)", Hit::Key(KeyCode::KeyS), true),
                    ("Leave  (Esc)", Hit::Key(KeyCode::Escape), false),
                ],
            );
        }
    } else if s.is_host() {
        c.button_row(
            440.0,
            &[
                ("Pick map  (M)", Hit::Key(KeyCode::KeyM), false),
                ("Preview / start point  (P)", Hit::Key(KeyCode::KeyP), false),
                (
                    if me_spectator {
                        spectate_label
                    } else {
                        ready_label
                    },
                    Hit::Key(if me_spectator {
                        KeyCode::KeyS
                    } else {
                        KeyCode::Space
                    }),
                    false,
                ),
                (&start_label, Hit::Key(KeyCode::Enter), true),
                ("Leave  (Esc)", Hit::Key(KeyCode::Escape), false),
            ],
        );
        let hint = if s.chat_input.is_some() {
            "type a message   Enter send   Esc cancel"
        } else if s.edit_password.is_some() {
            "type the new password   Enter save (empty: none)   Esc cancel"
        } else {
            "T chat   S spectate   [ ] rate   C defense (1-5 its rules)   K password   L players   F1 mods   Tab songs"
        };
        c.text_centered(
            w / 2.0,
            466.0,
            7.0,
            rgba(0x6f6f86, 1.0),
            &Canvas::fit_hints(7.0, w - 48.0, hint),
        );
    } else {
        let missing = s.map.is_some() && !have_map;
        if missing {
            let busy = download.is_some();
            let label = if busy {
                "Downloading..."
            } else {
                "Get it from the host  (D)"
            };
            c.button_row(
                440.0,
                &[
                    (label, Hit::Key(KeyCode::KeyD), true),
                    (
                        if busy { "..." } else { "From the mirrors  (M)" },
                        Hit::Key(KeyCode::KeyM),
                        false,
                    ),
                    ("Leave  (Esc)", Hit::Key(KeyCode::Escape), false),
                ],
            );
        } else {
            let mut buttons = vec![("Preview  (P)", Hit::Key(KeyCode::KeyP), false)];
            if !me_spectator {
                // Everyone picks their own mods (the speed is the host's).
                buttons.push(("Mods  (F1)", Hit::Key(KeyCode::F1), false));
                buttons.push((ready_label, Hit::Key(KeyCode::Space), true));
            }
            buttons.push((spectate_label, Hit::Key(KeyCode::KeyS), me_spectator));
            buttons.push(("Leave  (Esc)", Hit::Key(KeyCode::Escape), false));
            c.button_row(440.0, &buttons);
            if me_spectator {
                c.text_centered(
                    w / 2.0,
                    466.0,
                    7.0,
                    rgba(0x4fb4ff, 1.0),
                    "you'll watch the next match from its start",
                );
            }
        }
    }
    if let Some(r) = &s.results {
        let finished: Vec<(&PlayerInfo, &Final)> = r
            .iter()
            .filter_map(|p| p.result.as_ref().map(|f| (p, f)))
            .collect();
        if let Some(top_score) = finished.iter().map(|(_, f)| f.score).max() {
            let winners: Vec<&str> = finished
                .iter()
                .filter(|(_, f)| f.score == top_score)
                .map(|(p, _)| p.name.as_str())
                .collect();
            let acc = finished
                .iter()
                .find(|(_, f)| f.score == top_score)
                .map(|(_, f)| f.accuracy * 100.0)
                .unwrap_or(0.0);
            let text = if winners.len() > 1 {
                format!(
                    "Last match: tie between {} at {top_score:08}",
                    winners.join(" & ")
                )
            } else {
                format!(
                    "Last match winner: {} with {top_score:08} ({acc:.2}%)",
                    winners[0]
                )
            };
            let size = Canvas::fit_size(8.0, 6.5, w - 48.0, &text);
            c.text_centered(
                w / 2.0,
                424.0 + (8.0 - size) / 2.0,
                size,
                rgba(0xFFD34D, 1.0),
                &Canvas::fit(size, w - 48.0, &text),
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// In match

/// Waiting for everyone to load, then the countdown to the shared start time.
pub fn draw_waiting(
    c: &mut Canvas,
    s: &Session,
    start: Option<Instant>,
    loaded: bool,
    now: Instant,
) {
    let w = c.width;
    if let Some(m) = &s.map {
        c.text_centered(
            w / 2.0,
            120.0,
            11.0,
            rgba(0x9a9ab0, 1.0),
            &Canvas::fit(11.0, w - 48.0, &m.label()),
        );
    }
    let big = match start {
        Some(at) if at > now => format!("{}", (at - now).as_secs_f64().ceil() as u32),
        Some(_) => "GO".into(),
        None if loaded => "waiting for players...".into(),
        None => "loading...".into(),
    };
    let size = if start.is_some() { 48.0 } else { 16.0 };
    c.text_centered(w / 2.0, 180.0, size, rgba(0xffffff, 1.0), &big);
    for (i, p) in s
        .players
        .iter()
        .filter(|p| p.status != Status::Lobby)
        .enumerate()
        .take(10)
    {
        let (st, color) = status_text(p, true);
        c.text_centered(
            w / 2.0,
            270.0 + i as f32 * 16.0,
            9.0,
            rgba(color, 1.0),
            &Canvas::fit(9.0, w - 48.0, &format!("{}  -  {st}", p.name)),
        );
    }
}

/// Live standings drawn over the playfield.
/// During the intro: how many players have asked to skip it (it's skipped once all of them
/// have), or that Space asks.
pub fn draw_skip_votes(c: &mut Canvas, s: &Session) {
    let text = match (s.skip_asked, s.skip_votes) {
        (_, Some((votes, of))) => tr!(
            "SKIP {votes}/{of}  waiting for everyone",
            "スキップ {votes}/{of}  全員待ち",
            "ПРОПУСК {votes}/{of}  ждём всех";
        ),
        (true, None) => tr!("SKIP asked", "スキップ要求済み", "ПРОПУСК запрошен").to_string(),
        (false, None) => tr!(
            "Space: ask to skip (everyone must)",
            "Space: スキップ (全員必要)",
            "Space: попросить пропуск (нужны все)"
        )
        .to_string(),
    };
    let (w, y) = (c.width, 392.0);
    let tw = Canvas::text_width(8.0, &text) + 20.0;
    c.round_rect(
        [w / 2.0 - tw / 2.0, y - 5.0],
        [tw, 18.0],
        6.0,
        rgba(0x000000, 0.55),
        rgba(accent(), 0.8),
        1.0,
    );
    c.text_centered(w / 2.0, y, 8.0, rgba(0xffffff, 1.0), &text);
}

pub fn draw_scoreboard(c: &mut Canvas, s: &Session, my_live: &Live) {
    let y0 = 282.0;
    let my_id = s.client.id;
    let mut rows: Vec<(String, Live, bool, bool)> = s
        .players
        .iter()
        .filter(|p| p.status != Status::Lobby)
        .map(|p| {
            let me = Some(p.id) == my_id;
            // Our own row uses local numbers so it never lags behind the playfield.
            let live = if me { my_live.clone() } else { p.live.clone() };
            (
                p.name.clone(),
                live,
                me,
                p.result.as_ref().is_some_and(|r| r.quit),
            )
        })
        .collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.1.score));
    let x = 18.0;
    let h = 16.0;
    let lines: Vec<(String, u32)> = rows
        .iter()
        .enumerate()
        .take(8)
        .map(|(i, (name, live, me, quit))| {
            let color = if *me {
                0xFFD34D
            } else if *quit {
                0x6f6f86
            } else {
                0xc8c8d8
            };
            let name: String = name.chars().take(12).collect();
            (
                format!(
                    "{}. {:<12} {:08} {:6.2}% {:>4}x",
                    i + 1,
                    name,
                    live.score,
                    live.accuracy * 100.0,
                    live.combo
                ),
                color,
            )
        })
        .collect();
    // As wide as its longest line (wider when text comes out bigger).
    let bw = lines
        .iter()
        .map(|(l, _)| Canvas::text_width(8.0, l) + 12.0)
        .fold(300.0, f32::max);
    c.round_rect(
        [x - 6.0, y0 - 6.0],
        [bw, lines.len() as f32 * h + 10.0],
        6.0,
        rgba(0x000000, 0.45),
        rgba(0, 0.0),
        0.0,
    );
    for (i, (line, color)) in lines.iter().enumerate() {
        c.text(x, y0 + i as f32 * h, 8.0, rgba(*color, 1.0), line);
    }
}

pub fn draw_results(c: &mut Canvas, s: &Session) {
    let w = c.width;
    let final_results = s.results.as_ref();
    let mut rows: Vec<PlayerInfo> = match final_results {
        Some(r) => r.clone(),
        None => s
            .players
            .iter()
            .filter(|p| p.status != Status::Lobby)
            .cloned()
            .collect(),
    };
    let defense = s.map.as_ref().and_then(|m| m.defense);
    let (title, title_col) = match (final_results, defense) {
        (Some(_), Some(d)) => defense_verdict(d, s.team, &rows),
        (Some(_), None) => ("MATCH RESULTS".to_string(), 0xffffff),
        (None, _) => (
            "waiting for other players to finish...".to_string(),
            0xffffff,
        ),
    };
    c.text_centered(w / 2.0, 50.0, 14.0, rgba(title_col, 1.0), &title);
    if let Some(m) = &s.map {
        c.text_centered(
            w / 2.0,
            76.0,
            9.0,
            rgba(0x9a9ab0, 1.0),
            &Canvas::fit(9.0, w - 48.0, &m.label()),
        );
    }
    let score = |p: &PlayerInfo| p.result.as_ref().map(|r| r.score).unwrap_or(p.live.score);
    rows.sort_by_key(|p| std::cmp::Reverse(score(p)));
    let col = (w - 48.0).min(700.0);
    let x0 = (w - col) / 2.0;
    c.text(
        x0 + 14.0,
        102.0,
        8.0,
        rgba(0x6f6f86, 1.0),
        "#  PLAYER            SCORE      ACC    COMBO  GREAT  OK  MISS    UR",
    );
    for (i, p) in rows.iter().enumerate().take(10) {
        let y = 118.0 + i as f32 * 28.0;
        let me = Some(p.id) == s.client.id;
        let border = if i == 0 && final_results.is_some() {
            0xFFD34D
        } else {
            0x2a2a38
        };
        c.round_rect(
            [x0, y],
            [col, 24.0],
            6.0,
            rgba(if me { 0x2b2b3d } else { 0x1d1d27 }, 1.0),
            rgba(border, 1.0),
            1.0,
        );
        let name: String = p.name.chars().take(14).collect();
        let line = match &p.result {
            Some(r) => format!(
                "{}  {:<14} {:08} {:6.2}% {:>5}x {:>6} {:>3} {:>5} {:>5.0}{}",
                i + 1,
                name,
                r.score,
                r.accuracy * 100.0,
                r.max_combo,
                r.great,
                r.ok,
                r.miss,
                r.ur,
                if r.quit {
                    "  quit".to_string()
                } else if p.mods.is_empty() {
                    String::new()
                } else {
                    format!("  +{}", p.mods.concat())
                }
            ),
            None => format!(
                "{}  {:<14} {:08} {:6.2}%   playing ({:.0}%)",
                i + 1,
                name,
                p.live.score,
                p.live.accuracy * 100.0,
                p.live.progress * 100.0
            ),
        };
        c.text(
            x0 + 14.0,
            y + 8.0,
            8.0,
            rgba(if me { 0xFFD34D } else { 0xffffff }, 1.0),
            &Canvas::fit(8.0, col - 28.0, &line),
        );
    }
    c.button_row(
        440.0,
        &[(
            "Back to lobby  (Esc)",
            Hit::Key(KeyCode::Escape),
            final_results.is_some(),
        )],
    );
}
