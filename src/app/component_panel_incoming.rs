use crate::{
    app::{Language, render_status},
    pick_file::pick_save_folder,
    protocol::{
        packets::{IncomingTransfer, TransferStatus},
        transfer::TransferCommand,
    },
};
use bytesize::ByteSize;
use dioxus::prelude::*;
use tokio::sync::mpsc;

#[component]
pub fn IncomingPanel(
    transfers: Signal<Vec<IncomingTransfer>>,
    cmd_tx: Signal<Option<mpsc::UnboundedSender<TransferCommand>>>,
) -> Element {
    let lang = use_context::<Signal<Language>>();
    let l = *lang.read();
    let list = transfers.read();
    if list.is_empty() {
        return rsx! {
            div { class: "empty",
                p { {l.no_incoming()} }
            }
        };
    }

    rsx! {
        div { class: "transfer-list",
            for t in list.iter().rev() {
                div { class: "transfer-card", key: "{t.id}",
                    div { class: "transfer-info",
                        span { class: "transfer-filename", "{t.header.filename}" }
                        span { class: "transfer-size", "{ByteSize(t.header.file_size).to_string()}" }
                        span { class: "transfer-from", {l.from_label(&t.header.sender_name)} }
                    }
                    div { class: "transfer-right",
                        div { class: "transfer-status", {render_status(&t.status, l)} }
                        {
                            match &t.status {
                                TransferStatus::Pending => {
                                    let tid = t.id;
                                    rsx! {
                                        div { class: "transfer-actions",
                                            button {
                                                class: "accept-btn",
                                                onclick: move |_| {
                                                    spawn(async move {
                                                        let folder = pick_save_folder(*lang.read()).await;
                                                        if let Some(save_path) = folder && let Some(tx) = cmd_tx.read().as_ref() {
                                                            let _ = tx
                                                                .send(TransferCommand::AcceptTransfer {
                                                                    transfer_id: tid,
                                                                    save_path,
                                                                });
                                                        }
                                                    });
                                                },
                                                {l.accept()}
                                            }
                                            button {
                                                class: "reject-btn",
                                                onclick: move |_| {
                                                    if let Some(tx) = cmd_tx.read().as_ref() {
                                                        let _ = tx
                                                            .send(TransferCommand::RejectTransfer {
                                                                transfer_id: tid,
                                                            });
                                                    }
                                                },
                                                {l.reject()}
                                            }
                                        }
                                    }
                                }
                                _ => rsx! {},
                            }
                        }
                    }
                }
            }
        }
    }
}
