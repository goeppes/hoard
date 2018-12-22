use std::collections::HashSet;
use std::fs;
use std::io;
use std::ops::Deref;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use hex;
use multi_map::MultiMap as TwoKeyMap;
use pathdiff;
use regex::Regex;
use sha2::{Digest, Sha256};
use walkdir::{DirEntry, WalkDir};

use error::ResultExt;
use state::Index;
use Result;

fn is_empty_dir<P: AsRef<Path>>(path: P) -> bool {
    fs::read_dir(path)
        .map(|mut it| it.next().is_none())
        .unwrap_or(false)
}

/// Utility function for linking files
/// if both exists & have same inode
///   return
/// if dst exists & is same as src
///   remove dst
/// link src
///
/// Instead of returning unit, should return what action was taken:
/// linking, creating, etc
pub fn link<S, D>(src: S, dst: D) -> Result<bool>
where
    S: AsRef<Path>,
    D: AsRef<Path>,
{
    _link(src.as_ref(), dst.as_ref())
}

fn _link(src: &Path, dst: &Path) -> Result<bool> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).with_path(parent)?;
    }

    if !dst.exists() {
        fs::hard_link(src, dst)?;
        return Ok(true);
    }

    let src_ino = src.metadata().with_path(src)?.ino();
    let dst_ino = dst.metadata().with_path(dst)?.ino();

    if src_ino != dst_ino {
        fs::remove_file(dst)?;
        fs::hard_link(src, dst)?;
    }

    Ok(false)
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FileHash(String);

impl FileHash {
    pub fn of<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut file = fs::File::open(&path).with_path(&path)?;
        let mut hasher = Sha256::new();
        io::copy(&mut file, &mut hasher).with_path(&path)?;
        let hash = hasher.result();
        Ok(FileHash(hex::encode(&hash[..])))
    }

    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let name = path.to_string_lossy().to_string();
        FileHash::from_str(&name).or_else(|_| FileHash::of(path))
    }

    pub fn as_path(&self) -> String {
        format!("{}/{}", &self.0[0..2], &self.0[2..])
    }

    pub fn from_str(hash_str: &String) -> Result<Self> {
        lazy_static! {
            static ref RE: Regex = Regex::new("^[a-f0-9]{64}$").unwrap();
        }
        if RE.is_match(hash_str) {
            Ok(FileHash(hash_str.clone()))
        } else {
            bail!("The input string is not a valid SHA256 hash");
        }
    }
}

impl Deref for FileHash {
    type Target = String;

    fn deref(&self) -> &String {
        &self.0
    }
}

pub struct FileObject {
    path: PathBuf,
    hash: FileHash,
}

impl FileObject {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let parts = path.components().rev().take(2).collect::<Vec<_>>();
        let object = parts[0].as_os_str().to_string_lossy();
        let prefix = parts[1].as_os_str().to_string_lossy();
        let hash_str = format!("{}{}", prefix, object);
        Ok(FileObject {
            path: path.to_path_buf(),
            hash: FileHash::from_str(&hash_str)?,
        })
    }

    pub fn ino(&self) -> Result<u64> {
        Ok(self.path.metadata().with_path(&self.path)?.ino())
    }

    pub fn name(&self) -> String {
        "name".to_string()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn hash(&self) -> &FileHash {
        &self.hash
    }
}

// TODO: consider caching approach instead of eagerly loading.
struct ObjectStore {
    path: PathBuf,
    objects: TwoKeyMap<u64, FileHash, FileObject>,
}

impl ObjectStore {
    /// Loads an ObjectDatabase from on disk from the given path of
    /// a hoard repository.
    fn new<P: AsRef<Path>>(root: P) -> Result<Self> {
        let path = root.as_ref().join(".hoard/objects");
        let mut objects = TwoKeyMap::new();

        for entry in WalkDir::new(&path) {
            let entry = entry?;
            if !entry.path().is_file() {
                continue;
            }
            let object = FileObject::new(entry.path())?;
            objects.insert(object.ino()?, object.hash().clone(), object);
        }

        Ok(ObjectStore { path, objects })
    }

    /// Internal function
    fn get_by_ino(&self, ino: &u64) -> Option<&FileObject> {
        self.objects.get(ino)
    }

