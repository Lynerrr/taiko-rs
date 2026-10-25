//! Reads osu! stable's `osu!.db` (for each map: its MD5, folder, file and when it was last
//! played) and `collection.db` (named lists of map MD5s), next to the Songs folder.

use std::collections::HashMap;
use std::path::Path;

/// Formats of the star rating pairs changed from Int-Double to Int-Float in this version.
const FLOAT_STARS_VERSION: i32 = 20250107;
/// Entries stopped starting with their size in bytes in this version.
const NO_ENTRY_SIZE_VERSION: i32 = 20191106;
/// Difficulty values became floats and star ratings were added in this version.
const FLOAT_DIFF_VERSION: i32 = 20140609;

pub struct DbMap {
    pub folder: String,
    pub file: String,
    /// Unix seconds, 0 if never played.
    pub last_played: u64,
    /// osu!stable's ranked status: 4 ranked, 5 approved, 6 qualified, 7 loved, 2 pending (or
    /// graveyard / WIP: stable doesn't tell them apart), 1 not submitted, 0 unknown.
    pub status: u8,
}

#[derive(Default)]
pub struct OsuDb {
    pub by_md5: HashMap<String, DbMap>,
    /// (name, map MD5s).
    pub collections: Vec<(String, Vec<String>)>,
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn i16(&mut self) -> Option<i16> {
        Some(i16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn i64(&mut self) -> Option<i64> {
        Some(i64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn skip(&mut self, n: usize) -> Option<()> {
        self.take(n).map(|_| ())
    }
    /// 0x00 = empty, 0x0b = ULEB128 length + UTF-8.
    fn string(&mut self) -> Option<String> {
        match self.u8()? {
            0x00 => Some(String::new()),
            0x0b => {
                let (mut len, mut shift) = (0usize, 0);
                loop {
                    let byte = self.u8()?;
                    len |= ((byte & 0x7f) as usize) << shift;
                    if byte & 0x80 == 0 {
                        break;
                    }
                    shift += 7;
                    if shift > 35 {
                        return None;
                    }
                }
                Some(String::from_utf8_lossy(self.take(len)?).into_owned())
            }
            _ => None,
        }
    }
}

/// .NET DateTime ticks (100 ns since year 1) to Unix seconds.
fn ticks_to_unix(ticks: i64) -> u64 {
    const EPOCH_TICKS: i64 = 621_355_968_000_000_000;
    if ticks <= EPOCH_TICKS { 0 } else { ((ticks - EPOCH_TICKS) / 10_000_000) as u64 }
}

fn parse_osu_db(b: &[u8]) -> Option<HashMap<String, DbMap>> {
    let mut r = Reader { b, at: 0 };
    let version = r.i32()?;
    r.i32()?; // folder count
    r.u8()?; // account unlocked
    r.i64()?; // unlock date
    r.string()?; // player name
    let count = r.i32()?.max(0) as usize;
    let mut out = HashMap::with_capacity(count);
    for _ in 0..count {
        if version < NO_ENTRY_SIZE_VERSION {
            r.i32()?;
        }
        for _ in 0..5 {
            r.string()?; // artist, artist unicode, title, title unicode, creator
        }
        r.string()?; // difficulty
        r.string()?; // audio file
        let md5 = r.string()?;
        let file = r.string()?;
        let status = r.u8()?;
        r.skip(6)?; // circles, sliders, spinners
        r.i64()?; // last modified
        if version < FLOAT_DIFF_VERSION {
            r.skip(4)?; // AR CS HP OD as bytes
        } else {
            r.skip(16)?; // as floats
        }
        r.skip(8)?; // slider velocity
        if version >= FLOAT_DIFF_VERSION {
            let pair = if version >= FLOAT_STARS_VERSION { 1 + 4 + 1 + 4 } else { 1 + 4 + 1 + 8 };
            for _ in 0..4 {
                let n = r.i32()?.max(0) as usize;
                r.skip(n.checked_mul(pair)?)?;
            }
        }
        r.skip(12)?; // drain time, total time, preview time
        let points = r.i32()?.max(0) as usize;
        r.skip(points.checked_mul(17)?)?;
        r.skip(12)?; // difficulty id, set id, thread id
        r.skip(4)?; // grades
        r.skip(2)?; // local offset
        r.skip(4)?; // stack leniency
        r.u8()?; // mode
        r.string()?; // source
        r.string()?; // tags
        r.i16()?; // online offset
        r.string()?; // font
        r.u8()?; // unplayed
        let last_played = r.i64()?;
        r.u8()?; // osz2
        let folder = r.string()?;
        r.i64()?; // last checked
        r.skip(5)?; // ignore sounds / skin, disable storyboard / video, visual override
        if version < FLOAT_DIFF_VERSION {
            r.skip(2)?;
        }
        r.i32()?; // last modification (?)
        r.u8()?; // mania scroll speed
        if !md5.is_empty() {
            out.insert(md5, DbMap { folder, file, last_played: ticks_to_unix(last_played), status });
        }
    }
    Some(out)
}

fn parse_collections(b: &[u8]) -> Option<Vec<(String, Vec<String>)>> {
    let mut r = Reader { b, at: 0 };
    r.i32()?; // version
    let count = r.i32()?.max(0) as usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let name = r.string()?;
        let n = r.i32()?.max(0) as usize;
        let mut maps = Vec::with_capacity(n.min(100_000));
        for _ in 0..n {
            maps.push(r.string()?);
        }
        out.push((name, maps));
    }
    Some(out)
}

/// Reads the databases of the osu! install owning `songs` (its parent folder), if any.
pub fn load(songs: &Path) -> Option<OsuDb> {
    let dir = songs.parent()?;
    let db = std::fs::read(dir.join("osu!.db")).ok()?;
    let by_md5 = parse_osu_db(&db).unwrap_or_else(|| {
        eprintln!("osu!.db: unrecognised format, collections and play history unavailable");
        HashMap::new()
    });
    let collections = std::fs::read(dir.join("collection.db")).ok().and_then(|b| parse_collections(&b)).unwrap_or_default();
    Some(OsuDb { by_md5, collections })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(out: &mut Vec<u8>, v: &str) {
        out.push(0x0b);
        out.push(v.len() as u8);
        out.extend_from_slice(v.as_bytes());
    }

    #[test]
    fn reads_a_minimal_osu_db_and_collections() {
        let mut b = Vec::new();
        b.extend_from_slice(&20250108i32.to_le_bytes());
        b.extend_from_slice(&1i32.to_le_bytes()); // folders
        b.push(1);
        b.extend_from_slice(&0i64.to_le_bytes());
        s(&mut b, "me");
        b.extend_from_slice(&1i32.to_le_bytes()); // one map
        for v in ["A", "A", "T", "T", "C", "Oni", "audio.mp3", "abc123", "A - T (C) [Oni].osu"] {
            s(&mut b, v);
        }
        b.push(4);
        b.extend_from_slice(&[0; 6]);
        b.extend_from_slice(&0i64.to_le_bytes());
        b.extend_from_slice(&[0; 16]);
        b.extend_from_slice(&1.4f64.to_le_bytes());
        for _ in 0..4 {
            b.extend_from_slice(&1i32.to_le_bytes());
            b.push(0x08);
            b.extend_from_slice(&0i32.to_le_bytes());
            b.push(0x0c);
            b.extend_from_slice(&4.5f32.to_le_bytes());
        }
        b.extend_from_slice(&[0; 12]);
        b.extend_from_slice(&1i32.to_le_bytes()); // one timing point
        b.extend_from_slice(&[0; 17]);
        b.extend_from_slice(&[0; 12 + 4 + 2 + 4]);
        b.push(1); // taiko
        b.push(0); // source
        b.push(0); // tags
        b.extend_from_slice(&0i16.to_le_bytes());
        b.push(0); // font
        b.push(0); // unplayed
        // 2021-01-01 00:00:00 UTC.
        b.extend_from_slice(&(621_355_968_000_000_000i64 + 1_609_459_200 * 10_000_000).to_le_bytes());
        b.push(0);
        s(&mut b, "123 A - T");
        b.extend_from_slice(&0i64.to_le_bytes());
        b.extend_from_slice(&[0; 5]);
        b.extend_from_slice(&0i32.to_le_bytes());
        b.push(0);
        let maps = parse_osu_db(&b).expect("parses");
        let m = &maps["abc123"];
        assert_eq!((m.folder.as_str(), m.file.as_str(), m.last_played), ("123 A - T", "A - T (C) [Oni].osu", 1_609_459_200));

        let mut c = Vec::new();
        c.extend_from_slice(&20250108i32.to_le_bytes());
        c.extend_from_slice(&1i32.to_le_bytes());
        s(&mut c, "practice");
        c.extend_from_slice(&2i32.to_le_bytes());
        s(&mut c, "abc123");
        s(&mut c, "def456");
        assert_eq!(parse_collections(&c).unwrap(), vec![("practice".to_string(), vec!["abc123".to_string(), "def456".to_string()])]);
    }
}
