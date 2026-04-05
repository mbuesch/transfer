use crate::{
    app::{
        component_langselect::LanguageSelector, component_panel_devices::DevicesPanel,
        component_panel_incoming::IncomingPanel, component_panel_outgoing::OutgoingPanel,
    },
    device_name::get_device_name,
    l10n::Language,
    protocol::{
        discovery::{
            DeviceMap, broadcast_presence_ipv4, broadcast_presence_ipv6,
            compute_discovery_checksum, create_ipv4_listener_socket, create_ipv6_listener_socket,
            listen_for_devices, prune_stale_devices,
        },
        packets::{
            BROADCAST_INTERVAL, DiscoveredDevice, DiscoveryPacket, IpSupport, OutgoingTransfer,
            TRANSFER_PORT, TransferStatus,
        },
        transfer::{TransferCommand, TransferEvent, run_transfer_server, send_file},
    },
};
use anyhow as ah;
use bytesize::ByteSize;
use dioxus::prelude::*;
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    net::UdpSocket,
    sync::{Mutex, mpsc},
    time::{sleep, timeout},
};

mod component_banner_sharedfile;
mod component_langselect;
mod component_panel_devices;
mod component_panel_incoming;
mod component_panel_outgoing;

const CSS: &str = include_str!("app/style.css");

const DISPLAY_TIMEOUT: Duration = Duration::from_secs(120);

/// Retrieve file paths shared via Android's share intent (ACTION_SEND / ACTION_SEND_MULTIPLE).
#[cfg(target_os = "android")]
fn get_shared_files() -> Vec<PathBuf> {
    (|| -> Option<Vec<PathBuf>> {
        let ctx = ndk_context::android_context();
        let vm = unsafe { ::jni::JavaVM::from_raw(ctx.vm().cast()) };
        vm.attach_current_thread(
            |env| -> Result<Option<Vec<PathBuf>>, ::jni::errors::Error> {
                let activity =
                    unsafe { ::jni::objects::JObject::from_raw(env, ctx.context().cast()) };
                let class = env.get_object_class(&activity)?;
                let result = env.call_static_method(
                    &class,
                    ::jni::jni_str!("getSharedFiles"),
                    ::jni::jni_sig!("()[Ljava/lang/String;"),
                    &[],
                )?;
                let jobj = result.l()?;
                if jobj.is_null() {
                    return Ok(Some(vec![]));
                }
                let array =
                    env.cast_local::<::jni::objects::JObjectArray<::jni::objects::JString>>(jobj)?;
                let len = array.len(env)?;
                let mut paths = vec![];
                for i in 0..len {
                    let elem: ::jni::objects::JString = array.get_element(env, i)?;
                    if !elem.is_null() {
                        let s = elem.try_to_string(env)?;
                        paths.push(PathBuf::from(s));
                    }
                }
                // Clear the shared files after retrieval
                let _ = env.call_static_method(
                    &class,
                    ::jni::jni_str!("clearSharedFiles"),
                    ::jni::jni_sig!("()V"),
                    &[],
                );
                Ok(Some(paths))
            },
        )
        .ok()?
    })()
    .unwrap_or_default()
}

#[cfg(not(target_os = "android"))]
fn get_shared_files() -> Vec<PathBuf> {
    vec![]
}

