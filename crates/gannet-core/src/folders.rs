//! Folders the user sorts captures into, kept in the library beside them.
//! Every capture is in one: a new one goes in the folder named for the day
//! it was taken (made if there is none), and can be moved from there.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Folder {
    pub id: u64,
    pub name: String,
    /// The captures in it, by id, the latest put in first.
    pub captures: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Folders {
    pub list: Vec<Folder>,
    /// Goes up on every change made through these methods, so a view can
    /// tell its layout of them is still right without comparing them all.
    #[serde(skip)]
    rev: Rev,
}

/// A count of changes, never part of what folders are: two lists of the
/// same folders are equal whatever their counts.
#[derive(Debug, Clone, Copy, Default)]
struct Rev(u64);

impl PartialEq for Rev {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Folders {
    /// How many changes have been made (see `rev`).
    pub fn rev(&self) -> u64 {
        self.rev.0
    }

    fn changed(&mut self) {
        self.rev.0 += 1;
    }

    /// A new, empty folder at the top; its id.
    pub fn add(&mut self, name: &str) -> u64 {
        let id = self.list.iter().map(|f| f.id).max().map_or(1, |n| n + 1);
        self.list.insert(0, Folder { id, name: name.to_owned(), captures: Vec::new() });
        self.changed();
        id
    }

    /// A name no folder has yet: "New folder", then "New folder 2"…
    pub fn new_name(&self) -> String {
        let taken = |n: &str| self.list.iter().any(|f| f.name == n);
        let mut name = "New folder".to_owned();
        let mut n = 2;
        while taken(&name) {
            name = format!("New folder {n}");
            n += 1;
        }
        name
    }

    /// Name folder `id` `name`, trimmed; an empty name is ignored.
    pub fn rename(&mut self, id: u64, name: &str) -> bool {
        let name = name.trim();
        match self.list.iter_mut().find(|f| f.id == id) {
            Some(f) if !name.is_empty() && f.name != name => {
                f.name = name.to_owned();
                self.rev.0 += 1;
                true
            },
            _ => false,
        }
    }

    /// Take folder `id` away; its captures are in none after it, to be
    /// deleted with it (or filed by day again).
    pub fn remove(&mut self, id: u64) {
        self.list.retain(|f| f.id != id);
        self.changed();
    }

    pub fn get(&self, id: u64) -> Option<&Folder> {
        self.list.iter().find(|f| f.id == id)
    }

    /// The folder capture `id` is in.
    pub fn folder_of(&self, capture: &str) -> Option<u64> {
        self.list.iter().find(|f| f.captures.iter().any(|c| c == capture)).map(|f| f.id)
    }

    /// Every capture filed, with the folder it is in: for looking many up
    /// at once, where [`Self::folder_of`] would scan for each.
    pub fn index(&self) -> HashMap<&str, u64> {
        self.list.iter().flat_map(|f| f.captures.iter().map(move |c| (c.as_str(), f.id))).collect()
    }

