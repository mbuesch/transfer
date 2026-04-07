use crate::{
    app::{Language, TransferEvent, component_banner_sharedfile::SharedFileBanner, send_file},
    ipc::{DiscoveredDevice, OutgoingTransfer, TransferStatus},
    pick_file::pick_file_to_send,
};
use dioxus::prelude::*;
use std::{collections::HashMap, net::SocketAddr, path::PathBuf};
use tokio::sync::mpsc;
use uuid::Uuid;

#[component]
pub fn DevicesPanel(
    devices: Signal<HashMap<Uuid, DiscoveredDevice>>,
    event_tx: Signal<Option<mpsc::UnboundedSender<TransferEvent>>>,
    device_name: Signal<String>,
    next_send_id: Signal<u64>,
    outgoing_transfers: Signal<Vec<OutgoingTransfer>>,
    shared_files: Signal<Vec<PathBuf>>,
) -> Element {
    let lang = use_context::<Signal<Language>>();
    let l = *lang.read();
    let has_shared = !shared_files.read().is_empty();
    let devs = devices.read();
    if devs.is_empty() {
        return rsx! {
            if has_shared {
                SharedFileBanner { shared_files }
            }
            div { class: "empty",
                p { {l.no_devices()} }
                p { class: "hint", {l.no_devices_hint()} }
            }
        };
    }

    let dev_list: Vec<(Uuid, String, SocketAddr, u16)> = devs
        .values()
        .map(|d| (d.device_id, d.device_name.clone(), d.addr, d.transfer_port))
        .collect();

    rsx! {
        if has_shared {
            SharedFileBanner { shared_files }
        }
        div { class: "device-list",
            for (id, name, addr, _port) in dev_list {
                div { class: "device-card", key: "{id}",
                    div { class: "device-info",
                        span { class: "device-name", "{name}" }
                        span { class: "device-addr", "{addr.ip()}" }
                    }
                    button {
                        class: "send-btn",
                        onclick: {
                            let target_name = name.clone();
                            move |_| {
                                let target_name = target_name.clone();
                                let etx = event_tx.read().clone();
                                let sender = device_name.read().clone();
                                // Take shared files if present, otherwise open picker
                                let pending_shared: Vec<PathBuf> = shared_files.read().clone();
                                if !pending_shared.is_empty() {
                                    shared_files.write().clear();
                                    for path in pending_shared {
                                        let target_name = target_name.clone();
                                        let etx = etx.clone();
                                        let sender = sender.clone();
                                        let tid = {
                                            let mut id = next_send_id.write();
                                            let current = *id;
                                            *id += 1;
                                            current
                                        };
                                        spawn(async move {
                                            let filename = path
                                                .file_name()
                                                .map(|n| n.to_string_lossy().to_string())
                                                .unwrap_or_else(|| "file".to_string());
                                            outgoing_transfers
                                                .write()
                                                .push(OutgoingTransfer {
                                                    id: tid,
                                                    filename: filename.clone(),
                                                    file_size: 0,
                                                    target_device: target_name.clone(),
                                                    status: TransferStatus::Pending,
                                                });
                                            let meta_result = std::fs::metadata(&path);
                                            let can_send = match meta_result {
                                                Ok(m) if m.len() > 0 => {
                                                    let mut list = outgoing_transfers.write();
                                                    if let Some(t) =
                                                        list.iter_mut().find(|t| t.id == tid)
                                                    {
                                                        t.file_size = m.len();
                                                    }
                                                    true
                                                }
                                                Ok(_) => {
                                                    let mut list = outgoing_transfers.write();
                                                    if let Some(t) =
                                                        list.iter_mut().find(|t| t.id == tid)
                                                    {
                                                        t.status = TransferStatus::Failed(
                                                            "File is empty".to_string(),
                                                        );
                                                    }
                                                    false
                                                }
                                                Err(e) => {
                                                    let mut list = outgoing_transfers.write();
                                                    if let Some(t) =
                                                        list.iter_mut().find(|t| t.id == tid)
                                                    {
                                                        t.status = TransferStatus::Failed(
                                                            format!("Cannot read file: {e}"),
                                                        );
                                                    }
                                                    false
                                                }
                                            };
                                            if can_send && let Some(etx) = etx {
                                                tokio::spawn(async move {
                                                    if let Err(e) = send_file(addr, path, sender, tid, etx)
                                                        .await
                                                    {
                                                        log::warn!("Failed to send file: {e}");
                                                    }
                                                });
                                            }
                                        });
                                    }
                                } else {
                                    let tid = {
                                        let mut id = next_send_id.write();
                                        let current = *id;
                                        *id += 1;
                                        current
                                    };
                                    outgoing_transfers
                                        .write()
                                        .push(OutgoingTransfer {
                                            id: tid,
                                            filename: String::new(),
                                            file_size: 0,
                                            target_device: target_name.clone(),
                                            status: TransferStatus::Pending,
                                        });
                                    spawn(async move {
                                        let file = pick_file_to_send(*lang.read()).await;
                                        if let Some(path) = file {
                                            let filename = path
                                                .file_name()
                                                .map(|n| n.to_string_lossy().to_string())
                                                .unwrap_or_else(|| "file".to_string());
                                            {
                                                let mut list = outgoing_transfers.write();
                                                if let Some(t) =
                                                    list.iter_mut().find(|t| t.id == tid)
                                                {
                                                    t.filename = filename.clone();
                                                }
                                            }
                                            let meta_result = std::fs::metadata(&path);
                                            let can_send = match meta_result {
                                                Ok(m) if m.len() > 0 => {
                                                    let mut list = outgoing_transfers.write();
                                                    if let Some(t) = list.iter_mut().find(|t| t.id == tid) {
                                                        t.file_size = m.len();
                                                    }
                                                    true
                                                }
                                                Ok(_) => {
                                                    let mut list = outgoing_transfers.write();
                                                    if let Some(t) = list.iter_mut().find(|t| t.id == tid) {
                                                        t.status = TransferStatus::Failed(
                                                            "File is empty".to_string(),
                                                        );
                                                    }
                                                    false
                                                }
                                                Err(e) => {
                                                    let mut list = outgoing_transfers.write();
                                                    if let Some(t) = list.iter_mut().find(|t| t.id == tid) {
                                                        t.status = TransferStatus::Failed(
                                                            format!("Cannot read file: {e}"),
                                                        );
                                                    }
                                                    false
                                                }
                                            };
                                            if can_send && let Some(etx) = etx {
                                                tokio::spawn(async move {
                                                    if let Err(e) = send_file(addr, path, sender, tid, etx)
                                                        .await
                                                    {
                                                        log::warn!("Failed to send file: {e}");
                                                    }
                                                });
                                            }
                                        } else {
                                            outgoing_transfers.write().retain(|t| t.id != tid);
                                        }
                                    });
                                }
                            }
                        },
                        {if has_shared { l.send_shared() } else { l.send_file() }}
                    }
                }
            }
        }
    }
}