#[component]
pub fn App() -> Element {
    let detected_lang = Language::detect();
    let lang = use_context_provider(|| Signal::new(detected_lang));
    let device_id = uuid::Uuid::new_v4().to_string();
    let device_name = use_signal(get_device_name);

    let mut devices = use_signal(HashMap::new);
    let mut incoming_transfers = use_signal(Vec::new);
    let mut outgoing_transfers: Signal<Vec<OutgoingTransfer>> = use_signal(Vec::new);
    let mut status_msg = use_signal(|| detected_lang.starting().to_string());
    let mut active_tab = use_signal(|| 0_usize);
    let next_send_id = use_signal(|| 1_u64);
    let shared_files: Signal<Vec<PathBuf>> = use_signal(Vec::new);

    // Folder for automatic accepting of incoming files (None = manual accept)
    let auto_accept_folder: Signal<Option<PathBuf>> = use_signal(|| None);

    // Channel for transfer commands (UI -> transfer server)
    let mut cmd_tx = use_signal(|| None);
    // Channel for transfer events (transfer server -> UI)
    let mut event_tx_holder = use_signal(|| None);

    // Start background services once
    use_hook({
        let device_id = device_id.clone();
        move || {
            let device_map = Arc::new(Mutex::new(HashMap::new()));

            // Start discovery broadcasters (IPv4 + IPv6)
            let packet = DiscoveryPacket {
                device_id: device_id.clone(),
                device_name: device_name.read().clone(),
                transfer_port: TRANSFER_PORT,
                checksum: compute_discovery_checksum(
                    &device_id,
                    &device_name.read().clone(),
                    TRANSFER_PORT,
                ),
            };
            if IpSupport::ipv4() {
                spawn({
                    let packet = packet.clone();
                    async move {
                        loop {
                            broadcast_presence_ipv4(&packet).await;
                            sleep(BROADCAST_INTERVAL).await;
                        }
                    }
                });
            }
            if IpSupport::ipv6() {
                spawn({
                    let packet = packet.clone();
                    async move {
                        loop {
                            broadcast_presence_ipv6(&packet).await;
                            sleep(BROADCAST_INTERVAL).await;
                        }
                    }
                });
            }

            // Start discovery listeners (IPv4 + IPv6)
            if IpSupport::ipv4() {
                spawn({
                    let device_map = Arc::clone(&device_map);
                    let device_id = device_id.clone();
                    run_discovery_listener(
                        || async { create_ipv4_listener_socket().await },
                        "IPv4",
                        device_map,
                        device_id,
                    )
                });
            }
            if IpSupport::ipv6() {
                spawn({
                    let device_map = Arc::clone(&device_map);
                    let device_id = device_id.clone();
                    run_discovery_listener(
                        || async { create_ipv6_listener_socket().await },
                        "IPv6",
                        device_map,
                        device_id,
                    )
                });
            }

            // Periodic device-map sync to UI + pruning
            spawn({
                let device_map = Arc::clone(&device_map);
                async move {
                    loop {
                        sleep(Duration::from_secs(1)).await;

                        prune_stale_devices(&device_map).await;

                        let snapshot: HashMap<String, DiscoveredDevice> =
                            device_map.lock().await.clone();
                        let count = snapshot.len();
                        devices.set(snapshot);
                        let l = *lang.read();
                        status_msg.set(l.devices_found(count));
                    }
                }
            });

            // Transfer server
            let (ctx, crx) = mpsc::unbounded_channel::<TransferCommand>();

            // Poll for files shared via Android share intent
            spawn({
                let mut shared_files = shared_files;
                async move {
                    // Brief delay to let the Java side finish processing the share intent
                    sleep(Duration::from_millis(500)).await;
                    loop {
                        let files = tokio::task::spawn_blocking(get_shared_files)
                            .await
                            .unwrap_or_default();
                        if !files.is_empty() {
                            shared_files.set(files);
                        }
                        sleep(Duration::from_secs(1)).await;
                    }
                }
            });
            let (etx, mut erx) = mpsc::unbounded_channel::<TransferEvent>();

            cmd_tx.write().replace(ctx);
            event_tx_holder.write().replace(etx.clone());

            spawn(async move {
                run_transfer_server(TRANSFER_PORT, etx, crx).await;
            });

            // Process transfer events
            spawn(async move {
                while let Some(event) = erx.recv().await {
                    match event {
                        TransferEvent::IncomingRequest(t) => {
                            let transfer_id = t.id;
                            incoming_transfers.write().push(t);
                            active_tab.set(1);
                            // Auto-accept if an incoming folder has been selected
                            if let Some(folder) = auto_accept_folder.read().clone()
                                && let Some(tx) = cmd_tx.read().as_ref()
                            {
                                let _ = tx.send(TransferCommand::AcceptTransfer {
                                    transfer_id,
                                    save_path: folder,
                                });
                            }
                        }
                        TransferEvent::Progress {
                            transfer_id,
                            bytes_transferred,
                            total,
                        } => {
                            let mut list = incoming_transfers.write();
                            if let Some(t) = list.iter_mut().find(|t| t.id == transfer_id) {
                                t.status = TransferStatus::InProgress {
                                    bytes_transferred,
                                    total,
                                };
                            }
                        }
                        TransferEvent::Completed {
                            transfer_id,
                            save_path,
                        } => {
                            let mut should_purge = false;
                            {
                                let mut list = incoming_transfers.write();
                                if let Some(t) = list.iter_mut().find(|t| t.id == transfer_id) {
                                    t.status = TransferStatus::Completed;
                                    t.save_path = save_path;
                                    should_purge = true;
                                }
                            }
                            if should_purge {
                                spawn(async move {
                                    sleep(DISPLAY_TIMEOUT).await;
                                    incoming_transfers.write().retain(|t| t.id != transfer_id);
                                });
                            }
                        }
                        TransferEvent::Rejected { transfer_id } => {
                            let mut should_purge = false;
                            {
                                let mut list = incoming_transfers.write();
                                if let Some(t) = list.iter_mut().find(|t| t.id == transfer_id) {
                                    t.status = TransferStatus::Rejected;
                                    should_purge = true;
                                }
                            }
                            if should_purge {
                                spawn(async move {
                                    sleep(DISPLAY_TIMEOUT).await;
                                    incoming_transfers.write().retain(|t| t.id != transfer_id);
                                });
                            }
                        }
                        TransferEvent::Failed { transfer_id, error } => {
                            let mut should_purge = false;
                            {
                                let mut list = incoming_transfers.write();
                                if let Some(t) = list.iter_mut().find(|t| t.id == transfer_id) {
                                    t.status = TransferStatus::Failed(error);
                                    should_purge = true;
                                }
                            }
                            if should_purge {
                                spawn(async move {
                                    sleep(DISPLAY_TIMEOUT).await;
                                    incoming_transfers.write().retain(|t| t.id != transfer_id);
                                });
                            }
                        }
                        TransferEvent::SendProgress {
                            transfer_id,
                            bytes_sent,
                            total,
                        } => {
                            let mut list = outgoing_transfers.write();
                            if let Some(t) = list.iter_mut().find(|t| t.id == transfer_id) {
                                t.status = TransferStatus::InProgress {
                                    bytes_transferred: bytes_sent,
                                    total,
                                };
                            }
                        }
                        TransferEvent::SendCompleted { transfer_id } => {
                            let mut should_purge = false;
                            {
                                let mut list = outgoing_transfers.write();
                                if let Some(t) = list.iter_mut().find(|t| t.id == transfer_id) {
                                    t.status = TransferStatus::Completed;
                                    should_purge = true;
                                }
                            }
                            if should_purge {
                                spawn(async move {
                                    sleep(DISPLAY_TIMEOUT).await;
                                    outgoing_transfers.write().retain(|t| t.id != transfer_id);
                                });
                            }
                        }
                        TransferEvent::SendFailed { transfer_id, error } => {
                            let mut should_purge = false;
                            {
                                let mut list = outgoing_transfers.write();
                                if let Some(t) = list.iter_mut().find(|t| t.id == transfer_id) {
                                    t.status = TransferStatus::Failed(error);
                                    should_purge = true;
                                }
                            }
                            if should_purge {
                                spawn(async move {
                                    sleep(DISPLAY_TIMEOUT).await;
                                    outgoing_transfers.write().retain(|t| t.id != transfer_id);
                                });
                            }
                        }
                    }
                }
            });
        }
    });

    let l = *lang.read();

    rsx! {
        style { {CSS} }
        div { class: "app",
            div { class: "header",
                div { class: "header-top",
                    h1 { {l.app_title()} }
                    LanguageSelector { lang }
                }
                p { class: "status", "{status_msg}" }
            }
            div { class: "tabs",
                button {
                    class: if *active_tab.read() == 0 { "tab active" } else { "tab" },
                    onclick: move |_| active_tab.set(0),
                    {l.tab_devices()}
                }
                button {
                    class: if *active_tab.read() == 1 { "tab active" } else { "tab" },
                    onclick: move |_| active_tab.set(1),
                    {l.tab_incoming()}
                }
                button {
                    class: if *active_tab.read() == 2 { "tab active" } else { "tab" },
                    onclick: move |_| active_tab.set(2),
                    {l.tab_outgoing()}
                }
            }
            div { class: "content",
                match *active_tab.read() {
                    0 => rsx! {
                        DevicesPanel {
                            devices,
                            event_tx: event_tx_holder,
                            device_name,
                            next_send_id,
                            outgoing_transfers,
                            shared_files,
                        }
                    },
                    1 => rsx! {
                        IncomingPanel { transfers: incoming_transfers, cmd_tx, auto_accept_folder }
                    },
                    2 => rsx! {
                        OutgoingPanel { transfers: outgoing_transfers }
                    },
                    _ => rsx! {
                        p { "Unknown tab" }
                    },
                }
            }
        }
    }
}

