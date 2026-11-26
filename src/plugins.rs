use crate::library::Media;
use rhai::{AST, CallFnOptions, Engine, ImmutableString, OptimizationLevel, Scope};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

const ENTRY_POINT: &str = "on_track";
const MAX_SCRIPT_BYTES: u64 = 64 * 1024;

pub fn create_folder(library_path: &Path) -> io::Result<PathBuf> {
    let folder = library_path.with_file_name("Plugins");
    fs::create_dir_all(&folder)?;
    Ok(folder)
}

#[derive(Serialize, Deserialize)]
pub struct PluginConfig {
    pub path: PathBuf,
    pub enabled: bool,
}

impl PluginConfig {
    pub fn name(&self) -> String {
        self.path
            .file_stem()
            .unwrap_or(self.path.as_os_str())
            .to_string_lossy()
            .into_owned()
    }
}

pub struct Host {
    engine: Engine,
    entries: Vec<Entry>,
}

struct Entry {
    script: Option<AST>,
    status: String,
    output: String,
}

impl Host {
    pub fn load(configs: &[PluginConfig]) -> Self {
        let mut engine = Engine::new();
        engine
            .set_max_operations(20_000)
            .set_max_call_levels(12)
            .set_max_expr_depths(32, 32)
            .set_max_string_size(16 * 1024)
            .set_max_array_size(256)
            .set_max_map_size(128)
            .set_optimization_level(OptimizationLevel::None)
            .on_print(|_| {})
            .on_debug(|_, _, _| {});
        let entries = configs
            .iter()
            .map(|config| {
                let (script, status) = if config.enabled {
                    match compile(&engine, config) {
                        Ok(ast) => (Some(ast), "Ready".into()),
                        Err(error) => (None, error),
                    }
                } else {
                    (None, "Disabled".into())
                };
                Entry {
                    script,
                    status,
                    output: String::new(),
                }
            })
            .collect();
        Self { engine, entries }
    }

    pub fn track_changed(&mut self, media: &Media) {
        for entry in &mut self.entries {
            let Some(ast) = &entry.script else { continue };
            let result = self.engine.call_fn_with_options::<ImmutableString>(
                CallFnOptions::new().eval_ast(false),
                &mut Scope::new(),
                ast,
                ENTRY_POINT,
                (
                    media.title.clone(),
                    media.artist.clone(),
                    media.album.clone(),
                ),
            );
            match result {
                Ok(output) => {
                    entry.output = output
                        .chars()
                        .take(512)
                        .collect::<String>()
                        .trim()
                        .to_owned();
                    entry.status = "Ready".into();
                }
                Err(error) => {
                    entry.status = format!("Plugin error: {error}");
                    entry.output.clear();
                }
            }
        }
    }

    // The plugin list can grow in the same frame that adds a config, before the
    // host is reloaded. Indexing would panic while painting the plugin manager.
    pub fn status(&self, index: usize) -> &str {
        self.entries
            .get(index)
            .map_or("", |entry| entry.status.as_str())
    }

    pub fn output(&self, index: usize) -> &str {
        self.entries
            .get(index)
            .map_or("", |entry| entry.output.as_str())
    }
}

fn compile(engine: &Engine, config: &PluginConfig) -> Result<AST, String> {
    let mut bytes = Vec::new();
    File::open(&config.path)
        .and_then(|file| file.take(MAX_SCRIPT_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("Could not read plugin: {error}"))?;
    if bytes.len() as u64 > MAX_SCRIPT_BYTES {
        return Err("Plugin exceeds 64 KB".into());
    }
    let source =
        std::str::from_utf8(&bytes).map_err(|_| "Plugin must be saved as UTF-8".to_owned())?;
    let ast = engine
        .compile(source.trim_start_matches('\u{feff}'))
        .map_err(|error| error.to_string())?;
    if !ast
        .iter_functions()
        .any(|function| function.name == ENTRY_POINT && function.params.len() == 3)
    {
        return Err("Expected fn on_track(title, artist, album)".into());
    }
    Ok(ast)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stale_plugin_host_reports_no_status_or_output() {
        let host = Host::load(&[]);
        assert_eq!(host.status(0), "");
        assert_eq!(host.output(0), "");
    }

    #[test]
    fn disabled_plugins_keep_their_status_without_reading_the_script() {
        let configs = [PluginConfig {
            path: PathBuf::from("missing.rhai"),
            enabled: false,
        }];
        let host = Host::load(&configs);
        assert_eq!(host.status(0), "Disabled");
        assert_eq!(host.status(1), "");
        assert_eq!(host.output(0), "");
    }
}
