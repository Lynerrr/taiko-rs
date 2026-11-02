//! The public medals, in the order of their sections. Secret ones aren't here (see `medals`).
//! The pictures are in site/medal-art.txt; the dans' are their own names in the game's font.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use crate::medals::{Filter, Medal, Rule, Span};
use crate::online::{Said, ART};

const PATH: u8 = 0;
const COLLECTION: u8 = 1;
const DAN: u8 = 2;
const SKILL: u8 = 3;
const TRIALS: u8 = 4;
const GROWTH: u8 = 5;
const RIVALS: u8 = 6;
const TOGETHER: u8 = 8;

/// The medals' sections, for the website: `Medal::section` indexes this.
pub const SECTIONS: [&str; 9] = ["Path", "Mod collection", "Dan", "Skill", "Mod trials", "Growth", "Rivals", "Ura", "Together"];

/// The mods that go in a collection, by mod select column: all but Autoplay and Relax, whose
/// plays the server doesn't take.
const GROUPS: [(&str, &[&str]); 8] = [
    ("REDUCTION", &["EZ", "NF", "HT", "DC", "HA", "NB"]),
    ("INCREASE", &["HR", "SD", "PF", "DT", "NC", "HD", "FL", "PR", "GN"]),
    ("CONVERSION", &["SW", "RD", "CS", "AL", "WU", "WD", "AB"]),
    ("SCROLL", &["AC", "BR", "WV", "PU", "ST", "CH", "CC", "DR", "GB", "RL"]),
    ("SV LAB", &["AS", "HS", "EV", "SL", "SG", "LF", "PN", "MC", "LG", "MW", "SR"]),
    ("VISUAL", &["SU", "BL", "BO", "SP", "CL", "MM", "DY", "PY", "OT", "LI", "RA", "EC"]),
    ("RAILS", &["ZZ", "UT", "SI", "LP", "EB", "SS", "SK", "XF"]),
    ("PLAYFIELD", &["MR", "TL", "SN", "MN", "BA", "FF", "SH", "CK", "CR", "PT", "MV"]),
];

/// Mods by acronym and name, for the ones a song title could say.
const MOD_NAMES: [(&str, &str); 57] = [
    ("EZ", "Easy"), ("HT", "Half Time"), ("DC", "Daycore"), ("HR", "Hard Rock"), ("SD", "Sudden Death"), ("PF", "Perfect"), ("DT", "Double Time"), ("NC", "Nightcore"),
    ("HD", "Hidden"), ("FL", "Flashlight"), ("PR", "Precise"), ("SW", "Swap"), ("RD", "Random"), ("AL", "Alternate"), ("AC", "Accel"), ("BR", "Brake"), ("WV", "Wave"),
    ("PU", "Pulse"), ("ST", "Stutter"), ("CH", "Chaos"), ("CC", "Crescendo"), ("DR", "Drop"), ("GB", "Gearbox"), ("RL", "Roulette"), ("SL", "Split"), ("SG", "Swing"),
    ("LF", "Leapfrog"), ("PN", "Panic"), ("MC", "Mercy"), ("MW", "Moonwalk"), ("SU", "Sudden"), ("BL", "Blink"), ("BO", "Blackout"), ("SP", "Spotlight"),
    ("CL", "Colorless"), ("MM", "Memory"), ("DY", "Decoy"), ("PY", "Party"), ("OT", "Overtime"), ("LI", "Liar"), ("RA", "Radar"), ("EC", "Echo"), ("ZZ", "Zigzag"),
    ("SI", "Spiral"), ("LP", "Loop"), ("EB", "Elbow"), ("SS", "Shapeshift"), ("SK", "Snake"), ("MR", "Mirror"), ("TL", "Tilt"), ("SN", "Spin"), ("MN", "Mania"),
    ("XF", "Crossfire"), ("SH", "Shatter"), ("CK", "Cracked"), ("CR", "Crumble"), ("PT", "Portal"),
];

pub fn mod_names() -> &'static [(&'static str, &'static str)] {
    &MOD_NAMES
}

/// Every mod that goes in a collection.
pub fn collectable() -> HashSet<&'static str> {
    GROUPS.iter().flat_map(|(_, mods)| mods.iter().copied()).collect()
}

fn say(en: &str, ja: &str, ru: &str) -> Said {
    [en.to_string(), ja.to_string(), ru.to_string()]
}

/// 1,000,000 and 1 000 000.
fn thousands(n: u64, comma: char) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, d) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(comma);
        }
        out.push(d);
    }
    out
}

