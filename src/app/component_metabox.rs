use crate::app::Language;
use dioxus::prelude::*;

#[component]
pub fn MetaBox(lang: Signal<Language>, transfer_step_status: Signal<Option<String>>) -> Element {
    let status: String = transfer_step_status.read().cloned().unwrap_or_default();
    let current_lang = *lang.read();
    let version = env!("CARGO_PKG_VERSION");

    rsx! {
        div { class: "meta-box",
            small { class: "transfer-step-status", "{status}" }
            select {
                class: "lang-select",
                value: "{current_lang:?}",
                onchange: move |evt: Event<FormData>| {
                    let val = evt.value();
                    let new_lang = match val.as_str() {
                        "De" => Language::De,
                        _ => Language::En,
                    };
                    lang.set(new_lang);
                },
                for l in Language::ALL {
                    option { value: "{l:?}", selected: *l == current_lang, {l.label()} }
                }
            }
            small { class: "version", "{version}" }
        }
    }
}
