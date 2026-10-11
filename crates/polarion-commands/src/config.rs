//! Connection settings from the environment (and command-line overrides).

use polarion::{Auth, DocumentRef, ItemRef, Polarion};

use crate::error::{CommandError, Result};

/// Everything needed to connect, usually from environment variables:
///
/// | Variable | Meaning |
/// |---|---|
/// | `POLARION_URL` | Polarion base URL, e.g. `https://polarion.example.com/polarion` |
/// | `POLARION_TOKEN` | personal access token, or |
/// | `POLARION_USER`, `POLARION_PASSWORD` | user name and password |
/// | `POLARION_PROJECT` | default project for ids without a project |
/// | `POLARION_INSECURE=1` | accept invalid TLS certificates (test servers only) |
/// | `POLARION_WRITE_PROJECTS` | comma-separated projects that may be written; none: read-only |
/// | `POLARION_ALLOW_GLOBAL_WRITES=1` | allow writes not tied to a project (users, global folders, ...) |
/// | `POLARION_WRITE_TARGETS` | comma-separated objects that may be written, and nothing else (see [`WriteTarget`], [`split_targets`]) |
///
/// `POLARION_WRITE_TARGETS` is fail-closed: when it is set at all, even empty, the client is
/// restricted to the objects it names (none: read-only), and `POLARION_WRITE_PROJECTS` and
/// `POLARION_ALLOW_GLOBAL_WRITES` have no effect. Creates are never allowed then. Its entries
/// without a project are resolved against `POLARION_PROJECT` as it is in the environment
/// ([`Config::write_targets_project`]), so a command-line `-p` cannot move them.
#[derive(Clone, Default)]
pub struct Config {
    pub url: Option<String>,
    pub auth: Option<Auth>,
    pub project: Option<String>,
    pub insecure: bool,
    pub write_projects: Vec<String>,
    pub global_writes: bool,
    /// The write targets as given (`POLARION_WRITE_TARGETS`, `--write-targets`), resolved by
    /// [`Config::connect`] against [`Config::write_targets_project`]. `None`: writes are not
    /// restricted to targets; `Some`, even empty: they are.
    pub write_targets: Option<Vec<String>>,
    /// The project that write targets without one (`ID`, `doc:SPACE/NAME`, `run:ID`) are
    /// resolved against. It is not [`Config::project`]: targets from the environment are
    /// resolved against `POLARION_PROJECT` from the environment, so an option that changes the
    /// default project of the commands never changes what may be written. `None`: such entries
    /// are a `config` error.
    pub write_targets_project: Option<String>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("url", &self.url)
            .field("auth", &self.auth)
            .field("project", &self.project)
            .field("insecure", &self.insecure)
            .field("write_projects", &self.write_projects)
            .field("global_writes", &self.global_writes)
            .field("write_targets", &self.write_targets)
            .field("write_targets_project", &self.write_targets_project)
            .finish()
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn flag(name: &str) -> bool {
    var(name).is_some_and(|v| matches!(v.trim(), "1" | "true" | "yes" | "on"))
}

/// Split a comma-separated list, dropping empty entries.
pub fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Split a list of write targets: entries are separated by commas, `\,` is a comma inside an
/// entry (a document name with a comma) and `\\` a backslash; entries are trimmed, empty ones
/// dropped. An unescaped comma always ends an entry, so what follows it must parse as an entry
/// of its own ([`WriteTarget::parse`] refuses the `v2` of `doc:Specs/Brakes, v2`).
pub fn split_targets(value: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut current = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(next @ (',' | '\\')) => current.push(next),
                Some(next) => {
                    current.push('\\');
                    current.push(next);
                }
                None => current.push('\\'),
            },
            ',' => entries.push(std::mem::take(&mut current)),
            c => current.push(c),
        }
    }
    entries.push(current);
    entries
        .iter()
        .map(|e| e.trim())
        .filter(|e| !e.is_empty())
        .map(str::to_owned)
        .collect()
}

