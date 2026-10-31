//! Genres, read from a map's tags: the player's folders. Tags are free text, so each genre is
//! the words mappers actually use for it, matched whole ("metalcore" isn't "core"). A song can
//! be in several.

pub struct Genre {
    pub name: fn() -> &'static str,
    /// The folder's colour.
    pub colour: u32,
    /// Single words, or several in a row.
    words: &'static [&'static str],
}

pub const GENRES: [Genre; 17] = [
    Genre { name: || tr!("Electronic", "エレクトロニック", "Электроника"), colour: 0x4fb4ff, words: &["electronic", "electronica", "electro", "edm", "synthwave", "synthpop", "idm"] },
    Genre { name: || tr!("Hardcore", "ハードコア", "Хардкор"), colour: 0xeb452c, words: &["hardcore", "speedcore", "gabber", "frenchcore", "hardstyle", "j-core", "jcore", "artcore", "uptempo"] },
    Genre { name: || "Breakcore", colour: 0xc77dff, words: &["breakcore", "mashcore", "dariacore", "breakbeat"] },
    Genre { name: || "Drum & Bass", colour: 0xffd34d, words: &["dnb", "d&b", "d'n'b", "drumnbass", "neurofunk", "jungle", "liquid funk", "drum and bass", "drum & bass", "drum n bass", "drum'n'bass"] },
    Genre { name: || "Dubstep", colour: 0x86e05a, words: &["dubstep", "riddim", "brostep", "trap", "future bass", "bass house", "color bass"] },
    Genre { name: || tr!("Trance", "トランス", "Транс"), colour: 0x5ee6c8, words: &["trance", "psytrance", "psy-trance", "goa"] },
    Genre { name: || tr!("Techno & House", "テクノ / ハウス", "Техно и хаус"), colour: 0xff8a3d, words: &["techno", "house", "eurobeat", "garage", "disco"] },
    Genre { name: || tr!("Pop", "ポップ", "Поп"), colour: 0xff5fa2, words: &["pop", "j-pop", "jpop", "k-pop", "kpop", "denpa"] },
    Genre { name: || tr!("Rock", "ロック", "Рок"), colour: 0xd9654b, words: &["rock", "j-rock", "jrock", "punk", "grunge", "emo"] },
    Genre { name: || tr!("Metal", "メタル", "Метал"), colour: 0x8a8aa0, words: &["metal", "metalcore", "deathcore", "djent", "thrash"] },
    Genre { name: || tr!("Hip Hop", "ヒップホップ", "Хип-хоп"), colour: 0xe0a458, words: &["hip-hop", "hiphop", "rap", "hip hop", "lofi", "lo-fi"] },
    Genre { name: || tr!("Anime", "アニメ", "Аниме"), colour: 0xff7eb6, words: &["anime", "tv size", "tv-size", "アニメ"] },
    Genre { name: || tr!("Game music", "ゲーム音楽", "Музыка из игр"), colour: 0x7bd88f, words: &["vgm", "videogame", "video game", "game soundtrack", "ost"] },
    Genre { name: || "Touhou", colour: 0xf25c78, words: &["touhou", "東方", "東方project", "zun"] },
    Genre { name: || "Vocaloid", colour: 0x39c5bb, words: &["vocaloid", "miku", "初音ミク", "utau", "ボーカロイド"] },
    Genre { name: || tr!("Classical", "クラシック", "Классика"), colour: 0xc9b37e, words: &["classical", "orchestra", "orchestral", "symphony", "baroque"] },
    Genre { name: || tr!("Chiptune", "チップチューン", "Чиптюн"), colour: 0x9fe870, words: &["chiptune", "chip", "8bit", "8-bit", "bitpop"] },
];

/// The genres `tags` name, a bit for each of `GENRES`.
pub fn of(tags: &str) -> u32 {
    let tags = tags.to_lowercase();
    let words: Vec<&str> = tags.split_whitespace().collect();
    let says = |phrase: &&str| {
        let wanted: Vec<&str> = phrase.split(' ').collect();
        words.windows(wanted.len()).any(|w| w == wanted.as_slice())
    };
    GENRES.iter().enumerate().filter(|(_, g)| g.words.iter().any(|w| says(&w))).fold(0, |bits, (i, _)| bits | 1 << i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(bits: u32) -> Vec<&'static str> {
        GENRES.iter().enumerate().filter(|(i, _)| bits >> i & 1 != 0).map(|(_, g)| (g.name)()).collect()
    }

    #[test]
    fn genres_come_from_whole_words_and_phrases_in_any_case() {
        crate::lang::set(crate::lang::Lang::En);
        assert_eq!(named(of("Japanese J-POP featured artist")), ["Pop"]);
        assert_eq!(named(of("electronic dnb neurofunk")), ["Electronic", "Drum & Bass"]);
        assert_eq!(named(of("liquid drum and bass")), ["Drum & Bass"]);
        assert_eq!(named(of("東方project 上海アリス幻樂団 ZUN")), ["Touhou"]);
        assert_eq!(of("drum bass game the of"), 0, "the words alone say nothing");
        assert_eq!(of("popcorn rocket metallic"), 0, "only whole words");
        assert_eq!(of(""), 0);
    }

    #[test]
    fn every_genre_fits_in_the_bits() {
        assert!(GENRES.len() <= 32);
    }
}
