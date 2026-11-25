use bitnet_core::{AppError, ErrorCode, Scope};
use serde::{Deserialize, Serialize};

const FILE: &str = "gui-preferences.json";
const DIRECTORY: &str = "desktop";

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Preferences {
    show_advanced: bool,
}

pub(crate) fn show_advanced() -> bool {
    bitnet_platform::paths(Scope::User)
        .ok()
        .and_then(|paths| {
            bitnet_platform::read_private_file(&paths.data_dir.join(DIRECTORY).join(FILE)).ok()?
        })
        .and_then(|data| serde_json::from_slice::<Preferences>(&data).ok())
        .unwrap_or_default()
        .show_advanced
}

pub(crate) fn set_show_advanced(show_advanced: bool) -> Result<(), AppError> {
    let paths = bitnet_platform::paths(Scope::User)?;
    let directory = paths.data_dir.join(DIRECTORY);
    let encoded = serde_json::to_vec(&Preferences { show_advanced })
        .map_err(|_| AppError::new(ErrorCode::Internal, "Could not encode desktop preferences"))?;
    bitnet_platform::prepare_private_directory(&directory)?;
    bitnet_platform::write_private_file(&directory.join(FILE), &encoded)
}
