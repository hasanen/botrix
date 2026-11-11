//! The docs site is built by Jekyll, which runs Liquid over every page before
//! rendering it. Text such as `{{secret:TOKEN}}` looks like a Liquid output tag
//! and is silently replaced with nothing, so the published page shows
//! `Authorization: "Bearer "`. Pages that contain such text must wrap it in
//! `{% raw %}` ... `{% endraw %}`.

use std::fs;
use std::path::Path;

const RAW_OPEN: &str = "{% raw %}";
const RAW_CLOSE: &str = "{% endraw %}";

/// Byte offsets of Liquid delimiters (`{{` and `{%`) in `text`, ignoring the
/// raw tags themselves.
fn liquid_delimiters(text: &str) -> Vec<usize> {
    text.match_indices("{{")
        .chain(text.match_indices("{%"))
        .map(|(index, _)| index)
        .filter(|&index| {
            !text[index..].starts_with(RAW_OPEN) && !text[index..].starts_with(RAW_CLOSE)
        })
        .collect()
}

/// Whether every Liquid delimiter in `text` sits between a raw open and a raw
/// close tag. Text without any delimiters is trivially safe.
fn is_liquid_safe(text: &str) -> bool {
    let delimiters = liquid_delimiters(text);
    if delimiters.is_empty() {
        return true;
    }
    match (text.find(RAW_OPEN), text.rfind(RAW_CLOSE)) {
        (Some(open), Some(close)) => delimiters.iter().all(|&at| open < at && at < close),
        _ => false,
    }
}

#[test]
fn text_without_liquid_syntax_is_safe() {
    assert!(is_liquid_safe("Plain ${variable} text and `code`."));
}

#[test]
fn unwrapped_placeholders_are_not_safe() {
    assert!(!is_liquid_safe(
        "Authorization: \"Bearer {{secret:TOKEN}}\""
    ));
    assert!(!is_liquid_safe("{% if x %}y{% endif %}"));
}

#[test]
fn placeholders_inside_a_raw_block_are_safe() {
    let text = "{% raw %}\nBearer {{secret:TOKEN}}\n{% endraw %}\n";
    assert!(is_liquid_safe(text));
}

#[test]
fn placeholders_outside_the_raw_block_are_not_safe() {
    assert!(!is_liquid_safe(
        "{{secret:A}}\n{% raw %}\nplain\n{% endraw %}\n"
    ));
    assert!(!is_liquid_safe(
        "{% raw %}\nplain\n{% endraw %}\n{{secret:A}}\n"
    ));
}

#[test]
fn an_unclosed_raw_block_is_not_safe() {
    assert!(!is_liquid_safe("{% raw %}\nBearer {{secret:TOKEN}}\n"));
}

#[test]
fn every_docs_page_with_liquid_syntax_is_wrapped_in_raw_tags() {
    // Arrange
    let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
    let mut with_placeholders = Vec::new();
    let mut unsafe_pages = Vec::new();

    // Act
    for entry in fs::read_dir(&docs).expect("docs directory exists") {
        let path = entry.expect("readable entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = fs::read_to_string(&path).expect("readable page");
        if !liquid_delimiters(&text).is_empty() {
            with_placeholders.push(name.clone());
        }
        if !is_liquid_safe(&text) {
            unsafe_pages.push(name);
        }
    }

    // Assert
    assert!(
        unsafe_pages.is_empty(),
        "these pages use `{{{{` or `{{%` outside `{{% raw %}}` ... `{{% endraw %}}`, so Jekyll \
         would drop the text when building the site: {unsafe_pages:?}"
    );
    // The trigger pages document `{{secret:NAME}}`, so the check must find them.
    assert!(
        with_placeholders.len() >= 2,
        "expected to find the pages that document secret placeholders, found {with_placeholders:?}"
    );
}
