use std::fs;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{Result, WrapErr};
use walkdir::WalkDir;

use super::atomic_write;
use super::models::{CollectionNode, CollectionNodeKind, RequestDocument};

#[allow(dead_code)] // Used in Step 6 when collections pane is wired up.
/// Directories to skip during collection discovery.
const IGNORED_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".reqsmith",
    "dist",
    "build",
    ".next",
    "__pycache__",
    "vendor",
];

#[allow(dead_code)] // Used in Step 6.
/// Recursively discover `.req.yml` files from `cwd`, returning a tree of
/// collection nodes grouped by directory.
pub fn discover_requests(cwd: &Path) -> Result<Vec<CollectionNode>> {
    let mut file_paths: Vec<PathBuf> = WalkDir::new(cwd)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| {
            if !e.file_type().is_dir() {
                return true;
            }
            let name = e.file_name().to_str().unwrap_or("");
            !IGNORED_DIRS.contains(&name)
        })
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_file()
                && e.path()
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(".req.yml") || n.ends_with(".req.yaml"))
        })
        .map(|e| e.into_path())
        .collect();

    file_paths.sort();
    build_tree(cwd, &file_paths)
}

#[allow(dead_code)] // Used in Step 5.
/// Load a request document from a YAML file.
pub fn load_request(path: &Path) -> Result<RequestDocument> {
    let content =
        fs::read_to_string(path).wrap_err_with(|| format!("Failed to read {}", path.display()))?;
    let mut doc: RequestDocument = serde_yaml::from_str(&content)
        .wrap_err_with(|| format!("Failed to parse YAML in {}", path.display()))?;
    doc.file_path = Some(path.to_path_buf());
    Ok(doc)
}

#[allow(dead_code)] // Used in Step 5.
/// Save a request document to a YAML file using an atomic, symlink-refusing
/// write (shared with the rest of reqsmith's on-disk writes; see
/// [`atomic_write`]).
pub fn save_request(doc: &RequestDocument, path: &Path) -> Result<()> {
    let yaml =
        serde_yaml::to_string(doc).wrap_err("Failed to serialize request document to YAML")?;

    atomic_write::write_replace(path, yaml.as_bytes())
        .wrap_err_with(|| format!("Failed to write request file {}", path.display()))?;

    Ok(())
}

#[allow(dead_code)]
/// Build a directory tree from a flat list of file paths relative to `root`.
fn build_tree(root: &Path, file_paths: &[PathBuf]) -> Result<Vec<CollectionNode>> {
    let mut top_level: Vec<CollectionNode> = Vec::new();

    for path in file_paths {
        let relative = path.strip_prefix(root).unwrap_or(path);
        let components: Vec<&str> = relative
            .components()
            .filter_map(|c| c.as_os_str().to_str())
            .collect();

        insert_into_tree(&mut top_level, path, &components, 0);
    }

    Ok(top_level)
}

