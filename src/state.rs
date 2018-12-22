//! An in-memory representation of a hoard.
//!
//! Handles conversion from both JSON manifests and the filesystem.
//!
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use serde_json;
use walkdir::WalkDir;

use error::ResultExt;
use hoard::{self, FileHash};
use Result;

fn resolve(desire: &State, actual: &State, index: &Index) -> Vec<Change> {
    use self::ChangeType::*;

    let desire_keys: BTreeSet<_> = desire.inner.keys().collect();
    let actual_keys: BTreeSet<_> = actual.inner.keys().collect();
    let nameset: BTreeSet<_> = desire_keys.intersection(&actual_keys).collect();

    let mut objects_by_name: HashMap<&str, &Object> = HashMap::new();
    for (name, object) in index.by_name().iter() {
        if nameset.contains(&(&(**name).to_string())) {
            objects_by_name.insert(name.clone(), object.clone());
        }
    }

    let mut changes: BTreeSet<Change> = BTreeSet::new();
    changes.extend(desire.to_changeset(&objects_by_name, |o| Create(o)));
    changes.extend(actual.to_changeset(&objects_by_name, |o| Delete(o)));
    changes.extend(actual.extra.iter().map(|path| Change {
        _path: path.clone(),
        _type: Ignore,
    }));

    // need to clean up changes (cancel out, create modify, etc)
    let length = changes.len();
    let vector = Vec::with_capacity(length);
    let result = changes
        .into_iter()
        .fold(vector, |mut changes: Vec<Change>, curr| {
            if let Some(prev) = changes.pop() {
                if &prev._path == &curr._path {
                    match (&prev._type, &curr._type) {
                        (Create(ref o1), Delete(ref o2)) if o1 != o2 => {
                            changes.push(Change {
                                _path: prev._path,
                                _type: Modify(o1.clone(), o2.clone()),
                            });
                        }
                        (Ignore, _) => {
                            changes.push(prev.clone());
                        }
                        _ => {}
                    }
                } else {
                    changes.push(prev);
                    changes.push(curr);
                }
            } else {
                changes.push(curr);
            }
            changes
        });

    result
}

/// Represents an entry in the index.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Object {
    path: PathBuf,
    hash: FileHash,
    name: String,
    ino: u64,
}

impl Object {
    pub fn ino(&self) -> &u64 {
        &self.ino
    }

    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn hash(&self) -> &FileHash {
        &self.hash
    }
}

/// An in-memory represenation of the contents of the '.hoard/objects'
/// folder. Its purpose is generally for looking up objects based on
/// hash or inode.
pub struct Index {
    pub(crate) created: Vec<Object>,
    pub(crate) deleted: Vec<Object>,
    pub(crate) objects: Vec<Object>,
}

impl Index {
    /// Builds an index using the given path as the hoard root.
    pub fn from<P: AsRef<Path>>(root: P) -> Result<Self> {
        let path_by_name = root.as_ref().join(".hoard/objects/by-name");

        let mut objects = vec![];
        for entry in WalkDir::new(&path_by_name)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().read_link().is_ok())
        {
            let name = entry
                .path()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string();
            let path = entry.path().canonicalize().with_path(entry.path())?;

            if !path.exists() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "Symlink does not lead to object",
                )
                .into());
            }

            let hash = FileHash::from_path(&path).with_path(&path)?;
            let ino = path.metadata()?.ino();
            objects.push(Object {
                path,
                hash,
                name,
                ino,
            });
        }

        Ok(Index {
            created: vec![],
            deleted: vec![],
            objects,
        })
    }

    pub fn by_ino(&self) -> HashMap<&u64, &Object> {
        self.objects
            .iter()
            .map(|object| (object.ino(), object))
            .collect()
    }

    pub fn by_name(&self) -> HashMap<&str, &Object> {
        self.objects
            .iter()
            .map(|object| (object.name(), object))
            .collect()
    }

    pub fn by_hash(&self) -> HashMap<&FileHash, &Object> {
        self.objects
            .iter()
            .map(|object| (object.hash(), object))
            .collect()
    }
}

