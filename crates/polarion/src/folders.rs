//! Folders (spaces) of the "Documents & Pages" topic, per project or global.

use polarion_soap::types::Folder;

use crate::client::Polarion;
use crate::error::Result;

/// Folders: [`Polarion::folders`]. `project` `None` means the global level.
///
/// Results are the SOAP [`Folder`] type (name, title, project URI).
#[derive(Debug, Clone, Copy)]
pub struct Folders<'a> {
    polarion: &'a Polarion,
}

impl<'a> Folders<'a> {
    pub(crate) fn new(polarion: &'a Polarion) -> Self {
        Self { polarion }
    }

    pub async fn list(&self, project: Option<&str>) -> Result<Vec<Folder>> {
        Ok(self.polarion.soap().tracker().get_folders(project).await?)
    }

    /// Top-level folders, sorted by name.
    pub async fn roots(&self, project: Option<&str>) -> Result<Vec<Folder>> {
        Ok(self
            .polarion
            .soap()
            .tracker()
            .get_root_folders(project)
            .await?)
    }

    pub async fn children(&self, project: Option<&str>, parent: &str) -> Result<Vec<Folder>> {
        Ok(self
            .polarion
            .soap()
            .tracker()
            .get_child_folders(project, parent)
            .await?)
    }

    pub async fn get(&self, project: Option<&str>, name: &str) -> Result<Folder> {
        Ok(self
            .polarion
            .soap()
            .tracker()
            .get_folder(project, name)
            .await?)
    }

    /// The folder containing an object (document, wiki page, ...).
    pub async fn containing(&self, object_uri: &str) -> Result<Folder> {
        Ok(self
            .polarion
            .soap()
            .tracker()
            .get_folder_for_uri(object_uri)
            .await?)
    }

    /// Create a folder under `parent` (or at the top level). A global folder (`project`
    /// `None`) needs [`crate::PolarionBuilder::allow_global_writes`].
    pub async fn create(
        &self,
        project: Option<&str>,
        parent: Option<&str>,
        name: &str,
        title: &str,
    ) -> Result<Folder> {
        self.polarion.check_unbounded_write("createFolder")?;
        self.polarion.check_write_scope("createFolder", project)?;
        Ok(self
            .polarion
            .soap()
            .tracker()
            .create_folder(project, parent, name, title)
            .await?)
    }
}
