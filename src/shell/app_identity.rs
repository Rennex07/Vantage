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

/// Preserve cycle order across changes in stacking.
pub fn cycle_window<T: Clone + PartialEq>(
    order: &mut Vec<T>,
    candidates: &[T],
    focused: Option<&T>,
) -> Option<T> {
    order.retain(|item| candidates.contains(item));
    for item in candidates {
        if !order.contains(item) {
            order.push(item.clone());
        }
    }
    if order.is_empty() {
        return None;
    }
    match focused.and_then(|window| order.iter().position(|item| item == window)) {
        Some(index) => Some(order[(index + 1) % order.len()].clone()),
        None => candidates.first().cloned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dock_cycle_survives_restacking_and_window_removal() {
        let mut order = Vec::new();
        assert_eq!(cycle_window(&mut order, &[1, 2, 3], None), Some(1));
        assert_eq!(cycle_window(&mut order, &[1, 2, 3], Some(&1)), Some(2));
        assert_eq!(cycle_window(&mut order, &[2, 1, 3], Some(&2)), Some(3));
        assert_eq!(cycle_window(&mut order, &[3, 1], Some(&3)), Some(1));
        assert_eq!(cycle_window(&mut order, &[4], Some(&1)), Some(4));
        assert_eq!(cycle_window(&mut order, &[], Some(&4)), None);
    }

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
