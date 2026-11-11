//! Checks that compiled catalog keys still match calls in the client.

use std::collections::BTreeSet;
use std::path::Path;

use proc_macro2::{TokenStream, TokenTree};

fn add_source_keys(dir: &Path, keys: &mut BTreeSet<(String, String)>) {
    for entry in dir.read_dir().expect("source directory") {
        let path = entry.expect("source entry").path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name != "regression") {
                add_source_keys(&path, keys);
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let source = std::fs::read_to_string(&path).expect("Rust source");
            let tokens = source
                .parse()
                .unwrap_or_else(|e| panic!("cannot tokenize {}: {e}", path.display()));
            add_keys(tokens, keys);
        }
    }
}

fn add_keys(tokens: TokenStream, keys: &mut BTreeSet<(String, String)>) {
    let mut tokens = tokens.into_iter().peekable();
    while let Some(token) = tokens.next() {
        match token {
            TokenTree::Group(group) => add_keys(group.stream(), keys),
            TokenTree::Ident(name)
                if name == "tr"
                    && matches!(tokens.peek(), Some(TokenTree::Punct(p)) if p.as_char() == '!') =>
            {
                tokens.next();
                if let Some(TokenTree::Group(group)) = tokens.next() {
                    let args = group.stream();
                    if let Some(key) = catalog_key(args.clone()) {
                        keys.insert(key);
                    }
                    add_keys(args, keys);
                }
            }
            _ => {}
        }
    }
}

fn catalog_key(args: TokenStream) -> Option<(String, String)> {
    let mut args = args.into_iter();
    let (context, literal) = match args.next()? {
        TokenTree::Literal(literal) => (String::new(), literal),
        TokenTree::Ident(context) => {
            let TokenTree::Punct(separator) = args.next()? else {
                return None;
            };
            let TokenTree::Literal(literal) = args.next()? else {
                return None;
            };
            if separator.as_char() != ':' {
                return None;
            }
            (context.to_string(), literal)
        }
        _ => return None,
    };
    let english = syn::parse_str::<syn::LitStr>(&literal.to_string())
        .ok()?
        .value();
    Some((context, english))
}

#[test]
fn each_catalog_key_matches_a_client_translation_call() {
    let mut keys = BTreeSet::new();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for source in ["src", "crates/taiko-proto/src"] {
        add_source_keys(&root.join(source), &mut keys);
    }
    let missing: Vec<_> = crate::lang::entries(crate::lang::Lang::ZhCn)
        .iter()
        .filter(|&&(context, english, _)| {
            !keys.contains(&(context.to_string(), english.to_string()))
        })
        .map(|&(context, english, _)| format!("{context}\t{english:?}"))
        .collect();
    assert!(
        missing.is_empty(),
        "unused catalog keys:\n{}",
        missing.join("\n")
    );
}

#[test]
fn positional_arguments_are_used_by_the_catalog_for_legacy_calls() {
    crate::lang::with(crate::lang::Lang::ZhCn, || {
        assert_eq!(
            tr!("fps cap: {}", "FPS: {}", "FPS: {}"; 120),
            "帧率上限：120"
        );
    });
}
