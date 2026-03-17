#![allow(clippy::wrong_self_convention)]

use std::sync::OnceLock;

static FORCED_LANGUAGE: OnceLock<Language> = OnceLock::new();

#[cfg(not(target_os = "android"))]
fn system_locale() -> String {
    std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .unwrap_or_default()
}

#[cfg(target_os = "android")]
fn system_locale() -> String {
    (|| -> Option<String> {
        let ctx = ndk_context::android_context();
        let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) };
        vm.attach_current_thread(|env| -> Result<Option<String>, jni::errors::Error> {
            let locale_class = env.find_class(jni::jni_str!("java/util/Locale"))?;
            let locale = env
                .call_static_method(
                    locale_class,
                    jni::jni_str!("getDefault"),
                    jni::jni_sig!("()Ljava/util/Locale;"),
                    &[],
                )?
                .l()?;
            let lang_obj = env
                .call_method(
                    &locale,
                    jni::jni_str!("getLanguage"),
                    jni::jni_sig!("()Ljava/lang/String;"),
                    &[],
                )?
                .l()?;
            let jstr = env.cast_local::<jni::objects::JString>(lang_obj)?;
            let lang = jstr.try_to_string(env)?;
            Ok(Some(lang))
        })
        .ok()?
    })()
    .unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    En,
    De,
}

impl Language {
    pub const ALL: &[Language] = &[Language::En, Language::De];

    pub fn label(self) -> &'static str {
        match self {
            Language::En => "English",
            Language::De => "Deutsch",
        }
    }

    #[cfg_attr(target_os = "android", allow(dead_code))]
    pub fn set_forced(lang: Language) {
        let _ = FORCED_LANGUAGE.set(lang);
    }

    pub fn detect() -> Self {
        if let Some(&lang) = FORCED_LANGUAGE.get() {
            return lang;
        }
        let lang = system_locale().to_lowercase();
        if lang.starts_with("de") {
            Language::De
        } else {
            Language::En
        }
    }

    pub fn app_title(self) -> &'static str {
        match self {
            Language::En => "File Transfer",
            Language::De => "Datei-Transfer",
        }
    }

    pub fn starting(self) -> &'static str {
        match self {
            Language::En => "Starting...",
            Language::De => "Starte...",
        }
    }

    pub fn devices_found(self, count: usize) -> String {
        match self {
            Language::En => format!("{count} device(s) found on network"),
            Language::De => format!("{count} Gerät(e) im Netzwerk gefunden"),
        }
    }

    pub fn tab_devices(self) -> &'static str {
        match self {
            Language::En => "Devices",
            Language::De => "Geräte",
        }
    }

    pub fn tab_incoming(self) -> &'static str {
        match self {
            Language::En => "Incoming",
            Language::De => "Eingehend",
        }
    }

    pub fn tab_outgoing(self) -> &'static str {
        match self {
            Language::En => "Outgoing",
            Language::De => "Ausgehend",
        }
    }

    pub fn no_devices(self) -> &'static str {
        match self {
            Language::En => "No devices found on the network yet.",
            Language::De => "Noch keine Geräte im Netzwerk gefunden.",
        }
    }

    pub fn no_devices_hint(self) -> &'static str {
        match self {
            Language::En => {
                "Make sure another instance of Transfer is running on the same network."
            }
            Language::De => {
                "Stelle sicher, dass eine weitere Transfer-Instanz im selben Netzwerk läuft."
            }
        }
    }

    pub fn send_file(self) -> &'static str {
        match self {
            Language::En => "Send File",
            Language::De => "Datei senden",
        }
    }

    pub fn no_incoming(self) -> &'static str {
        match self {
            Language::En => "No incoming transfers.",
            Language::De => "Keine eingehenden Übertragungen.",
        }
    }

    pub fn from_label(self, name: &str) -> String {
        match self {
            Language::En => format!("From: {name}"),
            Language::De => format!("Von: {name}"),
        }
    }

    pub fn to_label(self, name: &str) -> String {
        match self {
            Language::En => format!("To: {name}"),
            Language::De => format!("An: {name}"),
        }
    }

    pub fn accept(self) -> &'static str {
        match self {
            Language::En => "Accept",
            Language::De => "Annehmen",
        }
    }

    pub fn reject(self) -> &'static str {
        match self {
            Language::En => "Reject",
            Language::De => "Ablehnen",
        }
    }

    pub fn no_outgoing(self) -> &'static str {
        match self {
            Language::En => "No outgoing transfers.",
            Language::De => "Keine ausgehenden Übertragungen.",
        }
    }

    pub fn no_outgoing_hint(self) -> &'static str {
        match self {
            Language::En => "Select a device and send a file to start.",
            Language::De => "Wähle ein Gerät und sende eine Datei.",
        }
    }

    pub fn status_pending(self) -> &'static str {
        match self {
            Language::En => "⏳ Pending",
            Language::De => "⏳ Ausstehend",
        }
    }

    pub fn status_completed(self) -> &'static str {
        match self {
            Language::En => "✓ Completed",
            Language::De => "✓ Abgeschlossen",
        }
    }

    pub fn status_rejected(self) -> &'static str {
        match self {
            Language::En => "✗ Rejected",
            Language::De => "✗ Abgelehnt",
        }
    }

    pub fn status_failed(self, err: &str) -> String {
        format!("✗ {err}")
    }

    #[cfg_attr(target_os = "android", allow(dead_code))]
    pub fn select_file_title(self) -> &'static str {
        match self {
            Language::En => "Select file to send",
            Language::De => "Datei zum Senden auswählen",
        }
    }

    #[cfg_attr(target_os = "android", allow(dead_code))]
    pub fn select_folder_title(self) -> &'static str {
        match self {
            Language::En => "Select save folder",
            Language::De => "Speicherordner auswählen",
        }
    }

    pub fn shared_file_ready(self, name: &str) -> String {
        match self {
            Language::En => format!("Shared file ready: {name}"),
            Language::De => format!("Geteilte Datei bereit: {name}"),
        }
    }

    pub fn shared_files_ready(self, count: usize) -> String {
        match self {
            Language::En => format!("{count} shared files ready to send"),
            Language::De => format!("{count} geteilte Dateien bereit zum Senden"),
        }
    }

    pub fn send_shared(self) -> &'static str {
        match self {
            Language::En => "Send",
            Language::De => "Senden",
        }
    }

    pub fn auto_accept_folder_label(self) -> &'static str {
        match self {
            Language::En => "Auto-accept folder:",
            Language::De => "Automatischer Empfangsordner:",
        }
    }

    pub fn auto_accept_folder_none(self) -> &'static str {
        match self {
            Language::En => "Not set (accept manually)",
            Language::De => "Nicht gesetzt (manuell annehmen)",
        }
    }

    pub fn select_auto_accept_folder(self) -> &'static str {
        match self {
            Language::En => "Select Folder",
            Language::De => "Ordner wählen",
        }
    }

    pub fn clear_auto_accept_folder(self) -> &'static str {
        match self {
            Language::En => "Clear",
            Language::De => "Entfernen",
        }
    }
}
