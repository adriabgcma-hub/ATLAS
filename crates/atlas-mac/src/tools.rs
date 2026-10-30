//! Herramientas del dispositivo: raíces seguras, listado/lectura y escritura.

use crate::{Error, Result};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

pub const READ_MAX_BYTES: u64 = 16 * 1024;
pub const LIST_MAX_ENTRIES: usize = 100;
pub const HASH_MAX_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileInfo {
    pub exists: bool,
    pub size: Option<u64>,
    pub sha256: Option<String>,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

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
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or(Error::InvalidPath)?;
        let names = [
            "Documents",
            "Desktop",
            "Downloads",
            "Pictures",
            "Music",
            "Movies",
        ];
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

    /// Resuelve una ruta de escritura: canonicaliza el padre, exige contención
    /// y rechaza `.`/`..` y destinos que ya son enlaces. El destino puede no existir.
    pub fn resolve_new(&self, path: &str) -> Option<PathBuf> {
        let candidate = Path::new(path);
        if !candidate.is_absolute() {
            return None;
        }
        if candidate.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        }) {
            return None;
        }
        let name = candidate.file_name()?.to_str()?.to_string();
        let parent = candidate.parent()?.canonicalize().ok()?;
        if !parent.is_dir() || !self.entries.iter().any(|root| parent.starts_with(root)) {
            return None;
        }
        let target = parent.join(name);
        match std::fs::symlink_metadata(&target) {
            Ok(meta) if meta.file_type().is_symlink() => None,
            _ => Some(target),
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
        return format!(
            "demasiado grande: {} bytes (máximo {})",
            meta.len(),
            READ_MAX_BYTES
        );
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

pub fn file_info(roots: &Roots, path: &str) -> std::result::Result<FileInfo, String> {
    let Some(target) = roots.resolve_new(path) else {
        return Err("fuera de las raíces autorizadas".to_string());
    };
    let meta = match std::fs::symlink_metadata(&target) {
        Ok(meta) => meta,
        Err(_) => {
            return Ok(FileInfo {
                exists: false,
                size: None,
                sha256: None,
            })
        }
    };
    if !meta.is_file() {
        return Err("no es un archivo regular".to_string());
    }
    if meta.len() > HASH_MAX_BYTES {
        return Err("demasiado grande para planificar".to_string());
    }
    let hash = sha256_file(&target).map_err(|_| "no se pudo leer".to_string())?;
    Ok(FileInfo {
        exists: true,
        size: Some(meta.len()),
        sha256: Some(hash),
    })
}

fn temp_path(parent: &Path) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    parent.join(format!(".atlas-write-{}-{nanos}.tmp", std::process::id()))
}

fn stage(parent: &Path, content: &str) -> std::result::Result<PathBuf, String> {
    let temp = temp_path(parent);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|_| "no se pudo escribir".to_string())?;
    use std::io::Write;
    if file.write_all(content.as_bytes()).is_err() || file.sync_all().is_err() {
        let _ = std::fs::remove_file(&temp);
        return Err("no se pudo escribir".to_string());
    }
    Ok(temp)
}

pub fn write_file(roots: &Roots, path: &str, content: &str, expected_hash: Option<&str>) -> String {
    let Some(target) = roots.resolve_new(path) else {
        return "fuera de las raíces autorizadas".to_string();
    };
    let Some(parent) = target.parent() else {
        return "fuera de las raíces autorizadas".to_string();
    };
    let Some(expected) = expected_hash else {
        if std::fs::symlink_metadata(&target).is_ok() {
            return "ya existe".to_string();
        }
        let temp = match stage(parent, content) {
            Ok(temp) => temp,
            Err(message) => return message,
        };
        let published = std::fs::hard_link(&temp, &target);
        let _ = std::fs::remove_file(&temp);
        return match published {
            Ok(()) => format!(
                "Escrito «{path}» ({} B, sha256 {})",
                content.len(),
                &sha256_hex(content.as_bytes())[..12]
            ),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                "ya existe".to_string()
            }
            Err(_) => "no se pudo escribir".to_string(),
        };
    };
    let meta = match std::fs::symlink_metadata(&target) {
        Ok(meta) => meta,
        Err(_) => return "el archivo ya no existe".to_string(),
    };
    if !meta.is_file() {
        return "no es un archivo regular".to_string();
    }
    if meta.len() > HASH_MAX_BYTES {
        return "demasiado grande para planificar".to_string();
    }
    match sha256_file(&target) {
        Ok(current) if current == expected => {}
        Ok(_) => return "el archivo cambió desde la confirmación".to_string(),
        Err(_) => return "no se pudo leer".to_string(),
    }
    let temp = match stage(parent, content) {
        Ok(temp) => temp,
        Err(message) => return message,
    };
    match std::fs::rename(&temp, &target) {
        Ok(()) => format!(
            "Escrito «{path}» ({} B, sha256 {})",
            content.len(),
            &sha256_hex(content.as_bytes())[..12]
        ),
        Err(_) => {
            let _ = std::fs::remove_file(&temp);
            "no se pudo escribir".to_string()
        }
    }
}

