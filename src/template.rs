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
///
/// Fields not present in the template are appended in key order; prefer
/// [`render_template_ordered`] to control their order.
pub fn render_template(
    fm: Value,
    body: &str,
    fields: &HashMap<String, Value>,
    placeholders: &HashMap<String, String>,
) -> (Value, String) {
    let mut fields: Vec<(String, Value)> =
        fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    fields.sort_by(|a, b| a.0.cmp(&b.0));
    let placeholders: Vec<(&str, &str)> = placeholders
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    render_template_ordered(fm, body, &fields, &placeholders)
}

/// Render a template with ordered fields: keys already in the template keep
/// their position, new keys are appended in the given order. Body
/// placeholders look like `{{ name }}`.
///
/// A template whose frontmatter is not a mapping (e.g. empty) is replaced by
/// a mapping of `fields`, so the result is always a usable entity.
pub fn render_template_ordered(
    fm: Value,
    body: &str,
    fields: &[(String, Value)],
    placeholders: &[(&str, &str)],
) -> (Value, String) {
    let mut map = match fm {
        Value::Mapping(m) => m,
        _ => serde_yaml::Mapping::new(),
    };
    for (key, value) in fields {
        map.insert(Value::String(key.clone()), value.clone());
    }

    let mut rendered_body = body.to_string();
    for (key, value) in placeholders {
        for pattern in [format!("{{{{ {} }}}}", key), format!("{{{{{}}}}}", key)] {
            rendered_body = rendered_body.replace(&pattern, value);
        }
    }

    (Value::Mapping(map), rendered_body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_keeps_template_order_and_appends_new_fields() {
        let fm: Value = serde_yaml::from_str("id: X\nname: ''\nstatus: a").unwrap();
        let fields = vec![
            ("status".to_string(), Value::String("b".into())),
            ("zeta".to_string(), Value::String("z".into())),
            ("alpha".to_string(), Value::String("a".into())),
            ("id".to_string(), Value::String("T-1".into())),
        ];
        let (out, body) = render_template_ordered(
            fm,
            "# {{ name }} {{id}}",
            &fields,
            &[("name", "N"), ("id", "T-1")],
        );
        let keys: Vec<&str> = out
            .as_mapping()
            .unwrap()
            .keys()
            .map(|k| k.as_str().unwrap())
            .collect();
        assert_eq!(keys, vec!["id", "name", "status", "zeta", "alpha"]);
        assert_eq!(frontmatter::get_str(&out, "status"), Some("b"));
        assert_eq!(body, "# N T-1");
    }

    #[test]
    fn test_render_non_mapping_template() {
        let fields = vec![("id".to_string(), Value::String("T-1".into()))];
        let (out, _) = render_template_ordered(Value::Null, "", &fields, &[]);
        assert_eq!(frontmatter::get_str(&out, "id"), Some("T-1"));
    }

    #[test]
    fn test_legacy_render_template() {
        let fm: Value = serde_yaml::from_str("id: X").unwrap();
        let mut fields = HashMap::new();
        fields.insert("id".to_string(), Value::String("C-1".into()));
        let mut ph = HashMap::new();
        ph.insert("name".to_string(), "Acme".to_string());
        let (out, body) = render_template(fm, "# {{ name }}", &fields, &ph);
        assert_eq!(frontmatter::get_str(&out, "id"), Some("C-1"));
        assert_eq!(body, "# Acme");
    }
}
