//! Prompt text generated from the catalogue.

use crate::common::ok;
use zuse_ui_block::{EXAMPLES, tool_description};

#[test]
fn tool_description_examples_all_parse() {
    let description = tool_description();
    for (title, spec) in EXAMPLES {
        assert!(
            description.contains(&format!("Example — {title}:\n{spec}")),
            "{title}"
        );
        ok(spec);
    }
}