async fn run_discovery_listener<F, Fut>(
    create_socket: F,
    label: &'static str,
    device_map: DeviceMap,
    own_id: String,
) where
    F: Fn() -> Fut + Send + 'static,
    Fut: Future<Output = ah::Result<UdpSocket>> + Send,
{
    let idle_timeout = Duration::from_secs(10);
    loop {
        match create_socket().await {
            Ok(socket) => loop {
                match timeout(
                    idle_timeout,
                    listen_for_devices(&socket, &own_id, &device_map),
                )
                .await
                {
                    Ok(()) => {}
                    Err(_) => {
                        log::debug!("{label} listener idle for 10s, recreating socket");
                        break;
                    }
                }
            },
            Err(e) => {
                log::warn!("{label} listener unavailable: {e}, retrying in 5s");
                sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

fn render_status(status: &TransferStatus, l: Language) -> Element {
    match status {
        TransferStatus::Pending => rsx! {
            span { class: "status-pending", {l.status_pending()} }
        },
        TransferStatus::InProgress {
            bytes_transferred,
            total,
        } => {
            let pct = if *total > 0 {
                (*bytes_transferred as f64 / *total as f64 * 100.0) as u32
            } else {
                0
            };
            let transferred_str = ByteSize(*bytes_transferred).to_string();
            let total_str = ByteSize(*total).to_string();
            rsx! {
                div { class: "progress-container",
                    div { class: "progress-bar",
                        div { class: "progress-fill", style: "width: {pct}%" }
                    }
                    span { class: "progress-text", "{transferred_str} / {total_str} ({pct}%)" }
                }
            }
        }
        TransferStatus::Completed => rsx! {
            span { class: "status-completed", {l.status_completed()} }
        },
        TransferStatus::Rejected => rsx! {
            span { class: "status-failed", {l.status_rejected()} }
        },
        TransferStatus::Failed(err) => rsx! {
            span { class: "status-failed", {l.status_failed(err)} }
        },
    }
}