/// The types of change that can be executed.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChangeType {
    Ignore,
    Create(Object),
    Delete(Object),
    Modify(Object, Object),
}

/// An executable change that produces side-effects in the filesystem.
///
/// This is produced as the result of resolving two `State`s, the
/// expected `State` given by a manifest file and the actual `State`
/// of the filesystem.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Change {
    _path: PathBuf,
    _type: ChangeType,
}

impl Change {
    pub fn execute(self) -> Result<()> {
        use self::ChangeType::*;
        match self._type {
            Ignore => {}
            Delete(_) => {
                fs::remove_file(&self._path)?;
            }
            Create(src) => {
                fs::hard_link(src.path(), &self._path)?;
            }
            Modify(_old, new) => {
                hoard::link(new.path(), &self._path)?;
            }
        };
        Ok(())
    }
}

/// An in-memory representation of the hoard.
///
/// This can be produced from a file manifest, allowing the user
/// to specify what state they would like the hoard to take.
///
/// Additionally, it can also be produced from the actual state of
/// the hoard on the filesystem.
///
/// By producing states from both sources, we can then compare and
/// produce a list of changes that should be taken to achieve the
/// desired state specified by the user.
pub struct State {
    pub(crate) inner: BTreeMap<String, BTreeSet<PathBuf>>,
    pub(crate) extra: BTreeSet<PathBuf>,
}

impl State {
    /// Builds a State according to a manifest.
    /// The manifest is in the format of the following:
    ///
    /// ```json
    /// {
    ///   "item-name-1": [
    ///     "path1/item-name-1",
    ///     "path2/item-name-1"
    ///   ],
    ///   "item-name-2": [
    ///     "path1/item-name-2",
    ///     "path3/item-name-2"
    ///   ]
    /// }
    /// ```
    fn from_file<P>(path: P, names: &HashSet<String>) -> Result<Self>
    where
        P: AsRef<Path>,
    {
        let mut state = State {
            inner: BTreeMap::new(),
            extra: BTreeSet::new(),
        };

        let file = fs::File::open(&path)?;
        let manifest: HashMap<_, _> = serde_json::from_reader(file)?;

        for (name, paths) in manifest.into_iter() {
            if names.contains(&name) {
                state.inner.insert(name, paths);
            } else {
                println!("No such object '{}'", name);
            }
        }

        {
            let mut dupes = BTreeMap::new();
            for (name, paths) in state.inner.iter() {
                for path in paths {
                    dupes.entry(path).or_insert(BTreeSet::new()).insert(name);
                }
            }
            dupes = dupes.into_iter().filter(|(_k, v)| v.len() > 1).collect();
            if !dupes.is_empty() {
                bail!("duplicate paths for entries: {:#?}", dupes);
            }
        }

        Ok(state)
    }

    /// Builds a State from the filesystem.
    /// Such a filesystem might look like the following.
    ///
    /// ```bash
    /// $ tree
    /// .
    /// ├── path1
    /// │   ├── item-name-1
    /// │   └── item-name-2
    /// ├── path2
    /// │   └── item-name-1
    /// └── path3
    ///     └── item-name-2
    ///
    /// 3 directories, 4 files
    /// ```
    fn from_path<P>(path: P, index: &Index) -> Result<Self>
    where
        P: AsRef<Path>,
    {
        let mut inner = BTreeMap::new();
        let mut extra = BTreeSet::new();

        for entry in WalkDir::new(&path.as_ref())
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
        {
            let path = entry.path();
            let ino = path.metadata()?.ino();
            if let Some(object) = index.by_ino().get(&ino) {
                inner
                    .entry(object.name().to_string())
                    .or_insert(BTreeSet::new())
                    .insert(path.to_path_buf());
            } else {
                extra.insert(path.to_path_buf());
            }
        }

        Ok(State { inner, extra })
    }

