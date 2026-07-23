use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use tracing::info;

pub struct I18nManager {
    // Optimization Phase 1.2: Lazy loading of translations
    translations: Mutex<HashMap<String, HashMap<String, String>>>,
    current_lang: Arc<Mutex<String>>,
}

impl fmt::Debug for I18nManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("I18nManager")
            .field("current_lang", &self.current_lang)
            .field(
                "available_locales",
                &self.translations.lock().unwrap().keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl Default for I18nManager {
    fn default() -> Self {
        Self::new()
    }
}

impl I18nManager {
    pub fn new() -> Self {
        // [Optimization] Initialization is now O(1), zero JSON parsing during startup
        Self {
            translations: Mutex::new(HashMap::new()),
            current_lang: Arc::new(Mutex::new("zh-CN".to_string())),
        }
    }

    /// Lazily ensure a language is loaded
    fn ensure_lang_loaded(&self, lang: &str) {
        let mut translations = self.translations.lock().unwrap();
        if translations.contains_key(lang) {
            return;
        }

        // Only parse JSON when actually needed
        let raw_json = match lang {
            "zh-CN" => Some(include_str!("../../locales/zh-CN.json")),
            "en-US" => Some(include_str!("../../locales/en-US.json")),
            _ => None,
        };

        if let Some(json) = raw_json {
            if let Ok(map) = serde_json::from_str::<HashMap<String, String>>(json) {
                translations.insert(lang.to_string(), map);
                info!("Lazy loaded locale: {}", lang);
            }
        }
    }

    pub fn set_language(&self, lang: &str) {
        // Ensure data is available before switching
        self.ensure_lang_loaded(lang);

        let mut current = self.current_lang.lock().unwrap();
        *current = lang.to_string();
        info!("Language switched to: {}", lang);
    }

    pub fn t(&self, key: &str) -> String {
        let lang = self.current_lang.lock().unwrap().clone();

        // Ensure current language is loaded (Optimization: only happens once per session/lang)
        self.ensure_lang_loaded(&lang);

        let translations = self.translations.lock().unwrap();
        translations
            .get(&lang)
            .and_then(|m| m.get(key))
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }

    pub fn get_current_lang(&self) -> String {
        self.current_lang.lock().unwrap().clone()
    }

    pub fn get_all_translations(&self) -> HashMap<String, String> {
        let lang = self.current_lang.lock().unwrap().clone();
        self.ensure_lang_loaded(&lang);

        let translations = self.translations.lock().unwrap();
        translations.get(&lang).cloned().unwrap_or_default()
    }
}
