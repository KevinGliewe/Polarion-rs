//! Files on the machine the commands run on: which ones a command may read and write
//! ([`LocalFiles`], part of the [`crate::Context`]), and the images a Markdown file shows.

use std::path::{Component, Path, PathBuf};

use polarion::LocalImage;
use polarion::polarion_markdown as md;

use crate::arg::decode;
use crate::error::{CommandError, Result};

/// Largest image a Markdown file may show, in bytes.
const MAX_IMAGE: usize = 20 * 1024 * 1024;

/// The files a command may read and write. Fail-closed: a context has none unless whoever
/// builds it says otherwise.
#[derive(Debug, Clone, Copy, Default)]
pub enum LocalFiles<'a> {
    /// None (the default).
    #[default]
    Refused,
    /// Any file the process can reach; relative paths are relative to its working directory.
    /// For a command line, where the paths are the user's own.
    Unrestricted,
    /// Only files inside this folder; relative paths are relative to it. Links that lead out
    /// of it are refused.
    Within(&'a Path),
}

impl LocalFiles<'_> {
    /// Where `path` is, if it may be used.
    fn resolve(&self, path: &Path) -> Result<PathBuf> {
        match self {
            LocalFiles::Refused => Err(CommandError::invalid(format!(
                "local files are not available here: {} cannot be used (over MCP, `file` and \
                 `dir` need the exchange folder, POLARION_MCP_FILES)",
                path.display()
            ))),
            LocalFiles::Unrestricted => Ok(path.to_owned()),
            LocalFiles::Within(root) => within(root, path),
        }
    }

    /// Whether `path` may be used at all: an error if not. Nothing is read or written.
    pub fn check(&self, path: &Path) -> Result<()> {
        self.resolve(path).map(|_| ())
    }

    /// The content of a file.
    pub fn read(&self, path: &Path) -> Result<Vec<u8>> {
        let resolved = self.resolve(path)?;
        std::fs::read(&resolved)
            .map_err(|e| CommandError::invalid(format!("cannot read {}: {e}", path.display())))
    }

    /// Write a file, replacing an existing one; its folders are created.
    pub fn write(&self, path: &Path, data: &[u8]) -> Result<()> {
        let resolved = self.resolve(path)?;
        let failed = |e: std::io::Error| {
            CommandError::invalid(format!("cannot write {}: {e}", path.display()))
        };
        if let Some(parent) = resolved.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(failed)?;
        }
        std::fs::write(&resolved, data).map_err(failed)
    }
}

/// `path` inside `root`: relative to it, without `..`, and (following links, for the part that
/// exists) still inside it.
fn within(root: &Path, path: &Path) -> Result<PathBuf> {
    let outside = || {
        CommandError::invalid(format!(
            "{} is outside the folder files may be exchanged in ({})",
            path.display(),
            root.display()
        ))
    };
    let root = root
        .canonicalize()
        .map_err(|e| CommandError::config(format!("cannot use {}: {e}", root.display())))?;
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(outside());
    }
    let joined = root.join(path);
    // The deepest part that exists decides: what is created below it is inside it.
    let mut existing = joined.as_path();
    let mut rest: Vec<&std::ffi::OsStr> = Vec::new();
    let resolved = loop {
        match existing.canonicalize() {
            Ok(resolved) => break resolved,
            Err(_) => {
                let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else {
                    return Err(outside());
                };
                // A link that leads nowhere: writing to it would write to its target.
                if existing.symlink_metadata().is_ok() {
                    return Err(outside());
                }
                rest.push(name);
                existing = parent;
            }
        }
    };
    if !resolved.starts_with(&root) {
        return Err(outside());
    }
    Ok(rest
        .into_iter()
        .rev()
        .fold(resolved, |p, name| p.join(name)))
}

/// The text of a Markdown file: UTF-8, or UTF-16 with a byte order mark.
pub(crate) fn read_text(files: &LocalFiles<'_>, path: &Path) -> Result<String> {
    decode(files.read(path)?)
        .map_err(|e| CommandError::invalid(format!("cannot read {}: the file {e}", path.display())))
}

