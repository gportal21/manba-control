//! Perfiles guardados y configuración general.
//!
//! ```text
//! %APPDATA%\manba-control\
//!   config.toml              perfil base y opciones generales
//!   perfiles\<nombre>.toml   un archivo por perfil
//! ```

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::profile::{Profile, TEMPLATE};

/// Perfil que siempre existe y no se puede renombrar ni borrar.
pub const DEFAULT_PROFILE: &str = "Predeterminado";

/// Los nombres viajan en la memoria compartida con un tamaño fijo.
pub const MAX_NAME_BYTES: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    /// Perfil que se usa cuando ningún juego tiene uno propio.
    #[serde(rename = "perfil_base")]
    pub base_profile: String,
    /// Cambiar de perfil y pausar desde el mando (Back + cruceta).
    #[serde(rename = "atajos")]
    pub shortcuts: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            base_profile: DEFAULT_PROFILE.to_string(),
            shortcuts: true,
        }
    }
}

/// Comprueba que un nombre de perfil sirva como nombre de archivo.
pub fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("el nombre del perfil no puede estar vacío".into());
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(format!("el nombre del perfil es demasiado largo (máximo {MAX_NAME_BYTES} letras)"));
    }
    if name.starts_with('.') || name.ends_with('.') {
        return Err("el nombre del perfil no puede empezar ni terminar en punto".into());
    }
    if let Some(c) = name
        .chars()
        .find(|c| c.is_control() || r#"\/:*?"<>|"#.contains(*c))
    {
        return Err(format!("el nombre del perfil no puede contener '{c}'"));
    }
    Ok(name.to_string())
}

#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// `%APPDATA%\manba-control`.
    pub fn from_appdata() -> Option<Self> {
        let appdata = std::env::var_os("APPDATA")?;
        Some(Self::new(PathBuf::from(appdata).join("manba-control")))
    }

    pub fn profiles_dir(&self) -> PathBuf {
        self.root.join("perfiles")
    }

    fn config_path(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    fn profile_path(&self, name: &str) -> Result<PathBuf, String> {
        Ok(self.profiles_dir().join(format!("{}.toml", validate_name(name)?)))
    }

    /// Prepara la carpeta. La primera vez convierte el `perfil.toml` de
    /// versiones anteriores en el perfil predeterminado.
    pub fn init(&self) -> Result<(), String> {
        let dir = self.profiles_dir();
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let default = self.profile_path(DEFAULT_PROFILE)?;
        if default.exists() {
            return Ok(());
        }
        let legacy = self.root.join("perfil.toml");
        if legacy.exists() {
            std::fs::rename(&legacy, &default).map_err(|e| e.to_string())
        } else {
            std::fs::write(&default, TEMPLATE).map_err(|e| e.to_string())
        }
    }

    /// Nombres de los perfiles, con el predeterminado primero.
    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.profiles_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension()? != "toml" {
                    return None;
                }
                let name = path.file_stem()?.to_str()?.to_string();
                validate_name(&name).ok()
            })
            .collect();
        names.sort_by_key(|name| (name != DEFAULT_PROFILE, name.to_lowercase()));
        names
    }

    pub fn exists(&self, name: &str) -> bool {
        self.profile_path(name).is_ok_and(|path| path.exists())
    }

    pub fn load(&self, name: &str) -> Result<Profile, String> {
        Profile::load(&self.profile_path(name)?)
    }

    pub fn save(&self, name: &str, profile: &Profile) -> Result<(), String> {
        profile.save(&self.profile_path(name)?)
    }

    /// Crea un perfil nuevo, vacío o copiado de `from`.
    pub fn create(&self, name: &str, from: Option<&str>) -> Result<String, String> {
        let name = validate_name(name)?;
        if self.exists(&name) {
            return Err(format!("ya existe un perfil llamado '{name}'"));
        }
        let mut profile = match from {
            Some(source) => self.load(source)?,
            None => Profile::default(),
        };
        // Los juegos se quedan en el original: un juego solo debe activar un perfil.
        profile.games.clear();
        self.save(&name, &profile)?;
        Ok(name)
    }

    pub fn rename(&self, old: &str, new: &str) -> Result<String, String> {
        if old == DEFAULT_PROFILE {
            return Err("el perfil predeterminado no se puede renombrar".into());
        }
        let new = validate_name(new)?;
        // Permite cambiar solo mayúsculas en un sistema de archivos que no las distingue.
        if self.exists(&new) && !old.eq_ignore_ascii_case(&new) {
            return Err(format!("ya existe un perfil llamado '{new}'"));
        }
        std::fs::rename(self.profile_path(old)?, self.profile_path(&new)?)
            .map_err(|e| e.to_string())?;
        let mut config = self.load_config();
        if config.base_profile == old {
            config.base_profile = new.clone();
            self.save_config(&config)?;
        }
        Ok(new)
    }

    pub fn delete(&self, name: &str) -> Result<(), String> {
        if name == DEFAULT_PROFILE {
            return Err("el perfil predeterminado no se puede borrar".into());
        }
        std::fs::remove_file(self.profile_path(name)?).map_err(|e| e.to_string())?;
        let mut config = self.load_config();
        if config.base_profile == name {
            config.base_profile = DEFAULT_PROFILE.to_string();
            self.save_config(&config)?;
        }
        Ok(())
    }

    /// Si el archivo falta o no se puede leer, se usan los valores por defecto.
    pub fn load_config(&self) -> AppConfig {
        std::fs::read_to_string(self.config_path())
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save_config(&self, config: &AppConfig) -> Result<(), String> {
        let body = toml::to_string_pretty(config).map_err(|e| e.to_string())?;
        write_atomic(&self.config_path(), &body)
    }

    /// Cambia cuando se crea, borra o modifica cualquier perfil o la
    /// configuración. Sirve para recargar solo cuando hace falta.
    pub fn signature(&self) -> Vec<(String, Option<SystemTime>)> {
        let modified = |path: &Path| std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let mut signature: Vec<_> = self
            .list()
            .into_iter()
            .map(|name| {
                let time = self.profile_path(&name).ok().and_then(|p| modified(&p));
                (name, time)
            })
            .collect();
        signature.push((String::new(), modified(&self.config_path())));
        signature
    }
}

