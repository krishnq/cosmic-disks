// SPDX-License-Identifier: GPL-3.0

//! XDG Desktop Portal helpers.

/// Opens the XDG Desktop Portal folder picker and returns the selected path,
/// or `None` if the user cancelled or the portal is unavailable.
pub async fn pick_folder() -> Option<String> {
    let title = crate::fl!("portal-pick-folder-title");
    let request = ashpd::desktop::file_chooser::SelectedFiles::open_file()
        .title(title.as_str())
        .directory(true)
        .send()
        .await
        .ok()?;

    let files = request.response().ok()?;
    let uri = files.uris().first()?;
    let path = url::Url::parse(uri.as_str()).ok()?.to_file_path().ok()?;
    Some(path.to_string_lossy().into_owned())
}