impl Config {
    pub fn from_env() -> Self {
        let auth = var("POLARION_TOKEN").map(Auth::Token).or_else(|| {
            Some(Auth::Password {
                user: var("POLARION_USER")?,
                password: var("POLARION_PASSWORD")?,
            })
        });
        Self {
            url: var("POLARION_URL"),
            auth,
            project: var("POLARION_PROJECT"),
            insecure: flag("POLARION_INSECURE"),
            write_projects: var("POLARION_WRITE_PROJECTS")
                .map(|v| split_list(&v))
                .unwrap_or_default(),
            global_writes: flag("POLARION_ALLOW_GLOBAL_WRITES"),
            // Set at all (even empty, even not Unicode) restricts writes; a value that is not
            // Unicode keeps its replacement characters, which no target accepts.
            write_targets: std::env::var_os("POLARION_WRITE_TARGETS")
                .map(|v| split_targets(&v.to_string_lossy())),
            write_targets_project: var("POLARION_PROJECT"),
        }
    }

    /// Whether any write is enabled: write targets, or (without them) write projects or
    /// global writes.
    pub fn writes_enabled(&self) -> bool {
        match &self.write_targets {
            Some(targets) => !targets.is_empty(),
            None => self.global_writes || !self.write_projects.is_empty(),
        }
    }

    /// The write targets resolved against [`Config::write_targets_project`]: `None` when writes
    /// are not restricted to targets. An entry that does not parse, or names no project when
    /// there is no such project, is a `config` error.
    pub fn resolve_write_targets(&self) -> Result<Option<Vec<WriteTarget>>> {
        let Some(entries) = &self.write_targets else {
            return Ok(None);
        };
        entries
            .iter()
            .map(|entry| WriteTarget::parse(entry, self.write_targets_project.as_deref()))
            .collect::<Result<Vec<_>>>()
            .map(Some)
    }

    /// A client for these settings; read-only unless writes are enabled, and restricted to the
    /// write targets when they are set.
    pub fn connect(&self) -> Result<Polarion> {
        let url = self
            .url
            .as_deref()
            .ok_or_else(|| CommandError::config("no server: set POLARION_URL or pass --url"))?;
        let auth = self.auth.clone().ok_or_else(|| {
            CommandError::config(
                "no credentials: set POLARION_TOKEN, or POLARION_USER and POLARION_PASSWORD",
            )
        })?;
        let builder = Polarion::builder(url, auth).danger_accept_invalid_certs(self.insecure);
        let builder = match self.resolve_write_targets()? {
            // Restricted mode, even with no targets: no budget, so nothing can be created.
            Some(targets) => {
                builder.restrict_writes(targets.into_iter().map(WriteTarget::into_uri))
            }
            None => builder
                .allow_writes(self.write_projects.iter().cloned())
                .allow_global_writes(self.global_writes),
        };
        Ok(builder.build()?)
    }
}

/// An object that may be written, from an entry of `POLARION_WRITE_TARGETS`:
///
/// | Entry | Object |
/// |---|---|
/// | `ID`, `PROJECT/ID` | a work item (`ID` in the default project), its id `PREFIX-NUMBER` |
/// | `doc:SPACE/NAME`, `doc:PROJECT/SPACE/NAME` | a document |
/// | `run:ID`, `run:PROJECT/ID` | a test run |
/// | `subterra:data-service:objects:/…` | any object, by URI (a `%revision` suffix is ignored) |
///
/// In a list, a comma inside a name is written `\,` ([`split_targets`]).
///
/// Displays as the kind and the form commands take it in: `work item P/ID`,
/// `document P/SPACE/NAME`, `test run <URI>`, or the URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteTarget {
    Item(ItemRef),
    Document(DocumentRef),
    TestRun {
        project: String,
        id: String,
        uri: String,
    },
    Uri(String),
}

const URI_PREFIX: &str = "subterra:data-service:objects:/";

const SYNTAX: &str = "expected ID or PROJECT/ID (a work item id is PREFIX-NUMBER; give the URI \
                      otherwise), doc:SPACE/NAME, doc:PROJECT/SPACE/NAME, run:ID, run:PROJECT/ID \
                      or an object URI; a comma inside a name is written \\,";

