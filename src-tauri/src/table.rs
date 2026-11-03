//! Ventana de tablas del chat: la tabla vive en memoria y se exporta solo a
//! `~/Documents/Atlas/tablas/`. El webview nunca elige una ruta.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

const MAX_ROWS: usize = 2000;
const MAX_COLUMNS: usize = 64;
const MAX_CELL_CHARS: usize = 2000;
const MAX_TITLE_CHARS: usize = 120;
const MAX_PAYLOAD_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TablePayload {
    pub title: String,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

struct Tables {
    next: u64,
    items: HashMap<String, TablePayload>,
}

pub struct TableState {
    tables: Mutex<Tables>,
}

impl TableState {
    pub fn new() -> Self {
        Self {
            tables: Mutex::new(Tables {
                next: 0,
                items: HashMap::new(),
            }),
        }
    }

    pub fn remove(&self, label: &str) {
        if let Ok(mut tables) = self.tables.lock() {
            tables.items.remove(label);
        }
    }
}

impl Default for TableState {
    fn default() -> Self {
        Self::new()
    }
}

fn validate(payload: &TablePayload) -> std::result::Result<(), String> {
    if payload.title.chars().count() > MAX_TITLE_CHARS {
        return Err("título demasiado largo".to_owned());
    }
    if payload.headers.is_empty() || payload.headers.len() > MAX_COLUMNS {
        return Err("número de columnas inválido".to_owned());
    }
    if payload.rows.len() > MAX_ROWS {
        return Err("demasiadas filas".to_owned());
    }
    if payload
        .headers
        .iter()
        .any(|cell| cell.chars().count() > MAX_CELL_CHARS)
    {
        return Err("celda demasiado larga".to_owned());
    }
    for row in &payload.rows {
        if row.len() != payload.headers.len() {
            return Err("fila con columnas distintas".to_owned());
        }
        if row.iter().any(|cell| cell.chars().count() > MAX_CELL_CHARS) {
            return Err("celda demasiado larga".to_owned());
        }
    }
    let size = serde_json::to_vec(payload)
        .map_err(|_| "tabla no serializable".to_owned())?
        .len();
    if size > MAX_PAYLOAD_BYTES {
        return Err("tabla demasiado grande".to_owned());
    }
    Ok(())
}

#[tauri::command]
pub fn open_table_window(
    app: AppHandle,
    state: State<'_, TableState>,
    payload: TablePayload,
) -> std::result::Result<String, String> {
    validate(&payload)?;
    let label = {
        let mut tables = state
            .tables
            .lock()
            .map_err(|_| "estado de tablas no disponible".to_owned())?;
        tables.next = tables.next.wrapping_add(1);
        let label = format!("table-{}", tables.next);
        tables.items.insert(label.clone(), payload);
        label
    };
    let window = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("table.html".into()))
        .title("Atlas · Tabla")
        .inner_size(920.0, 640.0)
        .min_inner_size(480.0, 320.0)
        .resizable(true)
        .build()
        .map_err(|_| "no se pudo abrir la ventana de tabla".to_owned());
    match window {
        Ok(window) => {
            let _ = window.set_focus();
            Ok(label)
        }
        Err(error) => {
            state.remove(&label);
            Err(error)
        }
    }
}

#[tauri::command]
pub fn table_payload(
    state: State<'_, TableState>,
    label: String,
) -> std::result::Result<TablePayload, String> {
    state
        .tables
        .lock()
        .map_err(|_| "estado de tablas no disponible".to_owned())?
        .items
        .get(&label)
        .cloned()
        .ok_or_else(|| "tabla no encontrada".to_owned())
}

#[tauri::command]
pub fn export_table_csv(
    app: AppHandle,
    state: State<'_, TableState>,
    label: String,
) -> std::result::Result<String, String> {
    let payload = {
        let tables = state
            .tables
            .lock()
            .map_err(|_| "estado de tablas no disponible".to_owned())?;
        tables
            .items
            .get(&label)
            .cloned()
            .ok_or_else(|| "tabla no encontrada".to_owned())?
    };
    validate(&payload)?;
    let documents = app
        .path()
        .document_dir()
        .map_err(|_| "no hay carpeta de documentos".to_owned())?;
    let dir = documents.join("Atlas").join("tablas");
    std::fs::create_dir_all(&dir).map_err(|_| "no se pudo crear la carpeta".to_owned())?;
    let name = format!("{}-{}.csv", slug(&payload.title), timestamp());
    let path = dir.join(&name);
    let csv = csv_text(&payload);
    let temp = dir.join(format!(".{name}.tmp"));
    std::fs::write(&temp, csv).map_err(|_| "no se pudo escribir el CSV".to_owned())?;
    std::fs::rename(&temp, &path).map_err(|_| {
        let _ = std::fs::remove_file(&temp);
        "no se pudo publicar el CSV".to_owned()
    })?;
    Ok(path.display().to_string())
}

fn csv_text(payload: &TablePayload) -> String {
    let mut lines = Vec::with_capacity(payload.rows.len() + 1);
    lines.push(
        payload
            .headers
            .iter()
            .map(|cell| csv_field(cell))
            .collect::<Vec<_>>()
            .join(","),
    );
    for row in &payload.rows {
        lines.push(
            row.iter()
                .map(|cell| csv_field(cell))
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    format!("\u{feff}{}\r\n", lines.join("\r\n"))
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

fn slug(value: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    let out = out.trim_matches('-');
    if out.is_empty() {
        "tabla".to_owned()
    } else {
        out.chars().take(60).collect()
    }
}

fn timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0);
    let days = (seconds / 86_400) as i64;
    let secs = seconds % 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    format!(
        "{year:04}{m:02}{d:02}-{:02}{:02}{:02}",
        secs / 3_600,
        (secs % 3_600) / 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> TablePayload {
        TablePayload {
            title: "Ventas 2026".to_owned(),
            headers: vec!["mes".to_owned(), "total".to_owned()],
            rows: vec![
                vec!["enero".to_owned(), "10".to_owned()],
                vec!["febrero".to_owned(), "20".to_owned()],
            ],
        }
    }

    #[test]
    fn el_csv_escapa_comas_comillas_y_saltos() {
        let text = csv_text(&TablePayload {
            title: "t".to_owned(),
            headers: vec!["a,b".to_owned(), "c\"d".to_owned(), "e\nf".to_owned()],
            rows: vec![],
        });
        assert!(text.starts_with('\u{feff}'));
        assert!(text.contains("\"a,b\""), "{text}");
        assert!(text.contains("\"c\"\"d\""), "{text}");
        assert!(text.contains("\"e\nf\""), "{text}");
    }

    #[test]
    fn el_nombre_saneado_no_sale_de_las_tablas() {
        assert_eq!(slug("Ventas 2026 / Q1"), "ventas-2026-q1");
        assert_eq!(slug("///"), "tabla");
        assert_eq!(slug(""), "tabla");
    }

    #[test]
    fn la_validacion_acota_filas_columnas_y_celdas() {
        assert!(validate(&payload()).is_ok());
        let mut bad = payload();
        bad.rows[0].pop();
        assert!(validate(&bad).is_err());
        let mut bad = payload();
        bad.headers = vec!["x".repeat(MAX_CELL_CHARS + 1)];
        bad.rows.clear();
        assert!(validate(&bad).is_err());
        let mut bad = payload();
        bad.rows = vec![payload().rows[0].clone(); MAX_ROWS + 1];
        assert!(validate(&bad).is_err());
    }
}
