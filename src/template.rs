use crate::error::{McError, McResult};
use crate::frontmatter;
use serde_yaml::Value;
use std::collections::HashMap;
use std::path::Path;

/// Load a template file and return its parsed frontmatter and body.
pub fn load_template(templates_dir: &Path, name: &str) -> McResult<(Value, String)> {
    let path = templates_dir.join(format!("{}.md", name));
    if !path.is_file() {
        return Err(McError::TemplateNotFound(path));
    }
    frontmatter::parse_file(&path)
}

/// Render a template: overwrite frontmatter fields from context, replace body placeholders.
pub fn render_template(
    mut fm: Value,
    body: &str,
    fields: &HashMap<String, Value>,
    placeholders: &HashMap<String, String>,
) -> (Value, String) {
    // Overwrite frontmatter fields
    if let Some(map) = fm.as_mapping_mut() {
        for (key, value) in fields {
            map.insert(Value::String(key.clone()), value.clone());
        }
    }

    // Replace body placeholders like {{ name }}, {{ title }}
    let mut rendered_body = body.to_string();
    for (key, value) in placeholders {
        let pattern = format!("{{{{ {} }}}}", key);
        rendered_body = rendered_body.replace(&pattern, value);
    }

    (fm, rendered_body)
}
