//! The model files of a repository: every `*.toml` file directly under
//! `.polint/models`, in name order, each read in full or not at all.

use std::path::Path;

/// The directory, under the repository root, that holds model files.
pub(crate) const MODELS_DIRECTORY: &str = ".polint/models";

/// The largest model file read. A larger one is reported and left out, so a
/// repository cannot make analysis read an unbounded file.
const MODEL_FILE_MAX_BYTES: u64 = 1_048_576;

/// One model file: its repository-relative name and its text, or why it
/// could not be read.
pub(crate) struct ModelFile {
    pub(crate) name: String,
    pub(crate) text: Result<String, String>,
}

/// Every model file of the repository at `root`, in name order. A file that
/// resolves outside the repository is not read.
pub(crate) fn model_files(root: &Path) -> Vec<ModelFile> {
    let directory = root.join(MODELS_DIRECTORY);
    let mut names = std::fs::read_dir(&directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "toml"))
        .filter_map(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .collect::<Vec<_>>();
    names.sort();
    names
        .into_iter()
        .map(|file| {
            let name = format!("{MODELS_DIRECTORY}/{file}");
            let text = crate::repo_fs::read_repo_file_to_string_with_limit(
                root,
                &name,
                MODEL_FILE_MAX_BYTES,
            )
            .map_err(|error| match error {
                crate::repo_fs::RepoFileReadError::TooLarge { max_bytes } => {
                    format!("larger than {max_bytes} bytes")
                }
                error => error.stable_reason().to_string(),
            });
            ModelFile { name, text }
        })
        .collect()
}