#[allow(dead_code)]
fn insert_into_tree(
    nodes: &mut Vec<CollectionNode>,
    full_path: &Path,
    components: &[&str],
    depth: usize,
) {
    if components.is_empty() {
        return;
    }

    let name = components[0];
    let is_leaf = components.len() == 1;

    if is_leaf {
        // This is a request file.
        let display_name = name
            .strip_suffix(".req.yml")
            .or_else(|| name.strip_suffix(".req.yaml"))
            .unwrap_or(name);
        nodes.push(CollectionNode {
            name: display_name.to_string(),
            path: full_path.to_path_buf(),
            kind: CollectionNodeKind::RequestFile,
            children: Vec::new(),
            depth,
        });
    } else {
        // This is a directory component. Find or create it.
        let dir_pos = nodes
            .iter()
            .position(|n| n.kind == CollectionNodeKind::Directory && n.name == name);

        let idx = if let Some(pos) = dir_pos {
            pos
        } else {
            let dir_path = full_path
                .ancestors()
                .nth(components.len() - 1)
                .unwrap_or(full_path)
                .to_path_buf();
            nodes.push(CollectionNode {
                name: name.to_string(),
                path: dir_path,
                kind: CollectionNodeKind::Directory,
                children: Vec::new(),
                depth,
            });
            nodes.len() - 1
        };

        insert_into_tree(
            &mut nodes[idx].children,
            full_path,
            &components[1..],
            depth + 1,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_temp_dir() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    fn write_request_file(dir: &Path, name: &str) {
        let yaml = format!("name: {name}\nmethod: GET\nurl: https://example.com\n");
        fs::write(dir.join(format!("{name}.req.yml")), yaml).unwrap();
    }

    #[test]
    fn discover_finds_req_yml_files() {
        let tmp = create_temp_dir();
        write_request_file(tmp.path(), "get_users");
        write_request_file(tmp.path(), "create_user");

        let nodes = discover_requests(tmp.path()).unwrap();
        assert_eq!(nodes.len(), 2);
        assert!(
            nodes
                .iter()
                .all(|n| n.kind == CollectionNodeKind::RequestFile)
        );
    }

    #[test]
    fn discover_skips_ignored_directories() {
        let tmp = create_temp_dir();
        write_request_file(tmp.path(), "root_request");

        let git_dir = tmp.path().join(".git");
        fs::create_dir_all(&git_dir).unwrap();
        write_request_file(&git_dir, "hidden");

        let target_dir = tmp.path().join("target");
        fs::create_dir_all(&target_dir).unwrap();
        write_request_file(&target_dir, "hidden");

        let nodes = discover_requests(tmp.path()).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].name, "root_request");
    }

    #[test]
    fn discover_builds_directory_tree() {
        let tmp = create_temp_dir();
        let sub = tmp.path().join("api").join("users");
        fs::create_dir_all(&sub).unwrap();
        write_request_file(&sub, "list");
        write_request_file(&sub, "create");

        let nodes = discover_requests(tmp.path()).unwrap();
        assert_eq!(nodes.len(), 1); // "api" directory
        assert_eq!(nodes[0].kind, CollectionNodeKind::Directory);
        assert_eq!(nodes[0].children.len(), 1); // "users" directory
        assert_eq!(nodes[0].children[0].children.len(), 2); // two request files
    }

    #[test]
    fn discover_returns_empty_for_no_files() {
        let tmp = create_temp_dir();
        let nodes = discover_requests(tmp.path()).unwrap();
        assert!(nodes.is_empty());
    }

    #[test]
    fn load_request_parses_valid_yaml() {
        let tmp = create_temp_dir();
        let path = tmp.path().join("test.req.yml");
        let yaml = "name: Test\nmethod: POST\nurl: https://example.com\n";
        fs::write(&path, yaml).unwrap();

        let doc = load_request(&path).unwrap();
        assert_eq!(doc.name, "Test");
        assert_eq!(doc.method, super::super::models::HttpMethod::Post);
        assert_eq!(doc.file_path, Some(path));
    }

    #[test]
    fn load_request_fails_on_invalid_yaml() {
        let tmp = create_temp_dir();
        let path = tmp.path().join("bad.req.yml");
        fs::write(&path, "not: [valid: yaml: {{{").unwrap();

        let result = load_request(&path);
        assert!(result.is_err());
    }

    #[test]
    fn save_and_load_round_trip() {
        let tmp = create_temp_dir();
        let path = tmp.path().join("roundtrip.req.yml");

        let doc = RequestDocument {
            name: "Round Trip".into(),
            method: super::super::models::HttpMethod::Put,
            url: "https://example.com/api".into(),
            headers: vec![super::super::models::KeyValueField {
                key: "Auth".into(),
                value: "Bearer token".into(),
                enabled: true,
            }],
            params: vec![],
            body: Some("body content".into()),
            auth_plugin: None,
            assertions: vec![],
            file_path: None,
        };

        save_request(&doc, &path).unwrap();
        let loaded = load_request(&path).unwrap();

        assert_eq!(loaded.name, doc.name);
        assert_eq!(loaded.method, doc.method);
        assert_eq!(loaded.url, doc.url);
        assert_eq!(loaded.headers, doc.headers);
        assert_eq!(loaded.body, doc.body);
    }

    #[test]
    fn collection_node_strips_extension_from_name() {
        let tmp = create_temp_dir();
        write_request_file(tmp.path(), "my_request");

        let nodes = discover_requests(tmp.path()).unwrap();
        assert_eq!(nodes[0].name, "my_request");
    }
}