/// The pictures by name.
fn pictures() -> &'static HashMap<&'static str, Vec<String>> {
    static ART_FILE: OnceLock<HashMap<&'static str, Vec<String>>> = OnceLock::new();
    ART_FILE.get_or_init(|| {
        let mut all: HashMap<&'static str, Vec<String>> = HashMap::new();
        let mut current = None;
        for line in include_str!("../site/medal-art.txt").lines() {
            if let Some(name) = line.strip_prefix("= ") {
                current = Some(name);
                all.insert(name, Vec::new());
            } else if let Some(rows) = current.filter(|_| !line.is_empty()).and_then(|name| all.get_mut(name)) {
                rows.push(line.to_string());
            }
        }
        all
    })
}

fn picture(name: &str) -> Vec<String> {
    pictures().get(name).cloned().unwrap_or_default()
}

/// A ladder's picture: the same on each step, with its step's pips along the bottom.
fn stepped(name: &str, step: usize) -> Vec<String> {
    let mut art = picture(name);
    if let Some(last) = art.last_mut() {
        let width = step * 2 - 1;
        let start = (ART - width) / 2;
        *last = (0..ART).map(|x| if x >= start && x < start + width && (x - start).is_multiple_of(2) { 'x' } else { '.' }).collect();
    }
    art
}

/// A dan's picture: its two characters one over the other, in the game's font.
fn dan_art(name: &str) -> Vec<String> {
    let mut art = vec![vec!['.'; ART]; ART];
    for (k, c) in name.chars().rev().take(2).collect::<Vec<_>>().into_iter().rev().enumerate() {
        let columns = crate::font::glyph(c);
        let left = (ART - columns.len()) / 2;
        for (x, column) in columns.iter().enumerate() {
            for y in 0..8 {
                if column & (1 << y) != 0 {
                    art[y + k * 8][left + x] = '#';
                }
            }
        }
    }
    art.into_iter().map(|row| row.into_iter().collect()).collect()
}

fn medal(id: impl Into<String>, section: u8, tier: u8, name: Said, about: Said, art: Vec<String>, rule: Rule) -> Medal {
    Medal { id: id.into(), section, tier, secret: false, name, about, art, rule, quiet: false }
}

fn clean(accuracy: f64, stars: f64) -> Filter {
    Filter { clean: true, accuracy: Span::from(accuracy), stars: Span::from(stars), ..Filter::default() }
}

fn full_combo(stars: f64) -> Filter {
    Filter { clean: true, full_combo: true, stars: Span::from(stars), ..Filter::default() }
}

fn with(mut f: Filter, mods: &[&str]) -> Filter {
    f.mods = mods.iter().map(|m| m.to_string()).collect();
    f
}

/// A mod's challenge: its names, how it's put, the play, and a picture.
fn trial(id: &str, tier: u8, name: [&str; 3], mods: &str, filter: Filter, art: &str) -> Medal {
    let stars = filter.stars.min.unwrap_or(0.0);
    let about = if filter.full_combo {
        say(&format!("{mods}: full combo at {stars}★+"), &format!("{mods}: ★{stars}以上でフルコンボ"), &format!("{mods}: фулл комбо на {stars}★+"))
    } else {
        let accuracy = (filter.accuracy.min.unwrap_or(0.0) * 100.0).round();
        say(&format!("{mods}: {accuracy}% at {stars}★+"), &format!("{mods}: ★{stars}以上で精度{accuracy}%"), &format!("{mods}: {accuracy}% на {stars}★+"))
    };
    medal(id, TRIALS, tier, say(name[0], name[1], name[2]), about, picture(art), Rule::Play(filter))
}

