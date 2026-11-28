//! The `.ics` export command; the writer lives in `rhythm_core::calendar_export`.
use crate::AppState;
use rhythm_core::calendar_export::CalendarExport;

#[tauri::command]
pub async fn calendar_export(
    state: tauri::State<'_, AppState>,
    destination: String,
) -> Result<CalendarExport, String> {
    state
        .calendar()
        .export(std::path::Path::new(&destination))
        .await
}
