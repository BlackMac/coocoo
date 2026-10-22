//! Page-tab list of one profile, maintained from `Target.*` discovery events.

use coocoo_proto::BrowserTab;
use serde::Deserialize;

/// `TargetInfo` subset from `Target.targetCreated` / `Target.targetInfoChanged`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TargetInfo {
    pub target_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
}

impl TargetInfo {
    /// User-visible page tab (excludes workers, iframes, DevTools and extension pages).
    fn is_tab(&self) -> bool {
        self.kind == "page" && !self.url.starts_with("devtools://") && !self.url.starts_with("chrome-extension://")
    }
}

#[derive(Debug)]
struct Entry {
    target_id: String,
    url: String,
    title: String,
    /// Logical clock of the last create/navigate/activate; the max is the active tab.
    touched: u64,
}

/// Ordered (creation order) list of page tabs.
#[derive(Debug, Default)]
pub(crate) struct TabList {
    entries: Vec<Entry>,
    clock: u64,
}

impl TabList {
    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    fn position(&self, target_id: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.target_id == target_id)
    }

    pub fn contains(&self, target_id: &str) -> bool {
        self.position(target_id).is_some()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The only tab, when it is still blank — reused by `browser.open` instead of a new renderer.
    pub fn sole_blank_tab(&self) -> Option<&str> {
        match self.entries.as_slice() {
            [only] if only.url.is_empty() || only.url == "about:blank" => Some(&only.target_id),
            _ => None,
        }
    }

    /// `Target.targetCreated` / `Target.targetInfoChanged`. Returns `true` when the list changed.
    /// A new tab or a URL change (navigation) makes the tab active.
    pub fn upsert(&mut self, info: TargetInfo) -> bool {
        let pos = self.position(&info.target_id);
        if !info.is_tab() {
            // A page can turn into DevTools/extension content; drop it then.
            return match pos {
                Some(i) => {
                    self.entries.remove(i);
                    true
                }
                None => false,
            };
        }
        match pos {
            None => {
                let touched = self.tick();
                self.entries.push(Entry { target_id: info.target_id, url: info.url, title: info.title, touched });
                true
            }
            Some(i) => {
                let navigated = self.entries[i].url != info.url;
                let retitled = self.entries[i].title != info.title;
                if navigated {
                    let touched = self.tick();
                    let e = &mut self.entries[i];
                    e.touched = touched;
                    e.url = info.url;
                }
                if retitled {
                    self.entries[i].title = info.title;
                }
                navigated || retitled
            }
        }
    }

    /// `Target.targetDestroyed` / crash. Returns `true` when a tab was removed.
    pub fn remove(&mut self, target_id: &str) -> bool {
        match self.position(target_id) {
            Some(i) => {
                self.entries.remove(i);
                true
            }
            None => false,
        }
    }

    /// Explicit activation (`browser.activate`, `browser.open`). Returns `true` when known.
    pub fn touch(&mut self, target_id: &str) -> bool {
        let touched = self.tick();
        match self.position(target_id) {
            Some(i) => {
                self.entries[i].touched = touched;
                true
            }
            None => false,
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    fn active_clock(&self) -> Option<u64> {
        self.entries.iter().map(|e| e.touched).max()
    }

    fn tab(e: &Entry, active: Option<u64>) -> BrowserTab {
        BrowserTab {
            target_id: e.target_id.clone(),
            url: e.url.clone(),
            title: e.title.clone(),
            active: Some(e.touched) == active,
        }
    }

    pub fn snapshot(&self) -> Vec<BrowserTab> {
        let active = self.active_clock();
        self.entries.iter().map(|e| Self::tab(e, active)).collect()
    }

    pub fn get(&self, target_id: &str) -> Option<BrowserTab> {
        let active = self.active_clock();
        self.entries.iter().find(|e| e.target_id == target_id).map(|e| Self::tab(e, active))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(id: &str, kind: &str, url: &str, title: &str) -> TargetInfo {
        TargetInfo { target_id: id.into(), kind: kind.into(), url: url.into(), title: title.into() }
    }

    fn active(list: &TabList) -> Vec<String> {
        list.snapshot().into_iter().filter(|t| t.active).map(|t| t.target_id).collect()
    }

    #[test]
    fn filters_non_tabs() {
        let mut list = TabList::default();
        assert!(!list.upsert(info("w", "service_worker", "https://a/sw.js", "")));
        assert!(!list.upsert(info("d", "page", "devtools://devtools/inspector.html", "DevTools")));
        assert!(!list.upsert(info("e", "page", "chrome-extension://abc/popup.html", "")));
        assert!(!list.upsert(info("f", "iframe", "https://a/frame", "")));
        assert!(list.upsert(info("p", "page", "about:blank", "")));
        assert_eq!(list.len(), 1);
        assert_eq!(list.sole_blank_tab(), Some("p"));
    }

    #[test]
    fn active_follows_create_navigate_activate() {
        let mut list = TabList::default();
        list.upsert(info("a", "page", "about:blank", ""));
        list.upsert(info("b", "page", "https://b/", "B"));
        assert_eq!(active(&list), ["b"]);
        assert_eq!(list.sole_blank_tab(), None);

        // Title-only change: list changes, active does not.
        assert!(list.upsert(info("a", "page", "about:blank", "Blank")));
        assert_eq!(active(&list), ["b"]);

        // Navigation activates.
        assert!(list.upsert(info("a", "page", "https://a/", "Blank")));
        assert_eq!(active(&list), ["a"]);

        // Unchanged info is not a change.
        assert!(!list.upsert(info("a", "page", "https://a/", "Blank")));

        assert!(list.touch("b"));
        assert_eq!(active(&list), ["b"]);
        assert!(!list.touch("zzz"));

        assert!(list.remove("b"));
        assert!(!list.remove("b"));
        assert_eq!(active(&list), ["a"]);
        assert!(list.get("a").unwrap().active);
    }

    #[test]
    fn page_turning_into_devtools_is_dropped() {
        let mut list = TabList::default();
        list.upsert(info("a", "page", "https://a/", ""));
        assert!(list.upsert(info("a", "page", "devtools://devtools/x", "")));
        assert_eq!(list.len(), 0);
    }

    #[test]
    fn keeps_creation_order() {
        let mut list = TabList::default();
        for id in ["1", "2", "3"] {
            list.upsert(info(id, "page", &format!("https://{id}/"), id));
        }
        list.touch("1");
        let ids: Vec<_> = list.snapshot().into_iter().map(|t| t.target_id).collect();
        assert_eq!(ids, ["1", "2", "3"]);
        assert_eq!(active(&list), ["1"]);
    }
}
