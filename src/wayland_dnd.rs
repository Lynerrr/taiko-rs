//! Drag and drop on Wayland, which winit 0.30 only does on Windows, X11 and macOS: a
//! `wl_data_device` of our own on winit's connection, with its own event queue and thread.

use std::io::Read;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

use wayland_backend::client::Backend;
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{event_created_child, Connection, Dispatch, Proxy, QueueHandle};

use crate::FileDrag;

const URI_LIST: &str = "text/uri-list";

/// Listens for files dragged over the game's windows on the Wayland connection `display` (winit's
/// `wl_display`, which lives as long as the game) and reports them to `send`.
pub fn start(display: *mut std::ffi::c_void, send: impl Fn(FileDrag) + Send + 'static) {
    // SAFETY: winit keeps the display connected until the process exits.
    let backend = unsafe { Backend::from_foreign_display(display.cast()) };
    let conn = Connection::from_backend(backend);
    let _ = std::thread::Builder::new().name("drag and drop".into()).spawn(move || {
        let mut queue = conn.new_event_queue();
        conn.display().get_registry(&queue.handle(), ());
        let mut dnd = Dnd { conn: conn.clone(), manager: None, seat: None, offer: None, send: Box::new(send) };
        while queue.blocking_dispatch(&mut dnd).is_ok() {}
    });
}

struct Dnd {
    conn: Connection,
    manager: Option<WlDataDeviceManager>,
    seat: Option<WlSeat>,
    /// What is being dragged over a window right now, if it is files.
    offer: Option<WlDataOffer>,
    send: Box<dyn Fn(FileDrag) + Send>,
}

impl Dnd {
    /// Once there are both, a data device for the seat (the first one: there's rarely another).
    fn get_device(&self, qh: &QueueHandle<Self>) {
        if let (Some(manager), Some(seat)) = (&self.manager, &self.seat) {
            manager.get_data_device(seat, qh, ());
        }
    }

    /// The dropped files: the offer's `text/uri-list`, read through a pipe.
    fn receive(&self, offer: &WlDataOffer) -> Vec<PathBuf> {
        let Ok((mut read, write)) = std::io::pipe() else { return Vec::new() };
        offer.receive(URI_LIST.into(), std::os::fd::AsFd::as_fd(&write));
        let _ = self.conn.flush();
        // The other end goes to whoever dragged the files; they close it when done writing.
        drop(write);
        let mut list = Vec::new();
        let _ = read.read_to_end(&mut list);
        paths(&list)
    }
}

/// The local files in a `text/uri-list` (`file:///home/me/My%20Map.osz`, one a line).
fn paths(list: &[u8]) -> Vec<PathBuf> {
    list.split(|&b| b == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .filter(|line| !line.is_empty() && !line.starts_with(b"#"))
        .filter_map(|line| line.strip_prefix(b"file://"))
        // A host part ("localhost") before the path.
        .map(|rest| &rest[rest.iter().position(|&b| b == b'/').unwrap_or(rest.len())..])
        .map(|path| PathBuf::from(std::ffi::OsString::from_vec(percent_decode(path))))
        .collect()
}

fn percent_decode(s: &[u8]) -> Vec<u8> {
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        match (s[i], s.get(i + 1).and_then(|&b| hex(b)), s.get(i + 2).and_then(|&b| hex(b))) {
            (b'%', Some(hi), Some(lo)) => {
                out.push(hi << 4 | lo);
                i += 3;
            }
            (b, ..) => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

impl Dispatch<WlRegistry, ()> for Dnd {
    fn event(dnd: &mut Self, registry: &WlRegistry, event: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        let wl_registry::Event::Global { name, interface, version } = event else { return };
        match interface.as_str() {
            "wl_seat" if dnd.seat.is_none() => {
                dnd.seat = Some(registry.bind(name, 1, qh, ()));
                dnd.get_device(qh);
            }
            "wl_data_device_manager" if dnd.manager.is_none() => {
                dnd.manager = Some(registry.bind(name, version.min(3), qh, ()));
                dnd.get_device(qh);
            }
            _ => {}
        }
    }
}

impl Dispatch<WlSeat, ()> for Dnd {
    fn event(_: &mut Self, _: &WlSeat, _: wayland_client::protocol::wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlDataDeviceManager, ()> for Dnd {
    fn event(_: &mut Self, _: &WlDataDeviceManager, _: wayland_client::protocol::wl_data_device_manager::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

/// Whether an offer comes as a list of files.
#[derive(Default)]
struct Files(std::sync::atomic::AtomicBool);

impl Dispatch<WlDataDevice, ()> for Dnd {
    fn event(dnd: &mut Self, _: &WlDataDevice, event: wl_data_device::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        use std::sync::atomic::Ordering;
        match event {
            wl_data_device::Event::Enter { serial, id: Some(offer), .. } => {
                if offer.data::<Files>().is_some_and(|f| f.0.load(Ordering::Relaxed)) {
                    offer.accept(serial, Some(URI_LIST.into()));
                    if offer.version() >= 3 {
                        offer.set_actions(DndAction::Copy, DndAction::Copy);
                    }
                    dnd.offer = Some(offer);
                    (dnd.send)(FileDrag::Hover);
                } else {
                    offer.accept(serial, None);
                    offer.destroy();
                }
            }
            wl_data_device::Event::Leave => {
                if let Some(offer) = dnd.offer.take() {
                    offer.destroy();
                    (dnd.send)(FileDrag::Left);
                }
            }
            wl_data_device::Event::Drop => {
                if let Some(offer) = dnd.offer.take() {
                    let files = dnd.receive(&offer);
                    if offer.version() >= 3 {
                        offer.finish();
                    }
                    offer.destroy();
                    (dnd.send)(FileDrag::Drop(files));
                }
            }
            // The clipboard: not ours to read.
            wl_data_device::Event::Selection { id: Some(offer) } => offer.destroy(),
            _ => {}
        }
    }

    event_created_child!(Dnd, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, Files::default()),
    ]);
}

impl Dispatch<WlDataOffer, Files> for Dnd {
    fn event(_: &mut Self, _: &WlDataOffer, event: wl_data_offer::Event, files: &Files, _: &Connection, _: &QueueHandle<Self>) {
        if let wl_data_offer::Event::Offer { mime_type } = event
            && mime_type == URI_LIST
        {
            files.0.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_files_out_of_a_uri_list() {
        let list = b"# from a file manager\r\nfile:///home/me/My%20Map%20%5B1%5D.osz\r\nfile://localhost/tmp/a.osz\r\nhttps://osu.ppy.sh/s/1\n";
        assert_eq!(paths(list), [PathBuf::from("/home/me/My Map [1].osz"), PathBuf::from("/tmp/a.osz")]);
        // Not UTF-8 is fine: paths are bytes on Linux.
        assert_eq!(paths(b"file:///tmp/%D0%BA%D0%B0%D1%80%D1%82%D0%B0.osz"), [PathBuf::from("/tmp/карта.osz")]);
    }
}