/// The images `markdown` (the content of `file`) shows from files next to it: every image whose
/// source is a relative path, read from the folder of `file`.
///
/// Sources with a scheme (`https:`, `workitemimg:`, `data:`, also `C:\…`), and those starting
/// with `/`, `\` or `#`, are not files and stay as they are. A relative source must not contain
/// `..`, must name a PNG, JPEG, GIF, WebP or SVG file (by its name and its content) of at most
/// 20 MB; anything else is an error, before the command sends anything.
pub(crate) fn local_images(
    files: &LocalFiles<'_>,
    file: &Path,
    markdown: &str,
) -> Result<Vec<LocalImage>> {
    let folder = file.parent().unwrap_or(Path::new(""));
    let mut images = Vec::new();
    for url in md::image_urls(markdown) {
        let Some(relative) = relative_path(&url) else {
            continue;
        };
        let invalid = |why: &str| CommandError::invalid(format!("image {url}: {why}"));
        let path = Path::new(&relative);
        if path
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
        {
            return Err(invalid(
                "only files in the folder of the Markdown file (or below it) can be attached",
            ));
        }
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| invalid("it names no file"))?
            .to_owned();
        let data = files
            .read(&folder.join(path))
            .map_err(|e| invalid(&e.message))?;
        if data.len() > MAX_IMAGE {
            return Err(invalid("it is larger than 20 MB"));
        }
        if !is_image(&file_name, &data) {
            return Err(invalid(
                "it is not a PNG, JPEG, GIF, WebP or SVG image (by its name and its content)",
            ));
        }
        images.push(LocalImage {
            url,
            file_name,
            data,
        });
    }
    Ok(images)
}

/// The path an image source names, if it is a relative one: without query and fragment,
/// percent-decoded, `\` as `/`.
fn relative_path(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next().unwrap_or("");
    if path.is_empty() || path.starts_with(['/', '\\']) {
        return None;
    }
    // A scheme: a letter, then letters, digits, `+`, `-` or `.`, up to a `:` (a drive letter too).
    if let Some((scheme, _)) = path.split_once(':')
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return None;
    }
    Some(percent_decode(path).replace('\\', "/"))
}

/// `%XX` sequences decoded; one that is not valid UTF-8 stays as written.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |j: usize| bytes.get(j).and_then(|&b| char::from(b).to_digit(16));
        match (bytes[i], hex(i + 1), hex(i + 2)) {
            (b'%', Some(hi), Some(lo)) => {
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_owned())
}

