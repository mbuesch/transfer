use crate::app::Language;
use dioxus::prelude::*;
use std::path::PathBuf;

#[component]
pub fn SharedFileBanner(shared_files: Signal<Vec<PathBuf>>) -> Element {
    let lang = use_context::<Signal<Language>>();
    let l = *lang.read();
    let files = shared_files.read();
    let names: Vec<String> = files
        .iter()
        .map(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "file".to_string())
        })
        .collect();
    let label = if names.len() == 1 {
        l.shared_file_ready(&names[0])
    } else {
        l.shared_files_ready(names.len())
    };

    rsx! {
        div { class: "shared-banner",
            span { class: "shared-banner-text", "{label}" }
            button {
                class: "shared-banner-dismiss",
                onclick: move |_| {
                    shared_files.write().clear();
                },
                "✕"
            }
        }
    }
}