pub fn rename(roots: &Roots, from: &str, to: &str) -> String {
    let source_meta = match std::fs::symlink_metadata(Path::new(from)) {
        Ok(meta) => meta,
        Err(_) => return "el origen no existe".to_string(),
    };
    if source_meta.file_type().is_symlink() {
        return "no es un archivo regular".to_string();
    }
    let Some(source) = roots.resolve(from) else {
        return "fuera de las raíces autorizadas".to_string();
    };
    let Some(destination) = roots.resolve_new(to) else {
        return "fuera de las raíces autorizadas".to_string();
    };
    let meta = match std::fs::symlink_metadata(&source) {
        Ok(meta) => meta,
        Err(_) => return "el origen no existe".to_string(),
    };
    if !meta.is_file() {
        return "no es un archivo regular".to_string();
    }
    if std::fs::symlink_metadata(&destination).is_ok() {
        return "el destino ya existe".to_string();
    }
    match std::fs::rename(&source, &destination) {
        Ok(()) => format!("Movido «{from}» → «{to}» ({} B)", meta.len()),
        Err(error) if error.raw_os_error() == Some(18) => {
            "no se puede mover entre volúmenes".to_string()
        }
        Err(_) => "no se pudo mover".to_string(),
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
        std::os::unix::fs::symlink(outside.path().join("s.txt"), dir.path().join("link.txt"))
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
        assert_eq!(
            read_file(&roots, "f.txt"),
            "fuera de las raíces autorizadas"
        );
    }

    #[test]
    fn escribir_crea_archivo_nuevo_y_falla_si_existe() {
        let dir = TempDir::new("write");
        let roots = Roots::new(vec![dir.path().to_path_buf()]);
        let target = dir.path().join("n.txt");
        let out = write_file(&roots, target.to_str().unwrap(), "hola", None);
        assert!(out.starts_with("Escrito «"));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "hola");
        let again = write_file(&roots, target.to_str().unwrap(), "otro", None);
        assert_eq!(again, "ya existe");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "hola");
    }

    #[test]
    fn sobrescribir_exige_el_hash_actual() {
        let dir = TempDir::new("overwrite");
        let roots = Roots::new(vec![dir.path().to_path_buf()]);
        let target = dir.path().join("n.txt");
        std::fs::write(&target, "viejo").unwrap();
        let hash = sha256_hex(b"viejo");
        let out = write_file(&roots, target.to_str().unwrap(), "nuevo", Some(&hash));
        assert!(out.starts_with("Escrito «"));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "nuevo");
        let stale = write_file(&roots, target.to_str().unwrap(), "tercero", Some(&hash));
        assert_eq!(stale, "el archivo cambió desde la confirmación");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "nuevo");
    }

    #[test]
    fn escribir_fuera_de_raices_o_por_enlace_se_rechaza() {
        let dir = TempDir::new("wsafe");
        let outside = TempDir::new("wsafe-out");
        let roots = Roots::new(vec![dir.path().to_path_buf()]);
        assert_eq!(
            write_file(
                &roots,
                outside.path().join("x.txt").to_str().unwrap(),
                "x",
                None
            ),
            "fuera de las raíces autorizadas"
        );
        std::os::unix::fs::symlink(outside.path().join("x.txt"), dir.path().join("link.txt"))
            .unwrap();
        assert_eq!(
            write_file(
                &roots,
                dir.path().join("link.txt").to_str().unwrap(),
                "x",
                None
            ),
            "fuera de las raíces autorizadas"
        );
    }

    #[test]
    fn file_info_devuelve_tamano_y_hash_y_rechaza_enlaces() {
        let dir = TempDir::new("info");
        let roots = Roots::new(vec![dir.path().to_path_buf()]);
        let target = dir.path().join("f.txt");
        std::fs::write(&target, "abc").unwrap();
        let info = file_info(&roots, target.to_str().unwrap()).unwrap();
        assert_eq!(info.size, Some(3));
        assert_eq!(info.sha256.as_deref(), Some(sha256_hex(b"abc").as_str()));
        let missing = file_info(&roots, dir.path().join("nuevo.txt").to_str().unwrap()).unwrap();
        assert!(!missing.exists);
        std::os::unix::fs::symlink(&target, dir.path().join("link.txt")).unwrap();
        assert_eq!(
            file_info(&roots, dir.path().join("link.txt").to_str().unwrap()),
            Err("fuera de las raíces autorizadas".to_string())
        );
    }

    #[test]
    fn renombrar_mueve_sin_pisar_y_dentro_de_raices() {
        let dir = TempDir::new("rename");
        let outside = TempDir::new("rename-out");
        let roots = Roots::new(vec![dir.path().to_path_buf()]);
        let from = dir.path().join("a.txt");
        let to = dir.path().join("b.txt");
        std::fs::write(&from, "hola").unwrap();
        let out = rename(&roots, from.to_str().unwrap(), to.to_str().unwrap());
        assert!(out.starts_with("Movido «"));
        assert!(!from.exists());
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "hola");
        let other = dir.path().join("c.txt");
        std::fs::write(&other, "x").unwrap();
        assert_eq!(
            rename(&roots, other.to_str().unwrap(), to.to_str().unwrap()),
            "el destino ya existe"
        );
        assert_eq!(
            rename(
                &roots,
                other.to_str().unwrap(),
                outside.path().join("d.txt").to_str().unwrap()
            ),
            "fuera de las raíces autorizadas"
        );
    }

    #[test]
    fn sha256_coincide_con_los_vectores_de_referencia() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