/// Whether a file is an image Polarion shows inline, by its extension and its first bytes.
fn is_image(file_name: &str, data: &[u8]) -> bool {
    let extension = file_name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "png" => data.starts_with(b"\x89PNG\r\n\x1a\n"),
        "jpg" | "jpeg" => data.starts_with(&[0xff, 0xd8, 0xff]),
        "gif" => data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a"),
        "webp" => data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP",
        "svg" => {
            let head = String::from_utf8_lossy(&data[..data.len().min(4096)]);
            head.trim_start_matches('\u{feff}')
                .trim_start()
                .starts_with('<')
                && head.contains("<svg")
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrest";

    /// A new, empty folder for one test.
    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "polarion-commands-files-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn relative_sources_are_files() {
        assert_eq!(relative_path("img/a.png").as_deref(), Some("img/a.png"));
        assert_eq!(
            relative_path("./a%20b.png?x=1#f").as_deref(),
            Some("./a b.png")
        );
        assert_eq!(relative_path("img\\a.png").as_deref(), Some("img/a.png"));
        assert_eq!(relative_path("../a.png").as_deref(), Some("../a.png"));
        for other in [
            "https://example.com/a.png",
            "workitemimg:1-a.png",
            "data:image/png;base64,AAAA",
            "C:\\img\\a.png",
            "/img/a.png",
            "\\\\server\\a.png",
            "//example.com/a.png",
            "#top",
            "",
            "?x",
        ] {
            assert_eq!(relative_path(other), None, "{other}");
        }
    }

    #[test]
    fn images_by_name_and_content() {
        assert!(is_image("a.png", PNG));
        assert!(is_image("a.PNG", PNG));
        assert!(is_image("a.jpeg", &[0xff, 0xd8, 0xff, 0xe0]));
        assert!(is_image("a.gif", b"GIF89a..."));
        assert!(is_image("a.webp", b"RIFF\0\0\0\0WEBPVP8 "));
        assert!(is_image(
            "a.svg",
            b"<?xml version=\"1.0\"?>\n<svg xmlns=\"x\"/>"
        ));
        // The name and the content must agree, and other files are refused.
        assert!(!is_image("a.jpg", PNG));
        assert!(!is_image("a.png", b"#!/bin/sh"));
        assert!(!is_image("a.svg", b"<html></html>"));
        assert!(!is_image("a.exe", PNG));
        assert!(!is_image("png", PNG));
    }

    #[test]
    fn refused_by_default() {
        let files = LocalFiles::default();
        let e = files.read(Path::new("a.md")).unwrap_err();
        assert_eq!(e.code, "invalid");
        assert!(e.message.contains("not available"), "{}", e.message);
        assert!(files.write(Path::new("a.md"), b"x").is_err());
    }

    #[test]
    fn within_a_folder_only() {
        let root = folder("within");
        let outside = folder("outside");
        std::fs::write(outside.join("secret.txt"), "s").unwrap();
        let files = LocalFiles::Within(&root);

        files.write(Path::new("out/SB-1/a.png"), PNG).unwrap();
        assert_eq!(std::fs::read(root.join("out/SB-1/a.png")).unwrap(), PNG);
        assert_eq!(files.read(Path::new("out/SB-1/a.png")).unwrap(), PNG);
        // An absolute path inside the folder is fine.
        assert_eq!(files.read(&root.join("out/SB-1/a.png")).unwrap(), PNG);

        for path in [
            PathBuf::from("../x"),
            PathBuf::from("out/../../x"),
            outside.join("secret.txt"),
        ] {
            let read = files.read(&path).unwrap_err();
            assert!(read.message.contains("outside"), "{}", read.message);
            let write = files.write(&path, b"x").unwrap_err();
            assert!(write.message.contains("outside"), "{}", write.message);
        }
        assert_eq!(std::fs::read(outside.join("secret.txt")).unwrap(), b"s");

        // A link out of the folder is not followed (where the platform lets a test make one).
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
            assert!(files.read(Path::new("link/secret.txt")).is_err());
            assert!(files.write(Path::new("link/new.txt"), b"x").is_err());
            assert!(!outside.join("new.txt").exists());
        }
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn the_images_of_a_markdown_file() {
        let root = folder("images");
        std::fs::create_dir_all(root.join("doc/img")).unwrap();
        std::fs::write(root.join("doc/img/a b.png"), PNG).unwrap();
        std::fs::write(root.join("doc/notes.txt"), "text").unwrap();
        std::fs::write(root.join("top.png"), PNG).unwrap();
        let files = LocalFiles::Unrestricted;
        let file = root.join("doc/d.md");

        let markdown = "![](img/a%20b.png) ![](https://example.com/x.png) ![](workitemimg:1-x.png)";
        let images = local_images(&files, &file, markdown).unwrap();
        assert_eq!(
            images,
            [LocalImage {
                url: "img/a%20b.png".into(),
                file_name: "a b.png".into(),
                data: PNG.to_vec(),
            }]
        );
        assert!(local_images(&files, &file, "no images").unwrap().is_empty());

        for (markdown, why) in [
            ("![](../top.png)", "folder of the Markdown file"),
            ("![](img/missing.png)", "cannot read"),
            ("![](notes.txt)", "not a PNG"),
        ] {
            let e = local_images(&files, &file, markdown).unwrap_err();
            assert_eq!(e.code, "invalid");
            assert!(e.message.contains(why), "{}", e.message);
        }
        // Without file access, a Markdown file with a local image is refused.
        assert!(local_images(&LocalFiles::Refused, &file, markdown).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