impl WriteTarget {
    /// One entry, with `project` for entries without one.
    pub fn parse(entry: &str, project: Option<&str>) -> Result<Self> {
        let entry = entry.trim();
        let invalid =
            |why: &str| CommandError::config(format!("invalid write target `{entry}`: {why}"));
        if entry
            .chars()
            .any(|c| c.is_control() || c == char::REPLACEMENT_CHARACTER)
        {
            return Err(invalid("control characters or text that is not Unicode"));
        }
        let default_project = || match project {
            Some(p) if is_id(p) => Ok(p),
            Some(p) => Err(CommandError::config(format!(
                "write target `{entry}`: the default project `{p}` is not a project id"
            ))),
            None => Err(CommandError::config(format!(
                "write target `{entry}` has no project: give it (PROJECT/...), or set \
                 POLARION_PROJECT (with --write-targets also -p)"
            ))),
        };
        if entry.starts_with("subterra:") {
            let uri = strip_revision(entry);
            if !uri.starts_with(URI_PREFIX) || !uri[URI_PREFIX.len()..].contains("${") {
                return Err(invalid(&format!("not an object URI ({URI_PREFIX}…)")));
            }
            return Ok(Self::from_uri(uri));
        }
        if let Some(location) = entry.strip_prefix("doc:") {
            let (project, space, name) = match location.split('/').collect::<Vec<_>>()[..] {
                [space, name] => (default_project()?, space, name),
                [project, space, name] if is_id(project) => (project, space, name),
                _ => return Err(invalid("expected doc:SPACE/NAME or doc:PROJECT/SPACE/NAME")),
            };
            if !is_name(space) || !is_name(name) {
                return Err(invalid("expected doc:SPACE/NAME or doc:PROJECT/SPACE/NAME"));
            }
            return Ok(Self::Document(DocumentRef::new(project, space, name)));
        }
        if let Some(run) = entry.strip_prefix("run:") {
            let (project, id) = match run.split('/').collect::<Vec<_>>()[..] {
                [id] => (default_project()?, id),
                [project, id] if is_id(project) => (project, id),
                _ => return Err(invalid("expected run:ID or run:PROJECT/ID")),
            };
            if !is_name(id) {
                return Err(invalid("expected run:ID or run:PROJECT/ID"));
            }
            return Ok(Self::TestRun {
                project: project.to_owned(),
                id: id.to_owned(),
                uri: polarion::uri::test_run_uri(project, id),
            });
        }
        let (project, id) = match entry.split('/').collect::<Vec<_>>()[..] {
            [id] if is_work_item_id(id) => (default_project()?, id),
            [project, id] if is_id(project) && is_work_item_id(id) => (project, id),
            _ => return Err(invalid(SYNTAX)),
        };
        Ok(Self::Item(ItemRef::new(project, id)))
    }

    /// A URI, as a work item or a document when it is the URI those are written to (anything
    /// else, such as the URI of a comment, stays a URI).
    fn from_uri(uri: &str) -> Self {
        let item = ItemRef::from_uri(uri);
        if let (Some(project), Some(id)) = (&item.project, &item.id)
            && is_id(project)
            && is_id(id)
            && ItemRef::new(project, id).uri == uri
        {
            return Self::Item(item);
        }
        let document = DocumentRef::from_uri(uri);
        if let (Some(project), Some(space), Some(name)) =
            (&document.project, &document.space, &document.name)
            && is_id(project)
            && is_name(space)
            && is_name(name)
            && DocumentRef::new(project, space, name).uri == uri
        {
            return Self::Document(document);
        }
        Self::Uri(uri.to_owned())
    }

    /// The object URI writes are restricted to.
    pub fn uri(&self) -> &str {
        match self {
            Self::Item(item) => &item.uri,
            Self::Document(document) => &document.uri,
            Self::TestRun { uri, .. } | Self::Uri(uri) => uri,
        }
    }

    fn into_uri(self) -> String {
        match self {
            Self::Item(item) => item.uri,
            Self::Document(document) => document.uri,
            Self::TestRun { uri, .. } | Self::Uri(uri) => uri,
        }
    }
}

impl std::fmt::Display for WriteTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Item(item) => write!(f, "work item {item}"),
            Self::Document(document) => write!(f, "document {document}"),
            // Test run commands take the URI.
            Self::TestRun { uri, .. } => write!(f, "test run {uri}"),
            Self::Uri(uri) => f.write_str(uri),
        }
    }
}

/// A project or work item id: ASCII letters, digits, `_`, `-` and `.`.
fn is_id(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// A work item id as Polarion makes them, `PREFIX-NUMBER` (`SB-12`), the prefix an id.
fn is_work_item_id(s: &str) -> bool {
    s.rsplit_once('-').is_some_and(|(prefix, number)| {
        is_id(prefix) && !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())
    })
}

