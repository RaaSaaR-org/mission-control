//! 404 and 500 pages.

use crate::html::components::{empty_state, link_button, page_header, Btn};
use crate::html::format::escape_html;
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

/// Render a 404 page.
pub fn not_found_page(page: &Page, path: &str) -> String {
    let body = format!(
        "{}{}",
        page_header("Not found", "", ""),
        empty_state(
            &format!("Nothing at <code>{}</code>", escape_html(path)),
            if path.starts_with("/entity/") {
                "The entity may have been renamed or removed."
            } else {
                "This page doesn't exist in this repo. Use the sidebar or search to find what you need."
            },
            &format!(
                "{}{}",
                link_button("/search", "Search", Btn::Secondary, ""),
                link_button("/", "Go to overview", Btn::Ghost, "")
            ),
        )
    );
    layout(page, "Not found", "", "", &body)
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
    }
}
