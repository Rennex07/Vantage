use std::collections::HashSet;

pub fn desktop_id(app_id: &str, installed: &HashSet<String>) -> Option<String> {
    let name = app_id.strip_suffix(".desktop").unwrap_or(app_id);
    if name.is_empty()
        || name.len() > 247
        || name.contains(['/', '\\'])
        || name.chars().any(char::is_control)
    {
        return None;
    }
    let id = format!("{name}.desktop");
    if installed.contains(&id) {
        return Some(id);
    }
    // X11-derived app IDs can differ in case. Ignore ambiguous matches.
    let mut candidates = installed
        .iter()
        .filter(|entry| entry.eq_ignore_ascii_case(&id));
    let entry = candidates.next()?;
    candidates.next().is_none().then(|| entry.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_prefers_exact_desktop_id_and_requires_unique_fallback() {
        let installed = ["org.App.desktop", "org.app.desktop", "org.other.desktop"]
            .map(str::to_owned)
            .into();
        assert_eq!(
            desktop_id("org.App", &installed).as_deref(),
            Some("org.App.desktop")
        );
        assert_eq!(
            desktop_id("ORG.OTHER.desktop", &installed).as_deref(),
            Some("org.other.desktop")
        );
        assert_eq!(desktop_id("ORG.APP", &installed), None);
        assert_eq!(desktop_id("../org.other", &installed), None);
        assert_eq!(desktop_id("missing", &installed), None);
    }
}