    /// Internal function
    fn get_by_hash(&self, hash: &FileHash) -> Option<&FileObject> {
        self.objects.get_alt(hash)
    }

    /// Puts an object that matches the file at path.
    ///
    /// If there is no such object currently in the store, then it will
    /// be created.
    fn put<P: AsRef<Path>>(&mut self, path: P) -> Result<FileHash> {
        let path = path.as_ref();

        let ino = path.metadata().with_path(&path)?.ino();
        if let Some(object) = self.get_by_ino(&ino) {
            return Ok(object.hash().clone());
        }

        let hash = FileHash::of(path)?;
        if let Some(_) = self.get_by_hash(&hash) {
            return Ok(hash);
        }

        let src = path;
        let dst = self.path.join(hash.as_path());

        link(&src, &dst)?;

        let object = FileObject::new(dst)?;
        self.objects
            .insert(object.ino()?, object.hash().clone(), object);

        Ok(hash)
    }
}

pub struct Repository {
    root: PathBuf,
}

impl Repository {
    /// Creates a new repository at the given path.
    pub fn init<P: AsRef<Path>>(path: P) -> Result<()> {
        Repository::_init(path.as_ref())
    }

    fn _init(path: &Path) -> Result<()> {
        fs::create_dir_all(path.join(".hoard/objects/by-hash"))?;
        fs::create_dir_all(path.join(".hoard/objects/by-name"))?;
        Ok(())
    }

    /// Opens an existing repository at the given path.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut root = path.as_ref().canonicalize()?.to_path_buf();

        // Dummy component due to how the loop logic works.
        // TODO: come up with something more elegant.
        root.push("fake");

        loop {
            if !root.pop() {
                break;
            }
            if root.join(".hoard").is_dir() {
                return Ok(Repository { root });
            }
        }

        bail!("No hoard repository found")
    }

    // change this to take a vector instead of a single argument?
    pub fn add<P: AsRef<Path>>(&mut self, paths: Vec<P>) -> Result<()> {
        // get index and state?

        // check if file already is in hoard
        // if it is, then just add the path
        // if it is not, then add a new object entry
        // check if path is below root
        // MULTI-THREADING?

        let mut results = Vec::new();
        for path in paths {
            self._expand(&mut results, path.as_ref())?;
        }
        println!("{:#?}", results);

        let index = Index::from(&self.root)?;
        for path in results {
            // if path inode is in index, ignore
            // if path hash is in index (and is same file), delete and link
            // if path hash is not in index, add to index
        }

        Ok(())
    }

    fn _expand(&self, results: &mut Vec<PathBuf>, path: &Path) -> Result<()> {
        fn is_index(entry: &DirEntry) -> bool {
            entry
                .file_name()
                .to_str()
                .map(|s| s == ".hoard")
                .unwrap_or(false)
        }

        let path = path.canonicalize()?;
        if !path.starts_with(&self.root) {
            bail!(
                "pathspec is not inside of hoard repository: {}",
                path.display()
            )
        }

        for entry in WalkDir::new(path)
            .into_iter()
            .filter_entry(|e| !is_index(e))
        {
            let entry = entry?;
            if entry.path().is_file() {
                results.push(entry.path().to_path_buf());
            }
        }
        Ok(())
    }

    /*
    fn _add_file(&mut self, path: &Path) -> Result<()> {
        let relative_path = pathdiff::diff_paths(&path, &self.root).unwrap();

        let created = link(object.path(), path)?;
        if created {
            println!("create: {}", path.display());
        }
        Ok(())
    }
    */

    pub fn apply(&self) -> Result<()> {
        let mut inodes: HashSet<u64> = HashSet::new();
        let mut paths: HashSet<PathBuf> = HashSet::new();

        let index = self.root.join(".hoard/");
        for entry in WalkDir::new(&self.root).contents_first(true) {
            let entry = entry?;

            if entry.path().starts_with(&index) {
                continue;
            }

            if inodes.contains(&entry.path().metadata()?.ino()) {
                if !paths.contains(entry.path()) {
                    fs::remove_file(entry.path())?;
                    println!("delete: {}", entry.path().display());
                }
            }

            if is_empty_dir(entry.path()) {
                fs::remove_dir(entry.path())?;
                println!("delete: {}/", entry.path().display());
            }
        }

        Ok(())
    }
}