/// A space, document name or test run id: not empty, no surrounding blanks, none of the
/// characters object URIs reserve.
fn is_name(s: &str) -> bool {
    !s.is_empty() && s.trim() == s && !s.contains(['$', '{', '}', '%', '#', '/'])
}

/// `uri` without a `%revision` suffix.
fn strip_revision(uri: &str) -> &str {
    match uri.rsplit_once('%') {
        Some((head, rev)) if !rev.is_empty() && rev.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => uri,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URI: &str = "subterra:data-service:objects:/default/";

    fn target(entry: &str) -> Result<WriteTarget> {
        WriteTarget::parse(entry, Some("Sandbox"))
    }

    fn uri(entry: &str) -> String {
        target(entry).unwrap().uri().to_owned()
    }

    #[test]
    fn entries_resolve_to_uris() {
        let item = format!("{URI}Sandbox${{WorkItem}}SB-1");
        assert_eq!(uri("SB-1"), item);
        assert_eq!(uri("Sandbox/SB-1"), item);
        assert_eq!(uri(&item), item);
        assert_eq!(uri(&format!("{item}%4711")), item);
        assert_eq!(uri("Other/OT-2"), format!("{URI}Other${{WorkItem}}OT-2"));

        let document = format!("{URI}Sandbox${{Module}}{{moduleFolder}}_default#Live fixture");
        assert_eq!(uri("doc:_default/Live fixture"), document);
        assert_eq!(uri("doc:Sandbox/_default/Live fixture"), document);
        assert_eq!(uri(&document), document);

        let run = format!("{URI}Sandbox${{TestRun}}RUN-1");
        assert_eq!(uri("run:RUN-1"), run);
        assert_eq!(uri("run:Sandbox/RUN-1"), run);
        assert_eq!(uri(&run), run);
    }

    #[test]
    fn targets_name_what_commands_take() {
        let names: Vec<String> = [
            "SB-1",
            "doc:_default/Live fixture",
            "run:RUN-1",
            &format!("{URI}Sandbox${{WorkItem}}SB-2"),
            &format!("{URI}Sandbox${{TestRun}}RUN-2"),
        ]
        .iter()
        .map(|e| target(e).unwrap().to_string())
        .collect();
        assert_eq!(
            names,
            [
                "work item Sandbox/SB-1",
                "document Sandbox/_default/Live fixture",
                &format!("test run {URI}Sandbox${{TestRun}}RUN-1"),
                "work item Sandbox/SB-2",
                &format!("{URI}Sandbox${{TestRun}}RUN-2"),
            ]
        );
        // The URI of a comment names the comment, not its work item.
        let comment = format!("{URI}Sandbox${{WorkItem}}SB-1/3");
        assert_eq!(target(&comment).unwrap(), WriteTarget::Uri(comment.clone()));
        assert_eq!(target(&comment).unwrap().to_string(), comment);
    }

    #[test]
    fn bad_entries_are_config_errors() {
        for entry in [
            "SB-1 SB-2",
            "a/b/c",
            "Sandbox/",
            "/SB-1",
            "doc:Spec",
            "doc:a/b/c/d",
            "doc:_default/a#b",
            "doc:Sand box/_default/x",
            "run:",
            "run:a/b/c",
            "run:R%1",
            "item:SB-1",
            "subterra:whatever",
            "subterra:data-service:objects:/default/",
            "SB-\u{FFFD}",
            "SB-\u{1b}1",
            // Work item ids are PREFIX-NUMBER: the rest of a name cut at a comma is no id.
            "v2",
            "SB",
            "SB-",
            "-1",
            "SB-1a",
            "Sandbox/v2",
        ] {
            let e = target(entry).unwrap_err();
            assert_eq!(e.code, "config", "{entry}");
            assert!(e.message.contains("write target"), "{entry}: {}", e.message);
        }
        // Without a default project, entries must name theirs; the default project must be an
        // id.
        for entry in ["SB-1", "doc:_default/x", "run:R"] {
            let e = WriteTarget::parse(entry, None).unwrap_err();
            assert_eq!(e.code, "config");
            assert!(e.message.contains("has no project"), "{}", e.message);
            for project in ["", " ", "a b", "P/Q"] {
                let e = WriteTarget::parse(entry, Some(project)).unwrap_err();
                assert_eq!(e.code, "config");
                assert!(e.message.contains("is not a project id"), "{}", e.message);
            }
        }
        WriteTarget::parse("Sandbox/SB-1", None).unwrap();
        WriteTarget::parse("Sandbox/SB-1", Some("")).unwrap();
    }

    #[test]
    fn lists_split_on_unescaped_commas() {
        assert_eq!(
            split_targets(" SB-1 ,, doc:Specs/SRS ,"),
            ["SB-1", "doc:Specs/SRS"]
        );
        assert!(split_targets("").is_empty());
        assert!(split_targets("  ,  , ").is_empty());
        // A comma inside a name, a backslash, other backslashes kept.
        assert_eq!(
            split_targets(r"doc:Specs/Brakes\, v2, SB-1"),
            ["doc:Specs/Brakes, v2", "SB-1"]
        );
        assert_eq!(split_targets(r"doc:a/b\\c,x\y\"), [r"doc:a/b\c", r"x\y\"]);
        assert_eq!(
            uri(&split_targets(r"doc:Specs/Brakes\, v2")[0]),
            format!("{URI}Sandbox${{Module}}{{moduleFolder}}Specs#Brakes, v2")
        );
        // Unescaped, the rest of the name is an entry of its own, and no work item.
        let parts = split_targets("doc:Specs/Brakes, v2");
        assert_eq!(parts, ["doc:Specs/Brakes", "v2"]);
        assert_eq!(target(&parts[1]).unwrap_err().code, "config");
    }

    #[test]
    fn targets_are_fail_closed_and_count_as_writes() {
        let mut config = Config {
            write_projects: vec!["Sandbox".into()],
            global_writes: true,
            ..Config::default()
        };
        assert!(config.writes_enabled());
        assert_eq!(config.resolve_write_targets().unwrap(), None);

        // Set but empty: restricted, and nothing may be written.
        config.write_targets = Some(Vec::new());
        assert!(!config.writes_enabled());
        assert_eq!(config.resolve_write_targets().unwrap(), Some(Vec::new()));

        config.write_targets = Some(vec!["SB-1".into()]);
        assert!(config.writes_enabled());
        // Resolved against their own project, never the commands' default project.
        assert_eq!(config.resolve_write_targets().unwrap_err().code, "config");
        config.project = Some("Other".into());
        assert_eq!(config.resolve_write_targets().unwrap_err().code, "config");
        config.write_targets_project = Some("Sandbox".into());
        assert_eq!(
            config.resolve_write_targets().unwrap().unwrap()[0].uri(),
            format!("{URI}Sandbox${{WorkItem}}SB-1")
        );
    }

    #[test]
    fn connect_restricts_the_client_to_the_targets() {
        let config = Config {
            url: Some("https://polarion.invalid/polarion".into()),
            auth: Some(Auth::Token("t".into())),
            project: Some("Other".into()),
            write_projects: vec!["Sandbox".into()],
            global_writes: true,
            write_targets: Some(vec!["SB-1".into(), "doc:_default/Spec".into()]),
            write_targets_project: Some("Sandbox".into()),
            ..Config::default()
        };
        let polarion = config.connect().unwrap();
        assert!(polarion.writes_restricted());
        assert!(!polarion.global_writes_allowed());
        assert_eq!(polarion.writable_projects().count(), 0);
        assert!(polarion.write_allowed(&format!("{URI}Sandbox${{WorkItem}}SB-1")));
        assert!(!polarion.write_allowed(&format!("{URI}Sandbox${{WorkItem}}SB-2")));
        assert_eq!(
            polarion.write_targets().collect::<Vec<_>>(),
            [
                format!("{URI}Sandbox${{Module}}{{moduleFolder}}_default#Spec"),
                format!("{URI}Sandbox${{WorkItem}}SB-1"),
            ]
        );

        let empty = Config {
            write_targets: Some(Vec::new()),
            ..config.clone()
        };
        let polarion = empty.connect().unwrap();
        assert!(polarion.writes_restricted());
        assert_eq!(polarion.write_targets().count(), 0);
        assert!(!polarion.write_allowed(&format!("{URI}Sandbox${{WorkItem}}SB-1")));

        let bad = Config {
            write_targets: Some(vec!["doc:nope".into()]),
            ..config
        };
        assert_eq!(bad.connect().unwrap_err().code, "config");
    }
}
