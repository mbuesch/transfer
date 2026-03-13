use crate::app::Language;
use dioxus::prelude::*;

#[component]
pub fn LanguageSelector(lang: Signal<Language>) -> Element {
    let current = *lang.read();
    rsx! {
        select {
            class: "lang-select",
            value: "{current:?}",
            onchange: move |evt: Event<FormData>| {
                let val = evt.value();
                let new_lang = match val.as_str() {
                    "De" => Language::De,
                    _ => Language::En,
                };
                lang.set(new_lang);
            },
            for l in Language::ALL {
                option { value: "{l:?}", selected: *l == current, {l.label()} }
            }
        }
    }
}