    /// [`Self::file`] each of `captures` (id, day) in turn, in one pass.
    /// Whether any was filed.
    pub fn file_all<'a>(&mut self, captures: impl IntoIterator<Item = (&'a str, String)>) -> bool {
        let filed: HashSet<String> = self.list.iter().flat_map(|f| f.captures.iter().cloned()).collect();
        let mut changed = false;
        for (capture, day) in captures {
            if filed.contains(capture) {
                continue;
            }
            let at = match self.list.iter().position(|f| f.name == day) {
                Some(i) => i,
                None => {
                    self.add(&day);
                    0
                },
            };
            let f = &mut self.list[at];
            if !f.captures.iter().any(|c| c == capture) {
                f.captures.insert(0, capture.to_owned());
                changed = true;
            }
        }
        if changed {
            self.changed();
        }
        changed
    }

    /// Put capture `capture` in folder `to`, or in none; out of any other.
    /// False if nothing changed.
    pub fn put(&mut self, capture: &str, to: Option<u64>) -> bool {
        if self.folder_of(capture) == to || to.is_some_and(|id| self.get(id).is_none()) {
            return false;
        }
        for f in &mut self.list {
            f.captures.retain(|c| c != capture);
        }
        if let Some(f) = to.and_then(|id| self.list.iter_mut().find(|f| f.id == id)) {
            f.captures.insert(0, capture.to_owned());
        }
        self.changed();
        true
    }

    /// Put capture `capture`, if it is in no folder, in the one named
    /// `day`, made at the top if there is none. False if it was in one.
    pub fn file(&mut self, capture: &str, day: &str) -> bool {
        if self.folder_of(capture).is_some() {
            return false;
        }
        let id = match self.list.iter().find(|f| f.name == day) {
            Some(f) => f.id,
            None => self.add(day),
        };
        self.put(capture, Some(id))
    }

    /// Forget the captures not in `ids` (deleted). False if none were.
    pub fn keep_only(&mut self, ids: &HashSet<&str>) -> bool {
        let mut changed = false;
        for f in &mut self.list {
            let before = f.captures.len();
            f.captures.retain(|c| ids.contains(c.as_str()));
            changed |= f.captures.len() != before;
        }
        if changed {
            self.changed();
        }
        changed
    }

    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("folders serialize")
    }

    pub fn from_json(bytes: &[u8]) -> Result<Folders, String> {
        serde_json::from_slice(bytes).map_err(|e| format!("can't read the folders: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_go_in_one_folder_at_most() {
        let mut f = Folders::default();
        let a = f.add("Bugs");
        let b = f.add("Ideas");
        assert_eq!(f.list[0].id, b, "newest on top");
        assert!(f.put("c1", Some(a)));
        assert!(!f.put("c1", Some(a)), "already there");
        assert!(f.put("c1", Some(b)));
        assert_eq!((f.folder_of("c1"), f.get(a).unwrap().captures.len()), (Some(b), 0));
        assert!(!f.put("c1", Some(99)), "no such folder");
        assert!(f.put("c1", None));
        assert_eq!(f.folder_of("c1"), None);
    }

    #[test]
    fn names_rename_and_removal() {
        let mut f = Folders::default();
        assert_eq!(f.new_name(), "New folder");
        let a = f.add(&f.new_name());
        assert_eq!(f.new_name(), "New folder 2");
        assert!(f.rename(a, "  Work "));
        assert_eq!(f.get(a).unwrap().name, "Work");
        assert!(!f.rename(a, "   "), "an empty name is ignored");
        f.put("c1", Some(a));
        f.remove(a);
        assert_eq!(f.folder_of("c1"), None);
    }

    #[test]
    fn captures_are_filed_by_day() {
        let mut f = Folders::default();
        let work = f.add("Work");
        f.put("c0", Some(work));
        assert!(f.file("c1", "9 Nov 2026"));
        assert!(f.file("c2", "9 Nov 2026"));
        assert!(!f.file("c0", "9 Nov 2026"), "already in a folder");
        assert_eq!(f.list.len(), 2, "one folder for the day");
        assert_eq!(f.list[0].name, "9 Nov 2026", "made at the top");
        assert_eq!(f.list[0].captures, vec!["c2".to_owned(), "c1".to_owned()]);
    }

    #[test]
    fn deleted_captures_are_forgotten_and_the_file_reads_back() {
        let mut f = Folders::default();
        let a = f.add("A");
        f.put("c1", Some(a));
        f.put("c2", Some(a));
        assert!(f.keep_only(&["c2"].into_iter().collect()));
        assert_eq!(f.get(a).unwrap().captures, vec!["c2".to_owned()]);
        assert_eq!(Folders::from_json(&f.to_json()).unwrap(), f);
        assert!(Folders::from_json(b"nope").is_err());
    }

    #[test]
    fn file_all_matches_file_one_by_one() {
        let mut one = Folders::default();
        one.add("Work");
        one.put("a", Some(1));
        let mut all = one.clone();
        let items = [("b", "Mon".to_owned()), ("a", "Tue".to_owned()), ("c", "Tue".to_owned()), ("d", "Mon".to_owned())];
        for (id, day) in &items {
            one.file(id, day);
        }
        assert!(all.file_all(items.iter().map(|(id, d)| (*id, d.clone()))));
        assert_eq!(all, one);
        assert!(!all.file_all(items.iter().map(|(id, d)| (*id, d.clone()))), "all filed already");
        let index = all.index();
        assert_eq!(index.len(), 4);
        assert_eq!(index.get("a"), Some(&1));
        assert_eq!(all.folder_of("d"), index.get("d").copied());
    }
}