    fn to_changeset<F>(&self, objects: &HashMap<&str, &Object>, mut func: F) -> BTreeSet<Change>
    where
        F: FnMut(Object) -> ChangeType,
    {
        let mut changes = BTreeSet::new();
        for (name, paths) in self.inner.iter() {
            if let Some(object) = objects.get(name.as_str()) {
                for path in paths.iter() {
                    changes.insert(Change {
                        _path: path.clone(),
                        _type: func((*object).clone()),
                    });
                }
            }
        }
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_execute_ignore() {
        let arg1 = "test/res/change_execute/ignore/test1.json";

        let change = Change {
            _path: PathBuf::from(arg1),
            _type: ChangeType::Ignore,
        };

        let result = change.execute();

        assert!(result.is_ok());
    }

    #[test]
    fn change_execute_create() {
        panic!("Unimplemented");
    }

    #[test]
    fn change_execute_delete() {
        panic!("Unimplemented");
    }

    #[test]
    fn change_execute_modify() {
        panic!("Unimplemented");
    }

    #[test]
    fn index_from_success() {
        let arg1 = "test/res/index_from/success";

        let result = Index::from(arg1).unwrap();

        assert!(result.created.is_empty());
        assert!(result.deleted.is_empty());
        assert_eq!(result.objects.len(), 1);
    }

    #[test]
    fn index_from_invalid() {
        let arg1 = "test/res/index_from/invalid";

        let result = Index::from(arg1);

        assert!(result.is_err());
    }

    #[test]
    fn state_from_file_duplicates() {
        let arg1 = "test/res/state_from_file/duplicates/test1.json";
        let arg2: HashSet<_, _> = [
            "item1".to_string(),
            "item2".to_string(),
            "item3".to_string(),
            "item4".to_string(),
        ]
        .iter()
        .cloned()
        .collect();

        let result = State::from_file(arg1, &arg2);

        assert!(result.is_err());
    }

    #[test]
    fn state_from_file_invalid() {
        let arg1 = "test/res/state_from_file/invalid/test1.json";
        let arg2 = HashSet::new();

        let result = State::from_file(arg1, &arg2);

        assert!(result.is_err());
    }

    #[test]
    fn state_from_file_no_objects() {
        let arg1 = "test/res/state_from_file/no_objects/test1.json";
        let arg2 = HashSet::new();

        let result = State::from_file(arg1, &arg2).unwrap();

        assert!(result.inner.is_empty());
    }

    #[test]
    fn state_from_file_success() {
        let arg1 = "test/res/state_from_file/success/test1.json";
        let arg2: HashSet<_, _> = [
            "item1".to_string(),
            "item2".to_string(),
            "item3".to_string(),
            "item4".to_string(),
        ]
        .iter()
        .cloned()
        .collect();

        let result = State::from_file(arg1, &arg2).unwrap();

        assert_eq!(result.inner.len(), 4);
    }

    #[test]
    fn state_from_path_extra() {
        let arg1 = "test/res/state_from_path/extra";
        let arg2 = Index {
            created: vec![],
            deleted: vec![],
            objects: vec![],
        };

        let result = State::from_path(arg1, &arg2).unwrap();

        assert_eq!(result.inner.len(), 0);
        assert_eq!(result.extra.len(), 4);
    }

    #[test]
    fn state_from_path_empty() {
        let arg1 = "test/res/state_from_path/empty";
        let arg2 = Index {
            created: vec![],
            deleted: vec![],
            objects: vec![],
        };

        let result = State::from_path(arg1, &arg2).unwrap();

        assert!(result.inner.is_empty());
        assert!(result.extra.is_empty());
    }

    #[test]
    fn state_from_path_success() {
        let arg1 = "test/res/state_from_path/success";
        let arg2 = Index::from(arg1).expect("Invalid hoard repository");

        let result = State::from_path(arg1, &arg2).unwrap();

        assert_eq!(result.inner.len(), 1);
        assert_eq!(result.extra.len(), 2);
    }
}