/// Escribe en un temporal y renombra, para no dejar nunca un archivo a medias.
pub(crate) fn write_atomic(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, contents)
        .and_then(|()| std::fs::rename(&temp, path))
        .map_err(|e| format!("no se pudo guardar {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(tag: &str) -> Store {
        let root = std::env::temp_dir().join(format!("manba-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        Store::new(root)
    }

    #[test]
    fn init_creates_default_profile() {
        let store = temp_store("init");
        store.init().unwrap();
        assert_eq!(store.list(), [DEFAULT_PROFILE]);
        assert_eq!(store.load(DEFAULT_PROFILE).unwrap(), Profile::default());
        let _ = std::fs::remove_dir_all(&store.root);
    }

    #[test]
    fn init_migrates_legacy_profile() {
        let store = temp_store("legacy");
        std::fs::create_dir_all(&store.root).unwrap();
        std::fs::write(store.root.join("perfil.toml"), "intervalo_ms = 4\n").unwrap();
        store.init().unwrap();
        assert_eq!(store.load(DEFAULT_PROFILE).unwrap().poll_interval_ms, 4);
        assert!(!store.root.join("perfil.toml").exists());
        let _ = std::fs::remove_dir_all(&store.root);
    }

    #[test]
    fn create_duplicate_rename_delete() {
        let store = temp_store("crud");
        store.init().unwrap();
        let mut base = Profile::default();
        base.poll_interval_ms = 8;
        base.games.push("juego.exe".into());
        store.save(DEFAULT_PROFILE, &base).unwrap();

        store.create("Copia", Some(DEFAULT_PROFILE)).unwrap();
        let copy = store.load("Copia").unwrap();
        assert_eq!(copy.poll_interval_ms, 8);
        assert!(copy.games.is_empty(), "los juegos no se copian");
        assert!(store.create("Copia", None).is_err(), "no se duplican nombres");

        store.save_config(&AppConfig { base_profile: "Copia".into(), shortcuts: false }).unwrap();
        store.rename("Copia", "Shooter").unwrap();
        assert_eq!(store.load_config().base_profile, "Shooter");
        assert!(store.list().contains(&"Shooter".to_string()));

        store.delete("Shooter").unwrap();
        assert_eq!(store.load_config().base_profile, DEFAULT_PROFILE);
        assert!(store.delete(DEFAULT_PROFILE).is_err());
        assert!(store.rename(DEFAULT_PROFILE, "Otro").is_err());
        let _ = std::fs::remove_dir_all(&store.root);
    }

    #[test]
    fn names_are_validated() {
        assert_eq!(validate_name("  Carreras ").unwrap(), "Carreras");
        assert!(validate_name("").is_err());
        assert!(validate_name("a/b").is_err());
        assert!(validate_name("con:dos").is_err());
        assert!(validate_name(".oculto").is_err());
        assert!(validate_name(&"x".repeat(MAX_NAME_BYTES + 1)).is_err());
    }

    #[test]
    fn signature_changes_with_new_profile() {
        let store = temp_store("sig");
        store.init().unwrap();
        let before = store.signature();
        store.create("Nuevo", None).unwrap();
        assert_ne!(before, store.signature());
        let _ = std::fs::remove_dir_all(&store.root);
    }
}