/// Every public medal.
pub fn all() -> Vec<Medal> {
    let mut v = Vec::new();

    // The path.
    v.push(medal("first_play", PATH, 1, say("First Hit", "はじめの一打", "Первый удар"), say("Keep your first play", "初めてのプレイを記録", "Сохранить первый плей"), picture("drum"), Rule::Plays { need: 1 }));
    let hits = [
        (10_000, 1, ["Warm-Up", "ウォームアップ", "Разогрев"]),
        (100_000, 2, ["Calluses", "マメ", "Мозоли"]),
        (500_000, 3, ["Worn-Out Sticks", "すり減ったバチ", "Стёртые палочки"]),
        (2_000_000, 4, ["Two Million Strikes", "二百万打", "Два миллиона по коже"]),
    ];
    for (i, (need, tier, n)) in hits.into_iter().enumerate() {
        let (en, ru) = (thousands(need, ','), thousands(need, ' '));
        let about = say(&format!("{en} hits in all"), &format!("累計{en}打"), &format!("{ru} попаданий за всё время"));
        v.push(medal(format!("hits_{need}"), PATH, tier, say(n[0], n[1], n[2]), about, stepped("sticks", i + 1), Rule::Hits { need }));
    }
    let maps = [
        (10, 1, ["Tourist", "観光客", "Турист"]),
        (50, 2, ["Traveller", "旅人", "Путешественник"]),
        (150, 3, ["Cartographer", "地図職人", "Картограф"]),
        (500, 4, ["Round the World", "世界一周", "Кругосветка"]),
    ];
    for (i, (need, tier, n)) in maps.into_iter().enumerate() {
        let about = say(&format!("{need} different maps at 70%+"), &format!("精度70%以上で{need}譜面"), &format!("{need} разных карт с точностью 70%+"));
        let filter = Filter { accuracy: Span::from(0.70), ..Filter::default() };
        v.push(medal(format!("maps_{need}"), PATH, tier, say(n[0], n[1], n[2]), about, stepped("compass", i + 1), Rule::Maps { need, filter }));
    }
    let hours = [
        (1, 1, ["First Shift", "初シフト", "Первая смена"]),
        (5, 1, ["Intern", "研修生", "Стажёр"]),
        (25, 2, ["Staff Drummer", "専属太鼓手", "Штатный барабанщик"]),
        (100, 3, ["Overtime Pay", "残業代", "Сверхурочные"]),
        (500, 4, ["Lives at Work", "職場に住む", "Живёт на работе"]),
    ];
    for (i, (need, tier, n)) in hours.into_iter().enumerate() {
        let about = say(&format!("{need} h of playing"), &format!("プレイ時間{need}時間"), &format!("{need} ч в игре"));
        v.push(medal(format!("hours_{need}"), PATH, tier, say(n[0], n[1], n[2]), about, stepped("clock", i + 1), Rule::Hours { need }));
    }
    let streaks = [
        (3, 1, ["Habit", "習慣", "Привычка"]),
        (7, 2, ["Full Week", "一週間", "Полная неделя"]),
        (14, 2, ["Fortnight", "二週間", "Две недели"]),
        (30, 3, ["No Days Off", "休みなし", "Месяц без выходных"]),
        (100, 4, ["The Hundred", "百日", "Сотня"]),
    ];
    for (i, (need, tier, n)) in streaks.into_iter().enumerate() {
        let about = say(&format!("A play every day, {need} days in a row"), &format!("{need}日連続でプレイ"), &format!("Дней подряд с плеем: {need}"));
        v.push(medal(format!("streak_{need}"), PATH, tier, say(n[0], n[1], n[2]), about, stepped("calendar", i + 1), Rule::Days { need, filter: Filter::default() }));
    }
    v.push(medal("binge", PATH, 2, say("Drumroll Day", "連打の日", "Барабанная дробь"), say("30 kept plays in a day", "1日に30プレイを記録", "30 сохранённых плеев за сутки"), picture("fastroll"), Rule::InADay { need: 30 }));
    let long = |notes: f64| Rule::Play(Filter { notes: Span::from(notes), ..clean(0.90, 0.0) });
    v.push(medal("marathon", PATH, 1, say("Marathon", "マラソン", "Марафонец"), say("A map of 2,000 notes at 90%+", "2000ノーツ以上を精度90%以上で", "Карта на 2000+ нот с точностью 90%+"), picture("shoe"), long(2000.0)));
    v.push(medal("ultramarathon", PATH, 3, say("Ultramarathon", "ウルトラマラソン", "Ультрамарафон"), say("A map of 5,000 notes at 90%+", "5000ノーツ以上を精度90%以上で", "Карта на 5000+ нот с точностью 90%+"), picture("road"), long(5000.0)));
    v.push(medal("n_rolls", PATH, 2, say("Drumroll", "連打", "Дробь"), say("5,000 drumroll hits", "連打5000回", "5 000 ударов по дробям"), picture("rollbar"), Rule::Rolls { need: 5000 }));

    // The mod collection.
    let collection = [
        (3, 1, ["Curious", "好奇心", "Любопытный"]),
        (10, 2, ["Experimenter", "実験家", "Экспериментатор"]),
        (25, 3, ["Naturalist", "博物学者", "Естествоиспытатель"]),
        (50, 3, ["Senior Researcher", "主任研究員", "Старший научный сотрудник"]),
        (74, 4, ["The Whole Catalogue", "全カタログ", "Весь каталог"]),
    ];
    for (i, (need, tier, n)) in collection.into_iter().enumerate() {
        let about = say(&format!("{need} of 74 mods in your collection"), &format!("74モード中{need}個を収集"), &format!("{need} из 74 модов в коллекции"));
        v.push(medal(format!("coll_{need}"), COLLECTION, tier, say(n[0], n[1], n[2]), about, stepped("cards", i + 1), Rule::Collection { need }));
    }
    let groups = [
        ("group_reduction", 2, ["No Pride", "プライドなし", "Без лишней гордости"], "feather"),
        ("group_increase", 3, ["Crank It Up", "全開", "Выкрутить на максимум"], "flame"),
        ("group_conversion", 2, ["Rewired", "配線変更", "Перепрошивка"], "swap"),
        ("group_scroll", 3, ["Hand on the Throttle", "スロットルを握る", "Рука на рычаге"], "lever"),
        ("group_svlab", 3, ["Lab Rat", "実験台", "Подопытный"], "flask"),
        ("group_visual", 3, ["Seeing Things", "幻覚", "Обман зрения"], "eye"),
        ("group_rails", 3, ["Train Driver", "運転士", "Машинист"], "train"),
        ("group_playfield", 3, ["Load-Bearing Lane", "解体工事", "Снос несущих стен"], "wreck"),
    ];
    for ((id, tier, n, art), (group, mods)) in groups.into_iter().zip(GROUPS) {
        let about = say(&format!("Every {group} mod"), &format!("{group}のモードをすべて"), &format!("Все моды группы {group}"));
        v.push(medal(id, COLLECTION, tier, say(n[0], n[1], n[2]), about, picture(art), Rule::Group { mods: mods.iter().map(|m| m.to_string()).collect() }));
    }

    // Dan: from real taiko ranks.
    let dans = [
        (2, 1, "Gokyu 五級", "五級", "Гокю 五級"),
        (3, 1, "Sankyu 三級", "三級", "Санкю 三級"),
        (4, 1, "Ikkyu 一級", "一級", "Иккю 一級"),
        (5, 1, "Shodan 初段", "初段", "Сёдан 初段"),
        (6, 1, "Nidan 二段", "二段", "Нидан 二段"),
        (7, 2, "Sandan 三段", "三段", "Сандан 三段"),
        (8, 2, "Yondan 四段", "四段", "Ёндан 四段"),
        (9, 4, "Godan 五段", "五段", "Годан 五段"),
        (10, 4, "Kuroto 玄人", "玄人", "Куродо 玄人"),
        (11, 4, "Meijin 名人", "名人", "Мэйдзин 名人"),
    ];
    for (stars, tier, en, ja, ru) in dans {
        let about = say(&format!("{stars}★ or harder at 90%+"), &format!("★{stars}以上を精度90%以上で"), &format!("{stars}★+, точность 90%+"));
        v.push(medal(format!("dan_{stars}"), DAN, tier, say(en, ja, ru), about, dan_art(ja), Rule::Play(clean(0.90, f64::from(stars)))));
    }

    // Skill.
    let combos = [
        (3, 1, ["Clean Run", "ノーミス", "Без осечек"]),
        (5, 1, ["Steel Nerves", "鋼の神経", "Стальные нервы"]),
        (7, 2, ["Iron Discipline", "鉄の規律", "Железная дисциплина"]),
        (8, 3, ["No Step Back", "一歩も引かず", "Ни шагу назад"]),
        (9, 4, ["Untouchable", "無敵", "Неприкасаемый"]),
    ];
    for (i, (stars, tier, n)) in combos.into_iter().enumerate() {
        let about = say(&format!("Full combo at {stars}★+"), &format!("★{stars}以上でフルコンボ"), &format!("Фулл комбо на {stars}★+"));
        v.push(medal(format!("fc_{stars}"), SKILL, tier, say(n[0], n[1], n[2]), about, stepped("shield", i + 1), Rule::Play(full_combo(f64::from(stars)))));
    }
    let accuracies = [
        (95, 6, 1, ["In the Pocket", "リズムに乗る", "В ритме"]),
        (97, 7, 2, ["Swiss Watch", "スイス時計", "Швейцарские часы"]),
        (98, 9, 3, ["Metronome", "メトロノーム", "Метроном"]),
        (99, 9, 4, ["Atomic Clock", "原子時計", "Атомные часы"]),
    ];
    for (i, (accuracy, stars, tier, n)) in accuracies.into_iter().enumerate() {
        let about = say(&format!("{accuracy}% at {stars}★+"), &format!("★{stars}以上で精度{accuracy}%"), &format!("{accuracy}% на {stars}★+"));
        v.push(medal(format!("acc_{accuracy}"), SKILL, tier, say(n[0], n[1], n[2]), about, stepped("target", i + 1), Rule::Play(clean(f64::from(accuracy) / 100.0, f64::from(stars)))));
    }
    let perfect = [
        (3, 2, ["Flawless", "完璧", "Идеал"]),
        (5, 3, ["Immaculate", "無欠", "Безупречно"]),
        (7, 4, ["Absolute", "絶対", "Абсолют"]),
    ];
    for (i, (stars, tier, n)) in perfect.into_iter().enumerate() {
        let about = say(&format!("100% at {stars}★+"), &format!("★{stars}以上で精度100%"), &format!("100% на {stars}★+"));
        v.push(medal(format!("ss_{stars}"), SKILL, tier, say(n[0], n[1], n[2]), about, stepped("crown", i + 1), Rule::Play(clean(0.99999, f64::from(stars)))));
    }
    let chains = [
        (500, 1, ["Rolling", "勢い", "Разгон"]),
        (1000, 1, ["One Thousand", "千", "Тысяча"]),
        (2000, 2, ["Unbroken", "途切れず", "Не сбиваясь"]),
        (3000, 3, ["Endless Roll", "終わらない連打", "Бесконечная дробь"]),
        (5000, 4, ["Perpetual Motion", "永久機関", "Вечный двигатель"]),
    ];
    for (i, (need, tier, n)) in chains.into_iter().enumerate() {
        let about = say(&format!("A combo of {need}"), &format!("{need}コンボ"), &format!("Комбо {need}"));
        let filter = Filter { clean: true, combo: Span::from(need as f64), ..Filter::default() };
        v.push(medal(format!("combo_{need}"), SKILL, tier, say(n[0], n[1], n[2]), about, stepped("chain", i + 1), Rule::Play(filter)));
    }
    v.push(medal("rate_2", SKILL, 2, say("Double Dose", "二倍量", "Двойная доза"), say("2.0x at 4★+, 90%+", "2.0倍速で★4以上を精度90%以上", "2.0x на 4★+, 90%+"), picture("double"), Rule::Play(Filter { rate: Span::from(1.995), ..clean(0.90, 4.0) })));
    v.push(medal("slowmo", SKILL, 1, say("Slow Motion", "スローモーション", "Замедленная съёмка"), say("0.5x at 90%+", "0.5倍速で精度90%以上", "0.5x, 90%+"), picture("snail"), Rule::Play(Filter { rate: Span::to(0.505), ..clean(0.90, 0.0) })));
    v.push(medal("scroll_fast", SKILL, 3, say("Lightspeed", "光速", "Гиперпрыжок"), say("Scroll speed 3.0+ at 5★+, 90%+", "スクロール速度3.0以上で★5以上を精度90%以上", "Скорость скролла 3.0+, 5★+, 90%+"), picture("rocket"), Rule::Play(Filter { scroll: Span::from(3.0), ..clean(0.90, 5.0) })));
    v.push(medal("scroll_slow", SKILL, 3, say("Slow Reader", "亀の歩み", "Черепаший шаг"), say("Scroll speed 0.5 or less at 5★+, 90%+", "スクロール速度0.5以下で★5以上を精度90%以上", "Скорость скролла 0.5 и ниже, 5★+, 90%+"), picture("turtle"), Rule::Play(Filter { scroll: Span::to(0.5), ..clean(0.90, 5.0) })));

    // Mod trials: each made for how its mod works.
    let combo = |mods: &[&str], stars: f64| with(full_combo(stars), mods);
    let at = |mods: &[&str], accuracy: f64, stars: f64| with(clean(accuracy, stars), mods);
    v.push(trial("t_cracked", 3, ["Not a Scratch", "ひび一つなく", "Ни трещины"], "Cracked", combo(&["CK"], 5.0), "crack"));
    let mut ruins = trial("t_crumble", 3, ["Ruins", "廃墟", "Руины"], "Crumble", Filter { combo: Span::from(1000.0), ..at(&["CR"], 0.90, 5.0) }, "ruin");
    ruins.about = say("Crumble: a combo of 1,000 at 5★+, 90%+", "Crumble: ★5以上で1000コンボ、精度90%以上", "Crumble: комбо 1000+, 90%+ на 5★+");
    v.push(ruins);
    v.push(trial("t_shatter", 3, ["Pieced Together", "破片を集めて", "Целое из осколков"], "Shatter", combo(&["SH"], 5.0), "shard"));
    v.push(trial("t_portal", 3, ["Now You're Thinking", "ポータルで考えろ", "Думай порталами"], "Portal", at(&["PT"], 0.95, 5.0), "portal"));
    v.push(trial("t_mv", 3, ["Eight Windows", "八つの窓", "Восемь окон"], "Multiverse", at(&["MV"], 0.90, 6.0), "windows"));
    v.push(trial("t_mv_fc", 4, ["Every Universe at Once", "全宇宙同時", "Все вселенные сразу"], "Multiverse", combo(&["MV"], 6.0), "galaxy"));
    v.push(trial("t_shapeshift", 3, ["Shapeshifter", "変身", "Оборотень"], "Shapeshift", at(&["SS"], 0.90, 6.0), "moon"));
    let rails = GROUPS[6].1.iter().filter(|m| **m != "XF").map(|m| m.to_string()).collect();
    v.push(medal("t_rails", TRIALS, 3, say("Roller Coaster", "ジェットコースター", "Американские горки"), say("4 different rails at 90%+, 5★+", "4種類のレールを★5以上・精度90%以上で", "4 разные рельсы с 90%+ на 5★+"), picture("coaster"), Rule::Variety { need: 4, mods: rails, filter: clean(0.90, 5.0) }));
    v.push(trial("t_mirrorswap", 3, ["Through the Looking Glass", "鏡の国", "Зазеркалье"], "Mirror + Swap", at(&["MR", "SW"], 0.85, 5.0), "mirror"));
    v.push(trial("t_seasick", 3, ["Seasick", "船酔い", "Морская болезнь"], "Spin + Tilt + Barrel Roll + Flip Flop", at(&["SN", "TL", "BA", "FF"], 0.85, 4.0), "boat"));
    v.push(trial("t_crossfire", 3, ["Crossfire", "十字砲火", "Перекрёстный огонь"], "Crossfire", at(&["XF"], 0.90, 4.0), "sights"));
    v.push(trial("t_memory", 3, ["By Heart", "暗譜", "Наизусть"], "Memory", at(&["MM"], 0.90, 3.0), "head"));
    v.push(trial("t_blind", 4, ["Blind Drummer", "盲目の太鼓手", "Слепой барабанщик"], "Memory + Colorless", at(&["MM", "CL"], 0.85, 3.0), "cane"));
    v.push(trial("t_colorless", 3, ["Colourblind", "色なし", "Дальтоник"], "Colorless", combo(&["CL"], 5.0), "halves"));
    v.push(trial("t_even", 2, ["By Ear", "耳で叩け", "На слух"], "Even", at(&["EV"], 0.90, 5.0), "ear"));
    v.push(trial("t_blackout", 2, ["Power Cut", "停電", "Пробки выбило"], "Blackout", at(&["BO"], 0.95, 5.0), "plug"));
    v.push(trial("t_radar", 3, ["Submarine", "潜水艦", "Подлодка"], "Radar", at(&["RA"], 0.90, 5.0), "periscope"));
    v.push(trial("t_echo", 3, ["Sight-Reader", "初見", "С листа"], "Echo", at(&["EC"], 0.90, 5.0), "sheet"));
    v.push(trial("t_trust", 3, ["Trust Issues", "誰を信じる", "Кому верить?"], "Liar + Decoy", at(&["LI", "DY"], 0.90, 5.0), "mask"));
    v.push(trial("t_lag", 3, ["Ping 999", "ピン999", "Пинг 999"], "Lag Spike", combo(&["LG"], 5.0), "bars"));
    let mut panic = trial("t_panic", 3, ["Don't Panic", "慌てるな", "Без паники"], "Panic", Filter { miss: Span::from(15.0), ..at(&["PN"], 0.85, 5.0) }, "panic");
    panic.about = say("Panic: 15+ misses and still 85% at 5★+", "Panic: 15ミス以上でも★5以上で精度85%以上", "Panic: 15+ промахов, но всё ещё 85%+ на 5★+");
    v.push(panic);
    v.push(trial("t_mercy", 3, ["Asked for No Mercy", "情け無用", "Пощады не просил"], "Mercy", combo(&["MC"], 6.0), "broken_heart"));
    let mut life = trial("t_sd", 2, ["One Life", "一度きりの命", "Одна жизнь"], "Sudden Death", at(&["SD"], 0.0, 6.0), "heart");
    life.about = say("Sudden Death: pass at 6★+", "Sudden Death: ★6以上をクリア", "Sudden Death: пройти 6★+");
    v.push(life);
    let mut only = trial("t_pf", 3, ["Nothing but Perfect", "完璧のみ", "Только идеально"], "Perfect", at(&["PF"], 0.0, 4.0), "star");
    only.about = say("Perfect: pass at 4★+", "Perfect: ★4以上をクリア", "Perfect: пройти 4★+");
    v.push(only);
    v.push(trial("t_gn", 2, ["All or Nothing", "全か無か", "Всё или ничего"], "Great or Nothing", at(&["GN"], 0.90, 6.0), "coin"));
    v.push(trial("t_fl", 3, ["Dead Battery", "電池切れ", "Фонарик сел"], "Flashlight", combo(&["FL"], 6.0), "flashlight"));
    v.push(trial("t_hdhr", 2, ["The Classics", "王道", "Классика жанра"], "Hidden + Hard Rock", at(&["HD", "HR"], 0.90, 7.0), "guitar"));
    v.push(trial("t_works", 4, ["The Works", "全部のせ", "Полный фарш"], "HD + HR + DT + FL", at(&["HD", "HR", "DT", "FL"], 0.90, 6.0), "burger"));
    v.push(trial("t_party", 2, ["Party Animal", "パーティーの主役", "Душа вечеринки"], "Party", at(&["PY"], 0.90, 5.0), "disco"));
    v.push(trial("t_moonwalk", 2, ["Moonwalker", "ムーンウォーク", "Лунная походка"], "Moonwalk", at(&["MW"], 0.90, 5.0), "moon"));
    v.push(trial("t_roulette", 3, ["Russian Roulette", "ロシアンルーレット", "Русская рулетка"], "Roulette", combo(&["RL"], 5.0), "revolver"));
    v.push(trial("t_chaos", 2, ["Chaos Tamed", "手なずけた混沌", "Хаос приручён"], "SV Chaos", at(&["CH"], 0.90, 6.0), "spiral"));
    let sink = Filter { mod_count: Span::from(6.0), ..clean(0.90, 4.0) };
    v.push(medal("t_sink", TRIALS, 3, say("Kitchen Sink", "ごちゃまぜ", "Винегрет"), say("6+ mods at once at 4★+, 90%+", "6モード以上を同時に★4以上で精度90%以上", "6+ модов разом, 4★+, 90%+"), picture("bowl"), Rule::Play(sink)));

    // Growth.
    v.push(medal("g_crutch", GROWTH, 2, say("No Crutches", "補助輪なし", "Без костылей"), say("Beat a map at 90%+ without the NF, EZ, HT, DC or HA you first played it with", "NF/EZ/HT/DC/HAで遊んだ譜面をそれらなしで精度90%以上", "Карта, пройденная с NF/EZ/HT/DC/HA, потом без них на 90%+"), picture("crutch"), Rule::Weaned { need: 1 }));
    v.push(medal("g_crutch5", GROWTH, 3, say("Stronger", "強くなった", "Стал сильнее"), say("The same on 5 maps", "同じことを5譜面で", "То же самое на 5 разных картах"), picture("arm"), Rule::Weaned { need: 5 }));
    v.push(medal("g_ladder", GROWTH, 3, say("Speed Ladder", "速度の階段", "Лесенка"), say("One map under 1x, at 1x and at 1.4x+, all at 90%+", "同じ譜面を1倍未満・1倍・1.4倍以上で精度90%以上", "Одна карта на <1x, 1x и 1.4x+ с 90%+"), picture("stairs"), Rule::SpeedLadder));
    v.push(medal("g_rematch", GROWTH, 2, say("Rematch", "リベンジ", "Реванш"), say("Back at a map after 7+ days, 3% better", "7日以上あとに同じ譜面で精度を3%更新", "Вернуться к карте через 7+ дней и поднять точность на 3%+"), picture("glove"), Rule::Rematch));

    // Rivals.
    v.push(medal("r_snipe", RIVALS, 2, say("Sniper", "狙撃手", "Снайпер"), say("Take first place on a map from someone", "他のプレイヤーから譜面の1位を奪う", "Забрать первое место на карте у другого игрока"), stepped("scope", 1), Rule::Snipes { need: 1 }));
    v.push(medal("r_snipe10", RIVALS, 3, say("Headhunter", "賞金稼ぎ", "Охотник за головами"), say("Take 10 first places", "1位を10回奪う", "Забрать 10 первых мест"), picture("skull"), Rule::Snipes { need: 10 }));
    v.push(medal("r_hill", RIVALS, 2, say("King of the Hill", "山の王", "Царь горы"), say("First place on a map with 3+ players", "3人以上いる譜面で1位", "Первое место на карте, где 3+ игрока"), picture("peak"), Rule::Hills { need: 1 }));
    v.push(medal("r_hill10", RIVALS, 3, say("Lord of the Range", "山脈の主", "Хозяин хребта"), say("10 such first places", "そのような1位を10回", "10 таких первых мест"), picture("range"), Rule::Hills { need: 10 }));
    let mut founder = medal("r_founder", RIVALS, 2, say("Founder", "創設メンバー", "Основатель"), say("A play before December 2026", "2026年12月より前にプレイ", "Сохранить плей до 1 декабря 2026"), picture("flag"), Rule::Before { time: 1_796_083_200 });
    founder.quiet = true;
    v.push(founder);

    // Together: counted on the server as it happens.
    let count = |event: &str, need| Rule::Count { event: event.to_string(), need };
    v.push(medal("v_chat", TOGETHER, 1, say("Hello, Server", "こんにちは、サーバー", "Привет, сервер"), say("Say something in the global chat", "全体チャットで発言", "Первое сообщение в общем чате"), picture("bubble"), count("chat", 1)));
    v.push(medal("v_avatar", TOGETHER, 1, say("Face of the Band", "バンドの顔", "Своё лицо"), say("Set a picture", "アバターを設定", "Поставить аватар"), picture("face"), count("avatar", 1)));
    v.push(medal("v_discord", TOGETHER, 1, say("Linked Up", "連携", "Связка"), say("Link your Discord", "Discordを連携", "Привязать Discord"), picture("link"), count("discord", 1)));
    v.push(medal("v_friend", TOGETHER, 1, say("Not Alone", "ひとりじゃない", "Не один"), say("Add your first friend", "初めてのフレンド", "Добавить первого друга"), picture("pair"), count("friends", 1)));
    v.push(medal("v_friends5", TOGETHER, 2, say("The Band", "バンド", "Своя банда"), say("5 friends", "フレンド5人", "5 друзей"), picture("band"), count("friends", 5)));
    v.push(medal("v_lobby", TOGETHER, 1, say("Live Set", "ライブ", "Живой звук"), say("Play a match in a lobby", "ロビーで対戦", "Сыграть матч в лобби"), picture("speaker"), count("matches", 1)));
    v.push(medal("v_win10", TOGETHER, 2, say("Lobby Terror", "ロビーの脅威", "Гроза лобби"), say("Win 10 lobby matches", "ロビーで10勝", "10 побед в лобби"), stepped("cup", 1), count("wins", 10)));
    v.push(medal("v_win50", TOGETHER, 3, say("Headliner", "ヘッドライナー", "Хедлайнер"), say("Win 50 lobby matches", "ロビーで50勝", "50 побед в лобби"), stepped("cup", 2), count("wins", 50)));
    v.push(medal("v_watch", TOGETHER, 1, say("Front Row", "最前列", "Первый ряд"), say("Watch a friend play live", "フレンドのプレイを観戦", "Посмотреть живой плей друга"), picture("ticket"), count("watched", 1)));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_medal_has_a_unique_id_a_picture_and_all_its_text() {
        let list = all();
        let ids: HashSet<&str> = list.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids.len(), list.len(), "two medals with one id");
        assert_eq!(list.len(), 118);
        for m in &list {
            assert_eq!(m.art.len(), ART, "{}", m.id);
            assert!(m.art.iter().all(|r| r.chars().count() == ART && r.chars().all(|c| ".#xo".contains(c))), "{}: {:?}", m.id, m.art);
            assert!(m.art.iter().any(|r| r.contains(['#', 'x'])), "{} has an empty picture", m.id);
            assert!(m.name.iter().chain(&m.about).all(|t| !t.is_empty()), "{}", m.id);
            assert!((1..=4).contains(&m.tier), "{}", m.id);
            assert!(usize::from(m.section) < SECTIONS.len(), "{}", m.id);
            assert!(!m.secret, "{} would be in the public source", m.id);
        }
    }

    #[test]
    fn the_collection_is_every_mod_but_autoplay_and_relax() {
        let mods = collectable();
        assert_eq!(mods.len(), 74);
        assert!(!mods.contains("AT") && !mods.contains("RX"));
    }

    #[test]
    fn mod_trials_only_ask_for_mods_that_can_be_on_together() {
        // Their acronyms are the game's: the collection has every one of them.
        let mods = collectable();
        for m in all() {
            if let Rule::Play(f) = &m.rule {
                assert!(f.mods.iter().all(|a| mods.contains(a.as_str())), "{}: {:?}", m.id, f.mods);
            }
        }
    }

    #[test]
    fn a_ladder_shows_its_step() {
        assert_eq!(stepped("sticks", 1)[ART - 1], ".......x........");
        assert_eq!(stepped("sticks", 3)[ART - 1], ".....x.x.x......");
    }

    #[test]
    fn numbers_read_right() {
        assert_eq!(thousands(2_000_000, ','), "2,000,000");
        assert_eq!(thousands(500, ' '), "500");
    }
}
