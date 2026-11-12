//! Mapping sessions from the game's side: whether the server runs them, invites, getting into
//! one until the editor opens on it (`editor::collab::connect`), and the host inviting friends.
//! A server that doesn't list `mapping::FEATURE` never hears of any of it.

use crate::*;
use editor::collab::connect::{Connecting, HostSet, Ready};
use editor::view::collab::Relink;
use taiko_proto::mapping::{self, HostCmd, Link, Mode};

/// Getting into a session: the connection and how far along it is.
pub(crate) struct MappingJoin {
    link: Link,
    connecting: Connecting,
    server: String,
    token: String,
}

impl State {
    /// Whether the game speaks of mapping sessions: logged in to a server that runs them.
    pub(crate) fn mapping_ok(&self) -> bool {
        self.mapping_sessions && !self.cfg.online_token.is_empty()
    }

    /// `from` invites us to their session: a notification that joins it when clicked.
    pub(crate) fn mapping_invited(
        &mut self,
        from: String,
        session: u64,
        title: String,
        mode: Mode,
    ) {
        let what = match mode {
            Mode::Guest => tr!("a guest difficulty", "ゲスト難易度", "гостевую сложность"),
            Mode::Review => tr!("a review", "レビュー", "ревью"),
            Mode::Collab => tr!("a collab", "コラボ", "коллаб"),
        };
        self.social.notify(
            tr!("{from} invites you to map", "{from} からマッピングの招待", "{from} зовёт маппить"; [] [from]),
            format!("{what}: {title}"),
            0xC89CF2,
            social::NoteAction::Mapping(session),
        );
    }

    /// A session we were invited to is over: its invite goes.
    pub(crate) fn mapping_closed(&mut self, session: u64) {
        self.social
            .notes
            .retain(|n| n.action != social::NoteAction::Mapping(session));
    }

    fn start_mapping(&mut self, request: online::Request, connecting: Connecting) {
        let server = self.cfg.online_server.clone();
        let token = self.cfg.online_token.clone();
        self.mapping_join = Some(Box::new(MappingJoin {
            link: Link::start(server.clone(), request),
            connecting,
            server,
            token,
        }));
    }

    /// Joins the session we were invited to.
    pub(crate) fn join_mapping(&mut self, session: u64) {
        if !self.mapping_ok() || self.mapping_join.is_some() {
            return;
        }
        let token = self.cfg.online_token.clone();
        let songs = self.songs_dir.clone();
        self.start_mapping(
            online::Request::JoinMapping { token, session },
            Connecting::guest(songs),
        );
        self.toast(
            tr!(
                "joining the mapping session...",
                "マッピングセッションに参加中...",
                "подключаюсь к совместному маппингу..."
            )
            .into(),
        );
    }

    /// Opens a session for the set in `dir`, as its host.
    pub(crate) fn open_mapping(&mut self, dir: &Path, mode: Mode) {
        if !self.mapping_ok() || self.mapping_join.is_some() {
            return;
        }
        let set = match HostSet::read(dir) {
            Ok(set) => set,
            Err(e) => {
                self.toast(tr!("can't start a mapping session: {e}", "マッピングセッションを開始できません: {e}", "не получилось начать совместный маппинг: {e}"; [] [e]));
                return;
            }
        };
        let token = self.cfg.online_token.clone();
        let request = online::Request::OpenMapping {
            token,
            set: set.descriptor(),
            mode,
        };
        self.start_mapping(request, Connecting::host(set));
        self.toast(
            tr!(
                "starting the mapping session...",
                "マッピングセッションを開始中...",
                "начинаю совместный маппинг..."
            )
            .into(),
        );
    }

    /// Every frame while getting into a session: what the connection says, and the editor
    /// opened on the session once everything is in.
    pub(crate) fn poll_mapping(&mut self) {
        self.social.can_invite = self.mapping_ok()
            && self
                .scene
                .screen_as::<MapEditor>()
                .is_some_and(|e| e.view.hosting());
        let Some(j) = &mut self.mapping_join else {
            return;
        };
        let mut failed = None;
        for ev in j.link.poll() {
            match j.connecting.take(ev) {
                Ok(ups) => ups.into_iter().for_each(|u| j.link.send(u)),
                Err(why) => {
                    failed = Some(why);
                    break;
                }
            }
        }
        let ready = match failed {
            Some(why) => Some(Err(why)),
            None => j.connecting.ready(),
        };
        match ready {
            None => {
                let pct = (j.connecting.progress() * 100.0).round() as u32;
                let stage = j.connecting.stage();
                self.toast(tr!("mapping session: {stage}... {pct}%", "マッピングセッション: {stage}... {pct}%", "совместный маппинг: {stage}... {pct}%"; [] [stage, pct]));
            }
            Some(Err(why)) => {
                self.mapping_join = None;
                self.toast(tr!("couldn't get into the mapping session: {why}", "マッピングセッションに入れませんでした: {why}", "не удалось войти в совместный маппинг: {why}"; [] [why]));
            }
            Some(Ok(r)) => {
                let Some(j) = self.mapping_join.take() else {
                    return;
                };
                self.open_mapping_editor(r, *j);
            }
        }
    }

    fn open_mapping_editor(&mut self, r: Ready, j: MappingJoin) {
        let Some(entry) = self.entry_for(&r.path) else {
            self.toast(
                tr!(
                    "can't open the session's map",
                    "セッションのマップを開けません",
                    "не открыть карту сессии"
                )
                .into(),
            );
            return;
        };
        self.open_editor_at(r.path.clone(), entry);
        let Some(MapEditor { view, .. }) = self.scene.screen_mut::<MapEditor>() else {
            return;
        };
        let relink = Relink {
            server: j.server,
            token: j.token,
            session: r.session,
            epoch: r.epoch,
            retry_at: None,
        };
        if let Err(e) = view.attach_session(r, j.link, relink) {
            self.toast(tr!("couldn't get into the mapping session: {e}", "マッピングセッションに入れませんでした: {e}", "не удалось войти в совместный маппинг: {e}"; [] [e]));
        }
    }

    /// The host invites a friend to the session open in the editor.
    pub(crate) fn invite_to_mapping(&mut self, name: String) {
        if let Some(MapEditor { view, .. }) = self.scene.screen_mut::<MapEditor>()
            && view.hosting()
        {
            view.send_host(HostCmd::Invite(name.clone()));
            self.toast(
                tr!("invited {name}", "{name} を招待しました", "{name} приглашён(а)"; [] [name]),
            );
        }
    }

    /// The features to declare on the social connection (none to a server that runs no
    /// mapping sessions).
    pub(crate) fn social_features(&self) -> Vec<String> {
        if self.mapping_sessions {
            vec![mapping::FEATURE.to_string()]
        } else {
            Vec::new()
        }
    }
}
