//! 404 and 500 pages.

use crate::html::components::{empty_state, link_button, page_header, Btn};
use crate::html::format::{capitalize, escape_html, href_with};
use crate::html::layout::layout;
use crate::html::Page;

/// Render a 500 error page.
pub fn error_page(page: &Page, message: &str) -> String {
    let body = format!(
        "{}{}",
        page_header("This page couldn't be loaded", "", ""),
        empty_state(
            &format!(
                r#"<code class="error-message">{}</code>"#,
                escape_html(message)
            ),
            "Check the file for invalid frontmatter, then reload.",
            &link_button("/", "Go to overview", Btn::Secondary, ""),
        )
    );
    layout(page, "Error", "", "", &body)
}

/// Render a 404 page. For a missing entity, Search looks for its ID.
pub fn not_found_page(page: &Page, path: &str) -> String {
    let (hint, search) = match path.strip_prefix("/entity/") {
        Some(id) => (
            "The entity may have been renamed or removed.",
            href_with("/search", &[("q", id)]),
        ),
        None => (
            "This page doesn't exist in this repo. Use the sidebar or search to find what you need.",
            "/search".to_string(),
        ),
    };
    let body = format!(
        "{}{}",
        page_header("Not found", "", ""),
        empty_state(
            &format!("Nothing at <code>{}</code>", escape_html(path)),
            hint,
            &format!(
                "{}{}",
                link_button(&search, "Search", Btn::Secondary, ""),
                link_button("/", "Go to overview", Btn::Ghost, "")
            ),
        )
    );
    layout(page, "Not found", "", "", &body)
}

/// Render the 404 page for a kind this repo doesn't enable, with the
/// message and hint of [`crate::error::McError::not_available`].
pub fn not_available_page(page: &Page, path: &str, message: &str, hint: &str) -> String {
    let body = format!(
        "{}{}",
        page_header("Not available", "", ""),
        empty_state(
            &format!(
                "{}. Nothing at <code>{}</code>",
                code_spans(&capitalize(message)),
                escape_html(path)
            ),
            &code_spans(hint),
            &link_button("/", "Go to overview", Btn::Ghost, ""),
        )
    );
    layout(page, "Not available", "", "", &body)
}

/// Escape `text`, showing its `backticked` parts as code.
fn code_spans(text: &str) -> String {
    text.split('`')
        .enumerate()
        .map(|(i, part)| match i % 2 {
            1 => format!("<code>{}</code>", escape_html(part)),
            _ => escape_html(part),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::catalog::tests::{catalog, test_config};

    #[test]
    fn error_pages_escape_input() {
        let (_d, cfg) = test_config();
        let cat = catalog(Vec::new(), &cfg);
        let page = Page::new(&cfg, &cat, "");
        let html = not_found_page(&page, "/entity/<x>");
        assert!(html.contains("Nothing at <code>/entity/&lt;x&gt;</code>"));
        assert!(html.contains(r#"href="/search""#));
        let html = error_page(&page, "bad <yaml>");
        assert!(html.contains("bad &lt;yaml&gt;"));
        // A missing entity's Search looks for its ID.
        assert!(not_found_page(&page, "/entity/task-99").contains(r#"href="/search?q=task-99""#));
        let html = not_available_page(
            &page,
            "/proposals",
            "proposals are off (`paths:`)",
            "Add `x`.",
        );
        assert!(html.contains("Proposals are off (<code>paths:</code>)."));
        assert!(html.contains("Add <code>x</code>."));
    }
}
