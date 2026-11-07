# Translating taiko-rs

The game's text is English in the code, and each other language is a file of translations:

- `assets/lang/<code>.toml`: every piece of text in the interface (`ja.toml`, `ru.toml`, ...).
- `assets/lang/manual/<code>.txt`: the editor's manual (Help > Manual), if the language has one.
  Without one the manual shows `en.txt`.

## For translators

You need only these files. Each line of a `.toml` file is the English, then its translation:

```toml
# src/settings.rs
"Audio offset" = "Desfase de audio"
"Back" = ""
```

- `""` means not translated yet: the game shows the English there. Fill in as many as you like.
  The comment lines (`# src/settings.rs`) say where in the game's code the text is.
- Leave the English (left of the `=`) exactly as it is. It's how the game finds the translation.
- Holes in braces are filled in by the game: `{}` (in order), `{0}` / `{1}` (by position) and
  `{name}`. Keep each of them, in whatever order your sentence wants:
  `"{hits} of {MIN_HITS} hits" = "{hits} de {MIN_HITS} golpes"`. A hole shows a number the way
  the English one does (`{x:.1}` with one decimal), unless you give it its own: `{x:.0}`.
- A word that changes with a count: `{n|form|form}` picks one by the number `n`, which must be one
  of the text's holes. English and Spanish take two forms (one, other), Russian three (one,
  few, many), Japanese one: `"{n} bars" = "{n} {n|compás|compases}"`.
- A `[table]` (`[guide]`, `[music]`, ...) holds text whose English appears somewhere else too but
  wants other words in that place (`[music]` "Play" is the music player's play button).
- `[unused]` keeps translations of English the game doesn't have anymore, usually because the
  English changed a little. Reuse them for the new English and delete them.
- Quotes and backslashes in a text are written `\"` and `\\`, a new line `\n`.

The manual's file is plain text: `=== Title` starts a chapter, `try: ...` right after it is the
label of the chapter's button, and in a chapter `# ` starts a heading, `- ` a bullet, `> ` a
tip. Translate a copy of `assets/lang/manual/en.txt` keeping its chapters in the same order.

## For developers

Text in the interface goes through `tr!` (src/lang.rs):

```rust
tr!("Play")                       // &'static str
tr!(music: "Play")                // the same English with other words: [music] in the files
tr!("{} maps", n)                 // String, format! arguments
tr!("{n} maps"; n)                // variables the text names go after a ;
tr!("{} of {total}", done; total) // both
```

The English is checked like `format!`'s. A `{name}` that isn't after the `;`, or a format spec
the translations can't follow (src/lang.rs `SPECS`), doesn't compile. Translations are checked
by the `lang` tests.

After adding or changing text, run `cargo run --example lang_sync`: it writes every `tr!` text
into each `assets/lang/*.toml` (new ones empty, changed ones' old translations under `[unused]`).
`-- --check` only tells whether the files are up to date.

To add a language:

1. `cargo run --example lang_sync -- --new es` writes `assets/lang/es.toml` with every text empty,
   ready to hand to a translator. A copy of `assets/lang/manual/en.txt` as `es.txt` is the manual's.
2. In src/lang.rs, add `Lang::Es` with its name and code, its tables (`entries`) and its plural
   rule (`plural`, `plural_forms`); in src/editor/view/manual_text.rs, its manual (`file`).
3. The game's font (src/font.rs) draws ASCII, Cyrillic and Japanese only: letters a language needs
   beyond those (á, ñ, ¿ ...) want glyphs there.
