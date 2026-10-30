//! Herramientas de solo lectura: raíces seguras y listado/lectura de archivos.

use crate::{Error, Result};
use std::path::{Path, PathBuf};

pub const READ_MAX_BYTES: u64 = 16 * 1024;
pub const LIST_MAX_ENTRIES: usize = 100;

#[derive(Clone, Debug)]
pub struct Roots {
    entries: Vec<PathBuf>,
}

impl Roots {
    pub fn new(entries: Vec<PathBuf>) -> Self {
        let entries = entries
            .iter()
            .filter_map(|p| p.canonicalize().ok())
            .collect();
        Self { entries }
    }

    pub fn macos_default() -> Result<Self> {
        let home = std::env::var_os("HOME").map(PathBuf::from).ok_or(Error::InvalidPath)?;
        let names = ["Documents", "Desktop", "Downloads", "Pictures", "Music", "Movies"];
        let mut entries = Vec::new();
        for name in names {
            let dir = home.join(name);
            if dir.is_dir() {
                entries.push(dir.canonicalize().unwrap_or(dir));
            }
        }
        Ok(Self { entries })
    }

    pub fn resolve(&self, path: &str) -> Option<PathBuf> {
        let candidate = Path::new(path);
        if !candidate.is_absolute() {
            return None;
        }
        let resolved = candidate.canonicalize().ok()?;
        if self.entries.iter().any(|root| resolved.starts_with(root)) {
            Some(resolved)
        } else {
            None
        }
    }
}

pub fn list_dir(roots: &Roots, path: &str) -> String {
    let Some(dir) = roots.resolve(path) else {
        return "fuera de las raíces autorizadas".to_string();
    };
    let Ok(read) = std::fs::read_dir(&dir) else {
        return "no se pudo listar".to_string();
    };
    let mut entries: Vec<_> = read.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    if entries.is_empty() {
        return "directorio vacío".to_string();
    }
    let mut lines = Vec::new();
    for entry in entries.iter().take(LIST_MAX_ENTRIES) {
        let name = entry.file_name().to_string_lossy().to_string();
        match entry.metadata() {
            Ok(m) if m.is_dir() => lines.push(format!("{name}\tdirectorio")),
            Ok(m) => lines.push(format!("{name}\tarchivo\t{}", m.len())),
            Err(_) => lines.push(format!("{name}\tdesconocido")),
        }
    }
    lines.join("\n")
}

pub fn read_file(roots: &Roots, path: &str) -> String {
    let Some(file) = roots.resolve(path) else {
        return "fuera de las raíces autorizadas".to_string();
    };
    let Ok(meta) = std::fs::metadata(&file) else {
        return "no se pudo leer".to_string();
    };
    if !meta.is_file() {
        return "no es un archivo regular".to_string();
    }
    if meta.len() > READ_MAX_BYTES {
        return format!("demasiado grande: {} bytes (máximo {})", meta.len(), READ_MAX_BYTES);
    }
    let Ok(bytes) = std::fs::read(&file) else {
        return "no se pudo leer".to_string();
    };
    if bytes.contains(&0) {
        return "binario/no legible".to_string();
    }
    match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => "binario/no legible".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "atlas-tools-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn list_dir_enumera_nombres_ordenados_dentro_de_la_raiz() {
        let dir = TempDir::new("list");
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        std::fs::create_dir(dir.path().join("b")).unwrap();
        let roots = Roots::new(vec![dir.path().to_path_buf()]);
        let out = list_dir(&roots, dir.path().to_str().unwrap());
        assert!(out.contains("a.txt"));
        assert!(out.contains("directorio"));
    }

    #[test]
    fn read_file_devuelve_contenido_utf8() {
        let dir = TempDir::new("read");
        std::fs::write(dir.path().join("f.txt"), "hola mundo").unwrap();
        let roots = Roots::new(vec![dir.path().to_path_buf()]);
        assert_eq!(
            read_file(&roots, dir.path().join("f.txt").to_str().unwrap()),
            "hola mundo"
        );
    }

    #[test]
    fn ruta_fuera_de_las_raices_se_rechaza() {
        let dir = TempDir::new("in");
        let outside = TempDir::new("out");
        std::fs::write(outside.path().join("s.txt"), "secreto").unwrap();
        let roots = Roots::new(vec![dir.path().to_path_buf()]);
        assert_eq!(
            read_file(&roots, outside.path().join("s.txt").to_str().unwrap()),
            "fuera de las raíces autorizadas"
        );
    }

    #[test]
    fn enlace_que_escapa_de_la_raiz_se_rechaza() {
        let dir = TempDir::new("link");
        let outside = TempDir::new("linkout");
        std::fs::write(outside.path().join("s.txt"), "secreto").unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("s.txt"),
            dir.path().join("link.txt"),
        )
        .unwrap();
        let roots = Roots::new(vec![dir.path().to_path_buf()]);
        assert_eq!(
            read_file(&roots, dir.path().join("link.txt").to_str().unwrap()),
            "fuera de las raíces autorizadas"
        );
    }

    #[test]
    fn archivo_demasiado_grande_se_rechaza_con_tamano() {
        let dir = TempDir::new("big");
        let big = dir.path().join("big.txt");
        std::fs::write(&big, vec![b'a'; (READ_MAX_BYTES + 1) as usize]).unwrap();
        let roots = Roots::new(vec![dir.path().to_path_buf()]);
        let out = read_file(&roots, big.to_str().unwrap());
        assert!(out.starts_with("demasiado grande"));
    }

    #[test]
    fn binario_con_nul_se_rechaza() {
        let dir = TempDir::new("bin");
        let bin = dir.path().join("b.bin");
        std::fs::write(&bin, [0u8, 1, 2, 3]).unwrap();
        let roots = Roots::new(vec![dir.path().to_path_buf()]);
        assert_eq!(
            read_file(&roots, bin.to_str().unwrap()),
            "binario/no legible"
        );
    }

    #[test]
    fn ruta_relativa_se_rechaza() {
        let dir = TempDir::new("rel");
        let roots = Roots::new(vec![dir.path().to_path_buf()]);
        assert_eq!(read_file(&roots, "f.txt"), "fuera de las raíces autorizadas");
    }
}
