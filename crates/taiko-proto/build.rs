// The interface's translations, ../../assets/lang/<code>.toml, made into a table for each language
// (`lang::entries`): (context, English, translation), sorted, without the ones left empty.
#[cfg(feature = "client-catalog")]
fn lang_tables() {
    use std::fmt::Write;
    println!("cargo:rerun-if-changed=../../assets/lang");
    let mut files: Vec<_> = std::fs::read_dir("../../assets/lang")
        .expect("the translations (../../assets/lang)")
        .map(|e| e.expect("../../assets/lang").path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    files.sort();
    let mut out = String::new();
    for path in files {
        println!("cargo:rerun-if-changed={}", path.display());
        let at = path.display();
        let code = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("a language code");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{at}: {e}"));
        let table: toml::Table = text.parse().unwrap_or_else(|e| panic!("{at}: {e}"));
        let mut entries = Vec::new();
        for (key, value) in table {
            match value {
                toml::Value::String(t) => entries.push((String::new(), key, t)),
                // Translations kept for English the game no longer has (examples/lang_sync.rs).
                toml::Value::Table(_) if key == "unused" => {}
                toml::Value::Table(ctx) => {
                    for (en, t) in ctx {
                        let t = t
                            .as_str()
                            .unwrap_or_else(|| panic!("{at}: [{key}] {en:?}: text"));
                        entries.push((key.clone(), en, t.to_string()));
                    }
                }
                _ => panic!("{at}: {key:?} wants text in quotes or a [context] table"),
            }
        }
        entries.retain(|e| !e.2.is_empty());
        entries.sort();
        writeln!(
            out,
            "pub static {}: &[super::Entry] = &[",
            code.to_uppercase().replace('-', "_")
        )
        .unwrap();
        for (ctx, en, t) in entries {
            writeln!(out, "    ({ctx:?}, {en:?}, {t:?}),").unwrap();
        }
        out.push_str("];\n");
    }
    let dest = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("lang_tables.rs");
    std::fs::write(dest, out).expect("OUT_DIR/lang_tables.rs");
}

fn main() {
    #[cfg(feature = "client-catalog")]
    lang_tables();
}
