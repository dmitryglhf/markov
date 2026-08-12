use anyhow::{anyhow, Context, Result};
use jsonc_parser::cst::{CstInputValue, CstNode, CstObject, CstObjectProp, CstRootNode};
use jsonc_parser::ParseOptions;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// A settings file edited in place.
///
/// Zed and VS Code store JSONC — comments, trailing commas — so the file is
/// parsed into a lossless tree and printed back out. Everything we do not touch
/// survives byte for byte, which is the whole reason this is not string
/// splicing: a Windows path like C:\Users\ivan\markov\markov.exe has to be
/// escaped by something that knows JSON, not by us.
pub struct Document {
    path: PathBuf,
    root: CstRootNode,
    existed: bool,
}

impl Document {
    /// An empty document that is never saved, used to render the snippet
    /// `--print` shows. Going through the same tree as a real edit keeps one
    /// definition of what an entry looks like.
    pub fn scratch() -> Self {
        Self {
            path: PathBuf::from("<snippet>"),
            root: CstRootNode::parse("{}\n", &ParseOptions::default())
                .expect("an empty object parses"),
            existed: false,
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let (text, existed) = match fs::read_to_string(path) {
            Ok(text) => (text, true),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => (String::new(), false),
            Err(err) => {
                return Err(err).with_context(|| format!("could not read {}", path.display()))
            }
        };

        // A file that exists but holds nothing is normal; JSON parsing of it is not.
        let text = if text.trim().is_empty() {
            "{}\n".to_string()
        } else {
            text
        };

        let root = CstRootNode::parse(&text, &ParseOptions::default())
            .map_err(|err| anyhow!("{} is not valid JSON: {err}", path.display()))?;

        Ok(Self {
            path: path.to_path_buf(),
            root,
            existed,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn existed(&self) -> bool {
        self.existed
    }

    /// The object the agent entries live in, looked up without changing anything.
    pub fn container(&self, keys: &[&str]) -> Option<CstObject> {
        let mut object = self.root.object_value()?;
        for key in keys {
            object = object.object_value(key)?;
        }
        Some(object)
    }

    /// The same object, creating the levels that are missing. A level already
    /// taken by something that is not an object is an error rather than an
    /// overwrite — quietly replacing a setting is worse than refusing.
    pub fn container_or_create(&self, keys: &[&str]) -> Result<CstObject> {
        let mut object = self
            .root
            .object_value_or_create()
            .ok_or_else(|| anyhow!("{} does not hold a JSON object", self.path.display()))?;
        for key in keys {
            object = object
                .object_value_or_create(key)
                .ok_or_else(|| anyhow!("\"{key}\" in {} is not an object", self.path.display()))?;
        }
        Ok(object)
    }

    pub fn to_text(&self) -> String {
        self.root.to_string()
    }

    pub fn save(&self) -> Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| anyhow!("{} has no parent directory", self.path.display()))?;
        fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;

        if self.existed {
            let backup = backup_path(&self.path);
            fs::copy(&self.path, &backup)
                .with_context(|| format!("could not back up {}", self.path.display()))?;
        }

        // Written beside the original and renamed, so a failure halfway through
        // leaves the settings the IDE is reading intact.
        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .with_context(|| format!("could not write into {}", parent.display()))?;
        temp.write_all(self.to_text().as_bytes())?;
        temp.flush()?;
        temp.persist(&self.path)
            .with_context(|| format!("could not write {}", self.path.display()))?;

        Ok(())
    }
}

pub fn backup_path(path: &Path) -> PathBuf {
    let mut name = OsString::from(path.file_name().unwrap_or_default());
    name.push(".markov-backup");
    path.with_file_name(name)
}

/// Reads an entry as plain JSON, for comparing what is there with what we want.
pub fn get_entry(container: &CstObject, name: &str) -> Option<serde_json::Value> {
    container.get(name)?.value()?.to_serde_value()
}

pub fn set_entry(container: &CstObject, name: &str, value: CstInputValue) {
    let prop = match container.get(name) {
        Some(existing) => {
            existing.set_value(value.clone());
            existing
        }
        None => {
            container.ensure_multiline();
            container.append(name, value.clone())
        }
    };

    rename(&prop, name);
    if let Some(node) = prop.value() {
        reescape(&node, &value);
    }
}

/// jsonc-parser escapes only double quotes when it writes a string, so a path
/// like C:\Users\ivan\markov.exe would land as invalid JSON. Every string we
/// insert is rewritten with serde_json's escaping, walking the value we asked
/// for alongside the nodes we got back.
fn reescape(node: &CstNode, value: &CstInputValue) {
    match value {
        CstInputValue::String(text) => {
            if let Some(literal) = node.as_string_lit() {
                literal.set_raw_value(encode(text));
            }
        }
        CstInputValue::Array(items) => {
            if let Some(array) = node.as_array() {
                for (child, item) in array.children_exclude_trivia_and_tokens().iter().zip(items) {
                    reescape(child, item);
                }
            }
        }
        CstInputValue::Object(entries) => {
            if let Some(object) = node.as_object() {
                for (prop, (name, item)) in object.properties().iter().zip(entries) {
                    rename(prop, name);
                    if let Some(child) = prop.value() {
                        reescape(&child, item);
                    }
                }
            }
        }
        _ => {}
    }
}

fn rename(prop: &CstObjectProp, name: &str) {
    if let Some(literal) = prop.name().and_then(|name| name.as_string_lit()) {
        literal.set_raw_value(encode(name));
    }
}

fn encode(text: &str) -> String {
    serde_json::Value::String(text.to_string()).to_string()
}

/// The entry as it would be stored, for telling it apart from what is already
/// in the file without depending on how either side orders its keys.
pub fn entry_as_json(value: CstInputValue) -> Option<serde_json::Value> {
    let doc = Document::scratch();
    let object = doc.container_or_create(&[]).ok()?;
    set_entry(&object, "entry", value);
    get_entry(&object, "entry")
}

pub fn remove_entry(container: &CstObject, name: &str) -> bool {
    match container.get(name) {
        Some(existing) => {
            existing.remove();
            true
        }
        None => false,
    }
}
