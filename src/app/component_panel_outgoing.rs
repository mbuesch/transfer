use crate::{
    app::{Language, render_status},
    ipc::OutgoingTransfer,
};
use bytesize::ByteSize;
use dioxus::prelude::*;

#[component]
pub fn OutgoingPanel(transfers: Signal<Vec<OutgoingTransfer>>) -> Element {
    let lang = use_context::<Signal<Language>>();
    let l = *lang.read();
    let list = transfers.read();
    if list.is_empty() {
        return rsx! {
            div { class: "empty",
                p { {l.no_outgoing()} }
                p { class: "hint", {l.no_outgoing_hint()} }
            }
        };
    }

    rsx! {
        div { class: "transfer-list",
            for t in list.iter().rev() {
                div { class: "transfer-card", key: "{t.id}",
                    div { class: "transfer-info",
                        span { class: "transfer-filename",
                            {if t.filename.is_empty() { "…" } else { t.filename.as_str() }}
                        }
                        if t.file_size > 0 {
                            span { class: "transfer-size", "{ByteSize(t.file_size)}" }
                        }
                        span { class: "transfer-from", {l.to_label(&t.target_device)} }
                    }
                    div { class: "transfer-status", {render_status(&t.status, l)} }
                }
            }
        }
    }
}
