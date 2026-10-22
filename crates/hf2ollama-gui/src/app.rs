//! Application shell: top bar, downloads dock, status bar and view bodies.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Browse,
    Installed,
    Settings,
}

pub struct AppState {
    pub view: View,
}

impl AppState {
    pub fn new() -> AppState {
        AppState { view: View::Browse }
    }

    pub fn select(&mut self, _view: View) {
        todo!()
    }
}

/// `http://localhost:11434/` becomes `localhost:11434`.
pub fn host_of(_url: &str) -> String {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nav_selection_changes_view_state() {
        let mut s = AppState::new();
        assert_eq!(s.view, View::Browse);
        s.select(View::Installed);
        assert_eq!(s.view, View::Installed);
        s.select(View::Settings);
        assert_eq!(s.view, View::Settings);
        s.select(View::Browse);
        assert_eq!(s.view, View::Browse);
    }

    #[test]
    fn host_of_strips_scheme_and_path() {
        assert_eq!(host_of("http://localhost:11434"), "localhost:11434");
        assert_eq!(host_of("https://box.lan:8443/"), "box.lan:8443");
        assert_eq!(host_of("box:11434/api"), "box:11434");
    }
}
