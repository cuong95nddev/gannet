//! The library's trash: deleted captures stay, with their files, until
//! they are deleted for good, by hand or once they have been in it long
//! enough; until then each can be put back in the folder it was in.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trashed {
    pub id: String,
    /// When it was put in the trash, in ms since the epoch.
    pub at_ms: i64,
    /// The folder it was in, by name, to go back to.
    pub folder: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Trash {
    /// The latest put in first.
    pub items: Vec<Trashed>,
}

impl Trash {
    pub fn contains(&self, id: &str) -> bool {
        self.items.iter().any(|t| t.id == id)
    }

    /// Put capture `id` in, from folder `folder`; false if it is in already.
    pub fn add(&mut self, id: &str, at_ms: i64, folder: Option<String>) -> bool {
        if self.contains(id) {
            return false;
        }
        self.items.insert(0, Trashed { id: id.to_owned(), at_ms, folder });
        true
    }

    /// Take capture `id` out, to restore it or delete it for good.
    pub fn take(&mut self, id: &str) -> Option<Trashed> {
        let i = self.items.iter().position(|t| t.id == id)?;
        Some(self.items.remove(i))
    }

    /// The captures in it more than `max_age_ms` before `now_ms`.
    pub fn expired(&self, max_age_ms: i64, now_ms: i64) -> Vec<String> {
        self.items.iter().filter(|t| now_ms - t.at_ms > max_age_ms).map(|t| t.id.clone()).collect()
    }

    /// Forget the captures not in `ids` (gone). False if none were.
    pub fn keep_only(&mut self, ids: &HashSet<&str>) -> bool {
        let before = self.items.len();
        self.items.retain(|t| ids.contains(t.id.as_str()));
        self.items.len() != before
    }

    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("trash serializes")
    }

    pub fn from_json(bytes: &[u8]) -> Result<Trash, String> {
        serde_json::from_slice(bytes).map_err(|e| format!("can't read the trash: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_go_in_and_come_out() {
        let mut t = Trash::default();
        assert!(t.add("a", 10, Some("Work".into())));
        assert!(!t.add("a", 20, None), "in already");
        assert!(t.add("b", 30, None));
        assert_eq!(t.items[0].id, "b", "latest first");
        assert_eq!(t.take("a").map(|x| x.folder), Some(Some("Work".to_owned())));
        assert!(!t.contains("a") && t.take("a").is_none());
    }

    #[test]
    fn old_ones_expire_and_gone_ones_are_forgotten() {
        let mut t = Trash::default();
        t.add("old", 0, None);
        t.add("new", 900, None);
        assert_eq!(t.expired(500, 1000), vec!["old".to_owned()]);
        assert!(t.keep_only(&["new"].into_iter().collect()));
        assert_eq!(t.items.len(), 1);
        assert_eq!(Trash::from_json(&t.to_json()).unwrap(), t);
    }
}
