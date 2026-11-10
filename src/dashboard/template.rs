use std::collections::{HashMap, HashSet};
use std::fmt::{self, Display};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fragment(String);

impl Fragment {
    pub fn lit(value: &'static str) -> Self {
        Self(value.to_owned())
    }

    pub fn empty() -> Self {
        Self(String::new())
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl Display for Fragment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromIterator<Fragment> for Fragment {
    fn from_iter<T: IntoIterator<Item = Fragment>>(iter: T) -> Self {
        let mut value = String::new();
        for fragment in iter {
            value.push_str(&fragment.0);
        }
        Self(value)
    }
}

pub enum Slot {
    Text(String),
    Html(Fragment),
}

pub fn text(value: impl Display) -> Slot {
    Slot::Text(value.to_string())
}

pub fn html(value: Fragment) -> Slot {
    Slot::Html(value)
}

pub struct Template {
    name: &'static str,
    src: &'static str,
}

impl Template {
    pub const fn new(name: &'static str, src: &'static str) -> Self {
        Self { name, src }
    }

    pub fn fill(&self, slots: &[(&str, Slot)]) -> Fragment {
        let source = self.normalized_source();
        let mut names = HashSet::new();
        let mut provided = HashMap::new();
        for (name, slot) in slots {
            debug_assert!(
                valid_name(name),
                "{}: invalid slot name {name:?}",
                self.name
            );
            names.insert((*name).to_owned());
            provided.insert(*name, slot);
        }

        let mut template_names = HashSet::new();
        let mut rest = source.as_str();
        while let Some(start) = rest.find("{{") {
            rest = &rest[start + 2..];
            let Some(end) = rest.find("}}") else {
                debug_assert!(false, "{}: unclosed slot", self.name);
                break;
            };
            let name = &rest[..end];
            debug_assert!(
                valid_name(name),
                "{}: invalid slot name {name:?}",
                self.name
            );
            template_names.insert(name.to_owned());
            rest = &rest[end + 2..];
        }
        debug_assert_eq!(
            template_names, names,
            "{}: template slots and supplied slots differ",
            self.name
        );
        debug_assert_eq!(
            provided.len(),
            slots.len(),
            "{}: duplicate supplied slot",
            self.name
        );

        let mut out = String::with_capacity(source.len());
        let mut rest = source.as_str();
        while let Some(start) = rest.find("{{") {
            out.push_str(&rest[..start]);
            rest = &rest[start + 2..];
            let Some(end) = rest.find("}}") else {
                out.push_str("{{");
                out.push_str(rest);
                return Fragment(out);
            };
            let name = &rest[..end];
            if let Some(slot) = provided.get(name) {
                match slot {
                    Slot::Text(value) => out.push_str(&escape(value)),
                    Slot::Html(fragment) => out.push_str(&fragment.0),
                }
            }
            rest = &rest[end + 2..];
        }
        out.push_str(rest);
        Fragment(out)
    }

    fn normalized_source(&self) -> String {
        let mut out = String::with_capacity(self.src.len());
        for line in self.src.lines() {
            out.push_str(line.trim_start_matches([' ', '\t']));
        }
        out
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
}

fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Template, text};

    #[test]
    fn escapes_text_slots_once() {
        const TEMPLATE: Template = Template::new("escape-test", "<p>{{value}}</p>");
        let rendered = TEMPLATE.fill(&[("value", text("<&>\"'&amp;"))]);
        assert_eq!(
            rendered.to_string(),
            "<p>&lt;&amp;&gt;&quot;&#39;&amp;amp;</p>"
        );
    }

    #[test]
    fn collapses_template_indentation_not_slot_text() {
        const TEMPLATE: Template =
            Template::new("whitespace-test", "  <p>\n    {{value}}\n  </p>\n");
        let rendered = TEMPLATE.fill(&[("value", text(" first\n  second "))]);
        assert_eq!(rendered.to_string(), "<p> first\n  second </p>");
    }

    #[test]
    #[should_panic(expected = "template slots and supplied slots differ")]
    fn slot_mismatch_panics_in_debug() {
        const TEMPLATE: Template = Template::new("mismatch-test", "<p>{{expected}}</p>");
        let _ = TEMPLATE.fill(&[("actual", text("value"))]);
    }
}
